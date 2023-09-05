use core::future::Future;
use core::ops::RangeInclusive;
use core::pin::Pin;
use core::time::Duration;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use anyhow::{anyhow, bail, ensure, Context as _};
use async_nats::Request;
use async_trait::async_trait;
use futures::{stream, FutureExt, Stream};
use nkeys::KeyPair;
use serde::{Deserialize, Serialize};
use tokio::io::{empty, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, trace, warn};

use wascap::jwt;
use wasmcloud_core::chunking::{ChunkEndpoint, CHUNK_RPC_EXTRA_TIME, CHUNK_THRESHOLD_BYTES};
use wasmcloud_core::{Invocation, InvocationResponse, WasmCloudEntity};
use wasmcloud_runtime::capability::{
    blobstore, messaging, ActorIdentifier, Blobstore, Bus, KeyValueAtomic, KeyValueReadWrite,
    Messaging, TargetEntity, TargetInterface,
};
use wasmcloud_tracing::context::{attach_span_context, OtelHeaderInjector};

#[derive(Clone, Debug)]
pub struct Handler {
    pub nats: async_nats::Client,
    pub lattice_prefix: String,
    pub cluster_key: Arc<KeyPair>,
    pub host_key: Arc<KeyPair>,
    pub claims: jwt::Claims<jwt::Actor>,
    pub origin: WasmCloudEntity,
    // package -> target -> entity
    pub links: Arc<RwLock<HashMap<String, HashMap<String, WasmCloudEntity>>>>,
    pub targets: Arc<RwLock<HashMap<TargetInterface, TargetEntity>>>,
    pub chunk_endpoint: ChunkEndpoint,
    pub aliases: Arc<RwLock<HashMap<String, WasmCloudEntity>>>,
}

impl Handler {
    #[instrument(skip(self, operation, request))]
    async fn call_operation_with_payload(
        &self,
        target: Option<&TargetEntity>,
        operation: impl Into<String>,
        request: Vec<u8>,
    ) -> anyhow::Result<Result<Vec<u8>, String>> {
        let links = self.links.read().await;
        let aliases = self.aliases.read().await;
        let operation = operation.into();
        let (package, _) = operation
            .split_once('/')
            .context("failed to parse operation")?;
        let inv_target = resolve_target(target, links.get(package), &aliases).await?;
        let needs_chunking = request.len() > CHUNK_THRESHOLD_BYTES;
        let mut invocation = Invocation::new(
            &self.cluster_key,
            &self.host_key,
            self.origin.clone(),
            inv_target,
            operation,
            request,
            OtelHeaderInjector::default_with_span().into(),
        )?;

        // Validate that the actor has the capability to call the target
        ensure_actor_capability(
            self.claims.metadata.as_ref(),
            &invocation.target.contract_id,
        )?;

        if needs_chunking {
            self.chunk_endpoint
                .chunkify(&invocation.id, Cursor::new(invocation.msg))
                .await
                .context("failed to chunk invocation")?;
            invocation.msg = vec![];
        }

        let payload =
            rmp_serde::to_vec_named(&invocation).context("failed to encode invocation")?;
        let topic = match target {
            None | Some(TargetEntity::Link(_)) => format!(
                "wasmbus.rpc.{}.{}.{}",
                self.lattice_prefix, invocation.target.public_key, invocation.target.link_name,
            ),
            Some(TargetEntity::Actor(_)) => format!(
                "wasmbus.rpc.{}.{}",
                self.lattice_prefix, invocation.target.public_key
            ),
        };

        let timeout = needs_chunking.then_some(CHUNK_RPC_EXTRA_TIME); // TODO: add rpc_nats timeout
        let request = Request::new().payload(payload.into()).timeout(timeout);
        let res = self
            .nats
            .send_request(topic, request)
            .await
            .context("failed to publish on NATS topic")?;

        let InvocationResponse {
            invocation_id,
            mut msg,
            content_length,
            error,
            ..
        } = rmp_serde::from_slice(&res.payload).context("failed to decode invocation response")?;
        ensure!(invocation_id == invocation.id, "invocation ID mismatch");

        let resp_length =
            usize::try_from(content_length).context("content length does not fit in usize")?;
        if resp_length > CHUNK_THRESHOLD_BYTES {
            msg = self
                .chunk_endpoint
                .get_unchunkified(&invocation_id)
                .await
                .context("failed to dechunk response")?;
        } else {
            ensure!(resp_length == msg.len(), "message size mismatch");
        }

        if let Some(error) = error {
            Ok(Err(error))
        } else {
            Ok(Ok(msg))
        }
    }

