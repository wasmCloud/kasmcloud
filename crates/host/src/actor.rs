use core::pin::Pin;
use core::task::{Context, Poll};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use anyhow::{anyhow, Context as _};
use bytes::{BufMut, Bytes, BytesMut};
use futures::stream::AbortHandle;
use tokio::io::{stderr, AsyncRead, AsyncWrite};
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, trace, warn};
use ulid::Ulid;

use wasmcloud_core::{
    chunking::{ChunkEndpoint, CHUNK_THRESHOLD_BYTES},
    Invocation, InvocationResponse, WasmCloudEntity,
};
use wasmcloud_runtime::{capability::IncomingHttp, ActorInstancePool, Runtime};

use crate::handler::{ensure_actor_capability, Handler};

pub struct ActorGroup {
    pub public_key: String,

    pub actors: RwLock<HashMap<String, Arc<Actor>>>,
    pub links: Arc<RwLock<HashMap<String, HashMap<String, WasmCloudEntity>>>>,
}

pub struct Actor {
    pub image_ref: String,
    pub handler: Handler,
    pub pool: ActorInstancePool,

    pub instances: RwLock<Vec<Arc<ActorInstance>>>,
}

pub struct ActorInstance {
    pub id: Ulid,
    pub runtime: Runtime,
    pub handler: Handler,
    pub pool: ActorInstancePool,
    pub nats: async_nats::Client,
    pub valid_issuers: Vec<String>,
    pub chunk_endpoint: ChunkEndpoint,
    pub calls: AbortHandle,
}

impl ActorInstance {
    async fn handle_invocation(
        &self,
        contract_id: &str,
        operation: &str,
        msg: Vec<u8>,
    ) -> anyhow::Result<Result<Vec<u8>, String>> {
        ensure_actor_capability(self.handler.claims.metadata.as_ref(), contract_id)?;

        let mut instance = self
            .pool
            .instantiate(self.runtime.clone())
            .await
            .context("failed to instantiate actor")?;
        instance
            .stderr(stderr())
            .await
            .context("failed to set stderr")?
            .blobstore(Arc::new(self.handler.clone()))
            .bus(Arc::new(self.handler.clone()))
            .keyvalue_atomic(Arc::new(self.handler.clone()))
            .keyvalue_readwrite(Arc::new(self.handler.clone()))
            .messaging(Arc::new(self.handler.clone()));

        match (contract_id, operation) {
            ("wasmcloud:httpserver", "HttpServer.HandleRequest") => {
                let req: wasmcloud_compat::HttpRequest =
                    rmp_serde::from_slice(&msg).context("failed to decode HTTP request")?;
                let req = http::Request::try_from(req).context("failed to convert request")?;

                let res = match wasmcloud_runtime::ActorInstance::from(instance)
                    .into_incoming_http()
                    .await
                    .context("failed to instantiate `wasi:http/incoming-handler`")?
                    .handle(req.map(|body| -> Box<dyn AsyncRead + Send + Sync + Unpin> {
                        Box::new(Cursor::new(body))
                    }))
                    .await
                {
                    Ok(res) => res,
                    Err(err) => return Ok(Err(format!("{err:#}"))),
                };
                let res = wasmcloud_compat::HttpResponse::from_http(res)
                    .await
                    .context("failed to convert response")?;
                let res = rmp_serde::to_vec_named(&res).context("failed to encode response")?;
                Ok(Ok(res))
            }
            _ => {
                let res = AsyncBytesMut::default();
                match instance
                    .call(operation, Cursor::new(msg), res.clone())
                    .await
                    .context("failed to call actor")?
                {
                    Ok(()) => {
                        let res = res.try_into().context("failed to unwrap bytes")?;
                        Ok(Ok(res))
                    }
                    Err(e) => Ok(Err(e)),
                }
            }
        }
    }

    async fn handle_call(&self, payload: impl AsRef<[u8]>) -> anyhow::Result<Bytes> {
        let invocation: Invocation =
            rmp_serde::from_slice(payload.as_ref()).context("failed to decode invocation")?;

        invocation.validate_antiforgery(&self.valid_issuers)?;

        let content_length: usize = invocation
            .content_length
            .try_into()
            .context("failed to convert content_length to usize")?;

        let inv_msg = if content_length > CHUNK_THRESHOLD_BYTES {
            self.chunk_endpoint.get_unchunkified(&invocation.id).await?
        } else {
            invocation.msg
        };

        let res = match self
            .handle_invocation(
                &invocation.origin.contract_id,
                &invocation.operation,
                inv_msg,
            )
            .await
            .context("failed to handle invocation")?
        {
            Ok(resp_msg) => {
                let content_length = resp_msg.len();
                let resp_msg = if content_length > CHUNK_THRESHOLD_BYTES {
                    debug!(inv_id = invocation.id, "chunking invocation response");
                    self.chunk_endpoint
                        .chunkify_response(&invocation.id, Cursor::new(resp_msg))
                        .await
                        .context("failed to chunk invocation response")?;
                    vec![]
                } else {
                    resp_msg
                };

                InvocationResponse {
                    msg: resp_msg,
                    invocation_id: invocation.id,
                    content_length: content_length as u64,
                    ..Default::default()
                }
            }
            Err(e) => InvocationResponse {
                invocation_id: invocation.id,
                error: Some(e),
                ..Default::default()
            },
        };

        rmp_serde::to_vec_named(&res)
            .map(Into::into)
            .context("failed to encode response")
    }

    pub async fn handle_message(
        &self,
        async_nats::Message {
            reply,
            payload,
            subject,
            ..
        }: async_nats::Message,
    ) {
        let res = self.handle_call(payload).await;
        match (reply, res) {
            (Some(reply), Ok(buf)) => {
                if let Err(e) = self.nats.publish(reply, buf).await {
                    println!("failed to publish response to `{subject}` request: {e:?}");
                }
            }
            (_, Err(e)) => {
                println!("failed to handle `{subject}` request: {e:?}");
            }
            _ => {}
        }
    }
}

#[derive(Clone, Default)]
struct AsyncBytesMut(Arc<std::sync::Mutex<BytesMut>>);

impl AsyncWrite for AsyncBytesMut {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Poll::Ready({
            self.0
                .lock()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?
                .put_slice(buf);
            Ok(buf.len())
        })
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl TryFrom<AsyncBytesMut> for Vec<u8> {
    type Error = anyhow::Error;

    fn try_from(buf: AsyncBytesMut) -> Result<Self, Self::Error> {
        buf.0
            .lock()
            .map(|buf| buf.clone().into())
            .map_err(|e| anyhow!(e.to_string()).context("failed to lock"))
    }
}