    #[instrument(skip(self, operation, request))]
    async fn call_operation(
        &self,
        target: Option<&TargetEntity>,
        operation: impl Into<String>,
        request: &impl Serialize,
    ) -> anyhow::Result<Vec<u8>> {
        let request = rmp_serde::to_vec_named(request).context("failed to encode request")?;
        self.call_operation_with_payload(target, operation, request)
            .await
            .context("failed to call target entity")?
            .map_err(|err| anyhow!(err).context("call failed"))
    }
}

#[async_trait]
impl Blobstore for Handler {
    #[instrument]
    async fn create_container(&self, name: &str) -> anyhow::Result<()> {
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasiBlobstoreBlobstore),
            "wasmcloud:blobstore/Blobstore.CreateContainer",
            &name,
        )
        .await
        .and_then(decode_empty_provider_response)
    }

    #[instrument]
    async fn container_exists(&self, name: &str) -> anyhow::Result<bool> {
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasiBlobstoreBlobstore),
            "wasmcloud:blobstore/Blobstore.ContainerExists",
            &name,
        )
        .await
        .and_then(decode_provider_response)
    }

    #[instrument]
    async fn delete_container(&self, name: &str) -> anyhow::Result<()> {
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasiBlobstoreBlobstore),
            "wasmcloud:blobstore/Blobstore.DeleteContainer",
            &name,
        )
        .await
        .and_then(decode_empty_provider_response)
    }

    #[instrument]
    async fn container_info(
        &self,
        name: &str,
    ) -> anyhow::Result<blobstore::container::ContainerMetadata> {
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiBlobstoreBlobstore),
                "wasmcloud:blobstore/Blobstore.GetContainerInfo",
                &name,
            )
            .await?;
        let wasmcloud_compat::blobstore::ContainerMetadata {
            container_id: name,
            created_at,
        } = decode_provider_response(res)?;
        let created_at = created_at
            .map(|wasmcloud_compat::Timestamp { sec, .. }| sec.try_into())
            .transpose()
            .context("timestamp seconds do not fit in `u64`")?;
        Ok(blobstore::container::ContainerMetadata {
            name,
            created_at: created_at.unwrap_or_default(),
        })
    }

    #[instrument]
    async fn get_data(
        &self,
        container: &str,
        name: String,
        range: RangeInclusive<u64>,
    ) -> anyhow::Result<(Box<dyn AsyncRead + Sync + Send + Unpin>, u64)> {
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiBlobstoreBlobstore),
                "wasmcloud:blobstore/Blobstore.GetObject",
                &wasmcloud_compat::blobstore::GetObjectRequest {
                    object_id: name.clone(),
                    container_id: container.into(),
                    range_start: Some(*range.start()),
                    range_end: Some(*range.end()),
                },
            )
            .await?;
        let wasmcloud_compat::blobstore::GetObjectResponse {
            success,
            error,
            initial_chunk,
            ..
        } = decode_provider_response(res)?;
        match (success, initial_chunk, error) {
            (_, _, Some(err)) => Err(anyhow!(err).context("failed to get object response")),
            (false, _, None) => bail!("failed to get object response"),
            (true, None, None) => Ok((Box::new(empty()), 0)),
            (
                true,
                Some(wasmcloud_compat::blobstore::Chunk {
                    object_id,
                    container_id,
                    bytes,
                    ..
                }),
                None,
            ) => {
                ensure!(object_id == name);
                ensure!(container_id == container);
                let size = bytes
                    .len()
                    .try_into()
                    .context("value size does not fit in `u64`")?;
                Ok((Box::new(Cursor::new(bytes)), size))
            }
        }
    }

    #[instrument]
    async fn has_object(&self, container: &str, name: String) -> anyhow::Result<bool> {
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasiBlobstoreBlobstore),
            "wasmcloud:blobstore/Blobstore.ObjectExists",
            &wasmcloud_compat::blobstore::ContainerObject {
                container_id: container.into(),
                object_id: name,
            },
        )
        .await
        .and_then(decode_provider_response)
    }

    #[instrument(skip(value))]
    async fn write_data(
        &self,
        container: &str,
        name: String,
        mut value: Box<dyn AsyncRead + Sync + Send + Unpin>,
    ) -> anyhow::Result<()> {
        let targets = self.targets.read().await;
        let mut bytes = Vec::new();
        value
            .read_to_end(&mut bytes)
            .await
            .context("failed to read bytes")?;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiBlobstoreBlobstore),
                "wasmcloud:blobstore/Blobstore.PutObject",
                &wasmcloud_compat::blobstore::PutObjectRequest {
                    chunk: wasmcloud_compat::blobstore::Chunk {
                        object_id: name,
                        container_id: container.into(),
                        bytes,
                        offset: 0,
                        is_last: true,
                    },
                    ..Default::default()
                },
            )
            .await?;
        let wasmcloud_compat::blobstore::PutObjectResponse { stream_id } =
            decode_provider_response(res)?;
        ensure!(
            stream_id.is_none(),
            "provider returned an unexpected stream ID"
        );
        Ok(())
    }

    #[instrument]
    async fn delete_objects(&self, container: &str, names: Vec<String>) -> anyhow::Result<()> {
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiBlobstoreBlobstore),
                "wasmcloud:blobstore/Blobstore.RemoveObjects",
                &wasmcloud_compat::blobstore::RemoveObjectsRequest {
                    container_id: container.into(),
                    objects: names,
                },
            )
            .await?;
        for wasmcloud_compat::blobstore::ItemResult {
            key,
            success,
            error,
        } in decode_provider_response::<Vec<_>>(res)?
        {
            if let Some(err) = error {
                bail!(err)
            }
            ensure!(success, "failed to delete object `{key}`");
        }
        Ok(())
    }

    #[instrument]
    async fn list_objects(
        &self,
        container: &str,
    ) -> anyhow::Result<Box<dyn Stream<Item = anyhow::Result<String>> + Sync + Send + Unpin>> {
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiBlobstoreBlobstore),
                "wasmcloud:blobstore/Blobstore.ListObjects",
                &wasmcloud_compat::blobstore::ListObjectsRequest {
                    container_id: container.into(),
                    max_items: Some(u32::MAX),
                    ..Default::default()
                },
            )
            .await?;
        let wasmcloud_compat::blobstore::ListObjectsResponse {
            objects,
            is_last,
            continuation,
        } = decode_provider_response(res)?;
        ensure!(is_last);
        ensure!(continuation.is_none(), "chunked responses not supported");
        Ok(Box::new(stream::iter(objects.into_iter().map(
            |wasmcloud_compat::blobstore::ObjectMetadata { object_id, .. }| Ok(object_id),
        ))))
    }

    #[instrument]
    async fn object_info(
        &self,
        container: &str,
        name: String,
    ) -> anyhow::Result<blobstore::container::ObjectMetadata> {
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiBlobstoreBlobstore),
                "wasmcloud:blobstore/Blobstore.GetObjectInfo",
                &name,
            )
            .await?;
        let wasmcloud_compat::blobstore::ObjectMetadata {
            object_id,
            container_id,
            content_length,
            ..
        } = decode_provider_response(res)?;
        Ok(blobstore::container::ObjectMetadata {
            name: object_id,
            container: container_id,
            size: content_length,
            created_at: 0,
        })
    }
}

#[async_trait]
impl Bus for Handler {
    #[instrument(skip(self))]
    async fn identify_wasmbus_target(
        &self,
        binding: &str,
        namespace: &str,
    ) -> anyhow::Result<TargetEntity> {
        let links = self.links.read().await;
        if links
            .get(namespace)
            .map(|bindings| bindings.contains_key(binding))
            .unwrap_or_default()
        {
            return Ok(TargetEntity::Link(Some(binding.into())));
        }
        Ok(TargetEntity::Actor(namespace.into()))
    }

    #[instrument(skip(self))]
    async fn set_target(
        &self,
        target: Option<TargetEntity>,
        interfaces: Vec<TargetInterface>,
    ) -> anyhow::Result<()> {
        let mut targets = self.targets.write().await;
        if let Some(target) = target {
            for interface in interfaces {
                targets.insert(interface, target.clone());
            }
        } else {
            for interface in interfaces {
                targets.remove(&interface);
            }
        }
        Ok(())
    }

    #[instrument(skip(self))]
    async fn call(
        &self,
        target: Option<TargetEntity>,
        operation: String,
    ) -> anyhow::Result<(
        Pin<Box<dyn Future<Output = Result<(), String>> + Send>>,
        Box<dyn AsyncWrite + Sync + Send + Unpin>,
        Box<dyn AsyncRead + Sync + Send + Unpin>,
    )> {
        let (mut req_r, req_w) = socket_pair()?;
        let (res_r, mut res_w) = socket_pair()?;

        let links = Arc::clone(&self.links);
        let aliases = Arc::clone(&self.aliases);
        let nats = self.nats.clone();
        let chunk_endpoint = self.chunk_endpoint.clone();
        let lattice_prefix = self.lattice_prefix.clone();
        let origin = self.origin.clone();
        let cluster_key = self.cluster_key.clone();
        let host_key = self.host_key.clone();
        let claims_metadata = self.claims.metadata.clone();
        Ok((
            async move {
                // TODO: Stream data
                let mut request = vec![];
                req_r
                    .read_to_end(&mut request)
                    .await
                    .context("failed to read request")
                    .map_err(|e| e.to_string())?;
                let links = links.read().await;
                let aliases = aliases.read().await;
                let (package, _) = operation
                    .split_once('/')
                    .context("failed to parse operation")
                    .map_err(|e| e.to_string())?;
                let inv_target = resolve_target(target.as_ref(), links.get(package), &aliases)
                    .await
                    .map_err(|e| e.to_string())?;
                let needs_chunking = request.len() > CHUNK_THRESHOLD_BYTES;
                let mut invocation = Invocation::new(
                    &cluster_key,
                    &host_key,
                    origin,
                    inv_target,
                    operation,
                    request,
                    OtelHeaderInjector::default_with_span().into(),
                )
                .map_err(|e| e.to_string())?;

                // Validate that the actor has the capability to call the target
                ensure_actor_capability(claims_metadata.as_ref(), &invocation.target.contract_id)
                    .map_err(|e| e.to_string())?;

                if needs_chunking {
                    chunk_endpoint
                        .chunkify(&invocation.id, Cursor::new(invocation.msg))
                        .await
                        .context("failed to chunk invocation")
                        .map_err(|e| e.to_string())?;
                    invocation.msg = vec![];
                }

                let payload = rmp_serde::to_vec_named(&invocation)
                    .context("failed to encode invocation")
                    .map_err(|e| e.to_string())?;
                let topic = match target {
                    None | Some(TargetEntity::Link(_)) => format!(
                        "wasmbus.rpc.{lattice_prefix}.{}.{}",
                        invocation.target.public_key, invocation.target.link_name,
                    ),
                    Some(TargetEntity::Actor(_)) => format!(
                        "wasmbus.rpc.{lattice_prefix}.{}",
                        invocation.target.public_key
                    ),
                };

                let timeout = needs_chunking.then_some(CHUNK_RPC_EXTRA_TIME); // TODO: add rpc_nats timeout
                let request = Request::new().payload(payload.into()).timeout(timeout);
                let res = nats
                    .send_request(topic, request)
                    .await
                    .context("failed to call provider")
                    .map_err(|e| e.to_string())?;

                let InvocationResponse {
                    invocation_id,
                    mut msg,
                    content_length,
                    error,
                    ..
                } = rmp_serde::from_slice(&res.payload)
                    .context("failed to decode invocation response")
                    .map_err(|e| e.to_string())?;
                if invocation_id != invocation.id {
                    return Err("invocation ID mismatch".into());
                }

                let resp_length = usize::try_from(content_length)
                    .context("content length does not fit in usize")
                    .map_err(|e| e.to_string())?;
                if resp_length > CHUNK_THRESHOLD_BYTES {
                    msg = chunk_endpoint
                        .get_unchunkified(&invocation_id)
                        .await
                        .context("failed to dechunk response")
                        .map_err(|e| e.to_string())?;
                } else if resp_length != msg.len() {
                    return Err("message size mismatch".into());
                }

                if let Some(error) = error {
                    Err(error)
                } else {
                    res_w
                        .write_all(&msg)
                        .await
                        .context("failed to write reply")
                        .map_err(|e| e.to_string())?;
                    Ok(())
                }
            }
            .boxed(),
            Box::new(req_w),
            Box::new(res_r),
        ))
    }

    #[instrument(skip(self, request))]
    async fn call_sync(
        &self,
        target: Option<TargetEntity>,
        operation: String,
        request: Vec<u8>,
    ) -> anyhow::Result<Vec<u8>> {
        self.call_operation_with_payload(target.as_ref(), operation, request)
            .await
            .context("failed to call linked provider")?
            .map_err(|e| anyhow!(e).context("provider call failed"))
    }
}

#[async_trait]
impl KeyValueAtomic for Handler {
    #[instrument(skip(self))]
    async fn increment(&self, bucket: &str, key: String, delta: u64) -> anyhow::Result<u64> {
        const METHOD: &str = "wasmcloud:keyvalue/KeyValue.Increment";
        if !bucket.is_empty() {
            bail!("buckets not currently supported")
        }
        let targets = self.targets.read().await;
        let value = delta.try_into().context("delta does not fit in `i32`")?;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiKeyvalueAtomic),
                METHOD,
                &wasmcloud_compat::keyvalue::IncrementRequest { key, value },
            )
            .await?;
        let new: i32 = decode_provider_response(res)?;
        let new = new.try_into().context("result does not fit in `u64`")?;
        Ok(new)
    }

    #[allow(unused)] // TODO: Implement https://github.com/wasmCloud/wasmCloud/issues/457
    #[instrument(skip(self))]
    async fn compare_and_swap(
        &self,
        bucket: &str,
        key: String,
        old: u64,
        new: u64,
    ) -> anyhow::Result<bool> {
        bail!("not supported")
    }
}

#[async_trait]
impl KeyValueReadWrite for Handler {
    #[instrument(skip(self))]
    async fn get(
        &self,
        bucket: &str,
        key: String,
    ) -> anyhow::Result<(Box<dyn AsyncRead + Sync + Send + Unpin>, u64)> {
        const METHOD: &str = "wasmcloud:keyvalue/KeyValue.Get";
        if !bucket.is_empty() {
            bail!("buckets not currently supported")
        }
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiKeyvalueReadwrite),
                METHOD,
                &key,
            )
            .await?;
        let wasmcloud_compat::keyvalue::GetResponse { value, exists } =
            decode_provider_response(res)?;
        if !exists {
            bail!("key not found")
        }
        let size = value
            .len()
            .try_into()
            .context("value size does not fit in `u64`")?;
        Ok((Box::new(Cursor::new(value)), size))
    }

    #[instrument(skip(self, value))]
    async fn set(
        &self,
        bucket: &str,
        key: String,
        mut value: Box<dyn AsyncRead + Sync + Send + Unpin>,
    ) -> anyhow::Result<()> {
        const METHOD: &str = "wasmcloud:keyvalue/KeyValue.Set";
        if !bucket.is_empty() {
            bail!("buckets not currently supported")
        }
        let mut buf = String::new();
        value
            .read_to_string(&mut buf)
            .await
            .context("failed to read value")?;
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasiKeyvalueReadwrite),
            METHOD,
            &wasmcloud_compat::keyvalue::SetRequest {
                key,
                value: buf,
                expires: 0,
            },
        )
        .await
        .and_then(decode_empty_provider_response)
    }

    #[instrument(skip(self))]
    async fn delete(&self, bucket: &str, key: String) -> anyhow::Result<()> {
        const METHOD: &str = "wasmcloud:keyvalue/KeyValue.Del";
        if !bucket.is_empty() {
            bail!("buckets not currently supported")
        }
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasiKeyvalueReadwrite),
                METHOD,
                &key,
            )
            .await?;
        let deleted: bool = decode_provider_response(res)?;
        ensure!(deleted, "key not found");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn exists(&self, bucket: &str, key: String) -> anyhow::Result<bool> {
        const METHOD: &str = "wasmcloud:keyvalue/KeyValue.Contains";
        if !bucket.is_empty() {
            bail!("buckets not currently supported")
        }
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasiKeyvalueReadwrite),
            METHOD,
            &key,
        )
        .await
        .and_then(decode_provider_response)
    }
}

#[async_trait]
impl Messaging for Handler {
    #[instrument(skip(self, body))]
    async fn request(
        &self,
        subject: String,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> anyhow::Result<messaging::types::BrokerMessage> {
        const METHOD: &str = "wasmcloud:messaging/Messaging.Request";

        let timeout_ms = timeout
            .as_millis()
            .try_into()
            .context("timeout milliseconds do not fit in `u32`")?;
        let targets = self.targets.read().await;
        let res = self
            .call_operation(
                targets.get(&TargetInterface::WasmcloudMessagingConsumer),
                METHOD,
                &wasmcloud_compat::messaging::RequestMessage {
                    subject,
                    body: body.unwrap_or_default(),
                    timeout_ms,
                },
            )
            .await?;
        let wasmcloud_compat::messaging::ReplyMessage {
            subject,
            reply_to,
            body,
        } = decode_provider_response(res)?;
        Ok(messaging::types::BrokerMessage {
            subject,
            reply_to,
            body: Some(body),
        })
    }

    #[instrument(skip(self, body))]
    async fn request_multi(
        &self,
        subject: String,
        body: Option<Vec<u8>>,
        timeout: Duration,
        max_results: u32,
    ) -> anyhow::Result<Vec<messaging::types::BrokerMessage>> {
        match max_results {
            0..=1 => {
                let res = self.request(subject, body, timeout).await?;
                Ok(vec![res])
            }
            2.. => bail!("at most 1 result can be requested at the time"),
        }
    }

    #[instrument(skip_all)]
    async fn publish(
        &self,
        messaging::types::BrokerMessage {
            subject,
            reply_to,
            body,
        }: messaging::types::BrokerMessage,
    ) -> anyhow::Result<()> {
        const METHOD: &str = "wasmcloud:messaging/Messaging.Publish";
        let targets = self.targets.read().await;
        self.call_operation(
            targets.get(&TargetInterface::WasmcloudMessagingConsumer),
            METHOD,
            &wasmcloud_compat::messaging::PubMessage {
                subject,
                reply_to,
                body: body.unwrap_or_default(),
            },
        )
        .await
        .and_then(decode_empty_provider_response)
    }
}

pub fn ensure_actor_capability(
    claims_metadata: Option<&jwt::Actor>,
    contract_id: impl AsRef<str>,
) -> anyhow::Result<()> {
    let contract_id: &str = contract_id.as_ref();
    match claims_metadata {
        // [ADR-0006](https://github.com/wasmCloud/wasmCloud/blob/main/adr/0006-actor-to-actor.md)
        // Allow actor to actor calls by default
        _ if contract_id.is_empty() => {}
        Some(jwt::Actor {
            caps: Some(ref caps),
            ..
        }) => {
            ensure!(
                caps.iter().any(|cap| cap == contract_id),
                "actor does not have capability claim `{contract_id}`"
            );
        }
        Some(_) | None => bail!("actor missing capability claims, denying invocation"),
    }
    Ok(())
}

#[cfg(unix)]
fn socket_pair() -> anyhow::Result<(tokio::net::UnixStream, tokio::net::UnixStream)> {
    tokio::net::UnixStream::pair().context("failed to create an unnamed unix socket pair")
}

#[cfg(windows)]
fn socket_pair() -> anyhow::Result<(tokio::io::DuplexStream, tokio::io::DuplexStream)> {
    Ok(tokio::io::duplex(8196))
}

/// Decode provider response accounting for the custom wasmbus-rpc encoding format
fn decode_provider_response<T>(buf: impl AsRef<[u8]>) -> anyhow::Result<T>
where
    for<'a> T: Deserialize<'a>,
{
    let buf = buf.as_ref();
    match buf.split_first() {
        Some((0x7f, _)) => bail!("CBOR responses are not supported"),
        Some((0xc1, buf)) => rmp_serde::from_slice(buf),
        _ => rmp_serde::from_slice(buf),
    }
    .context("failed to decode response")
}

fn decode_empty_provider_response(buf: impl AsRef<[u8]>) -> anyhow::Result<()> {
    let buf = buf.as_ref();
    if buf.is_empty() {
        Ok(())
    } else {
        decode_provider_response(buf)
    }
}

#[instrument]
async fn resolve_target(
    target: Option<&TargetEntity>,
    links: Option<&HashMap<String, WasmCloudEntity>>,
    aliases: &HashMap<String, WasmCloudEntity>,
) -> anyhow::Result<WasmCloudEntity> {
    const DEFAULT_LINK_NAME: &str = "default";

    trace!("resolve target");

    let target = match target {
        None => links
            .and_then(|targets| targets.get(DEFAULT_LINK_NAME))
            .context("link not found")?
            .clone(),
        Some(TargetEntity::Link(link_name)) => links
            .and_then(|targets| targets.get(link_name.as_deref().unwrap_or(DEFAULT_LINK_NAME)))
            .context("link not found")?
            .clone(),
        Some(TargetEntity::Actor(ActorIdentifier::Key(key))) => WasmCloudEntity {
            public_key: key.public_key(),
            ..Default::default()
        },
        Some(TargetEntity::Actor(ActorIdentifier::Alias(alias))) => aliases
            .get(alias)
            .context("unknown actor call alias")?
            .clone(),
    };
    Ok(target)
}
