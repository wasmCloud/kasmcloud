mod actor;
mod handler;
mod provider;

use core::num::NonZeroUsize;
use core::time::Duration;
use std::collections::{hash_map, HashMap};
use std::env;
use std::env::consts::{ARCH, FAMILY, OS};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, bail, ensure, Context as _};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures::stream::{AbortHandle, Abortable};
use futures::{stream, try_join, StreamExt, TryStreamExt};
use nkeys::{KeyPair, KeyPairType};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tokio::{process, spawn};
use tracing::{error, info, instrument};
use ulid::Ulid;
use url::Url;
use uuid::Uuid;

use wascap::jwt;
use wasmcloud_control_interface::LinkDefinition;
use wasmcloud_core::{chunking::ChunkEndpoint, HostData, OtelConfig, WasmCloudEntity};
use wasmcloud_host::{policy::RequestTarget, RegistryConfig};
use wasmcloud_runtime::{ActorInstancePool, Runtime};

use handler::Handler;

#[derive(Clone)]
pub struct NatsConfig {
    pub url: Url,
    /// Timeout period for all RPC calls
    pub timeout: Option<Duration>,
    /// Authentication JWT for RPC connection, must be specified with rpc_seed
    pub jwt: Option<String>,
    /// Authentication key pair for RPC connection, must be specified with rpc_jwt
    pub key: Option<Arc<KeyPair>>,
    /// Whether to require TLS for RPC connection
    pub tls: bool,
}

async fn connect_nats(config: NatsConfig) -> anyhow::Result<async_nats::Client> {
    let opts = async_nats::ConnectOptions::new().require_tls(config.tls);
    let opts = match (config.jwt, config.key) {
        (Some(jwt), Some(key)) => opts.jwt(jwt.to_string(), {
            move |nonce| {
                let key = key.clone();
                async move { key.sign(&nonce).map_err(async_nats::AuthError::new) }
            }
        }),
        (Some(_), None) | (None, Some(_)) => {
            bail!("cannot authenticate if only one of jwt or seed is specified")
        }
        _ => opts,
    };
    let opts = if let Some(timeout) = config.timeout {
        opts.request_timeout(Some(timeout))
    } else {
        opts
    };
    opts.connect(config.url.as_str())
        .await
        .context("failed to connect to NATS")
}

#[derive(Clone)]
pub struct HostConfig {
    pub allow_file_load: bool,

    pub rpc_config: NatsConfig,
    pub prov_rpc_config: NatsConfig,

    /// The lattice the host belongs to
    pub lattice_prefix: String,
    /// The domain to use for host Jetstream operations
    pub js_domain: Option<String>,

    /// The server key pair used by this host to generate its public key
    pub host_key: Option<Arc<KeyPair>>,
    /// The cluster key pair used by this host to sign all invocations
    pub cluster_key: Option<Arc<KeyPair>>,
    /// The identity keys (a printable 256-bit Ed25519 public key) that this host should allow invocations from
    pub cluster_issuers: Option<Vec<String>>,

    pub log_level: Option<wasmcloud_core::logging::Level>,
}

pub struct HostStats {
    pub provider_count: u64,
    pub actor_count: u64,
    pub actor_instance_count: u64,
}

pub struct Host {
    pub name: String,
    pub host_key: Arc<KeyPair>,
    pub lattice_prefix: String,
    pub cluster_key: Arc<KeyPair>,
    pub cluster_issuers: Vec<String>,

    config: HostConfig,
    runtime: Runtime,
    rpc_nats: async_nats::Client,
    prov_rpc_nats: async_nats::Client,
    chunk_endpoint: ChunkEndpoint,

    // name -> LinkDefinition
    links: RwLock<HashMap<String, LinkDefinition>>,

    // name -> public key
    actor_keys: RwLock<HashMap<String, String>>,

    // public key -> ActorGroup
    actors: RwLock<HashMap<String, Arc<actor::ActorGroup>>>,

    // image_ref -> Provider
    providers: RwLock<HashMap<String, provider::Provider>>,

    aliases: Arc<RwLock<HashMap<String, WasmCloudEntity>>>,
    registry_settings: RwLock<HashMap<String, RegistryConfig>>,
}

impl Host {
    pub async fn new(name: String, config: HostConfig) -> anyhow::Result<Self> {
        let cluster_key = if let Some(cluster_key) = &config.cluster_key {
            ensure!(cluster_key.key_pair_type() == KeyPairType::Cluster);
            Arc::clone(cluster_key)
        } else {
            Arc::new(KeyPair::new(KeyPairType::Cluster))
        };

        let mut cluster_issuers = config.cluster_issuers.clone().unwrap_or_default();
        if !cluster_issuers.contains(&cluster_key.public_key()) {
            cluster_issuers.push(cluster_key.public_key());
        }

        let host_key = if let Some(host_key) = &config.host_key {
            ensure!(host_key.key_pair_type() == KeyPairType::Server);
            Arc::clone(host_key)
        } else {
            Arc::new(KeyPair::new(KeyPairType::Server))
        };

        println!("cluster key public: {:?}", cluster_key.public_key());
        println!("cluster seed: {:?}", cluster_key.seed());
        println!("host key public: {:?}", host_key.public_key());
        println!("host seed: {:?}", host_key.seed());

        let runtime = Runtime::builder()
            .actor_config(wasmcloud_runtime::ActorConfig {
                require_signature: true,
            })
            .build()
            .context("failed to build runtime")?;

        let mut labels = HashMap::from([
            ("hostcore.arch".into(), ARCH.into()),
            ("hostcore.os".into(), OS.into()),
            ("hostcore.osfamily".into(), FAMILY.into()),
        ]);
        labels.extend(env::vars().filter_map(|(k, v)| {
            let k = k.strip_prefix("HOST_")?;
            Some((k.to_lowercase(), v))
        }));

        let (rpc_nats, prov_rpc_nats) = try_join!(
            async {
                let rpc_nats_url = config.rpc_config.url.as_str();
                info!("connecting to NATS RPC server: {rpc_nats_url}");
                connect_nats(config.rpc_config.clone())
                    .await
                    .context("failed to establish NATS RPC server connection")
            },
            async {
                let prov_rpc_nats_url = config.prov_rpc_config.url.as_str();
                info!("connecting to NATS Provider RPC server: {prov_rpc_nats_url}");
                connect_nats(config.prov_rpc_config.clone())
                    .await
                    .context("failed to establish NATS provider RPC server connection")
            }
        )?;

        let chunk_endpoint = ChunkEndpoint::with_client(
            &config.lattice_prefix,
            rpc_nats.clone(),
            config.js_domain.as_ref(),
        );

        Ok(Host {
            name,
            host_key,
            cluster_key,
            cluster_issuers,
            lattice_prefix: config.lattice_prefix.clone(),

            runtime,
            rpc_nats,
            prov_rpc_nats,
            chunk_endpoint,
            config,

            actors: RwLock::default(),
            actor_keys: RwLock::default(),
            providers: RwLock::default(),
            links: RwLock::default(),
            aliases: Arc::default(),
            registry_settings: RwLock::new(HashMap::new()),
        })
    }

    pub async fn stats(&self) -> HostStats {
        let actors = self.actors.read().await;
        let actor_count = actors.len();
        let mut instance_count: usize = 0;
        for (_, group) in &*actors {
            for (_, actor) in &*group.actors.read().await {
                instance_count += actor.instances.read().await.len();
            }
        }

        HostStats {
            actor_count: actor_count as u64,
            actor_instance_count: instance_count as u64,
            provider_count: self.providers.read().await.len() as u64,
        }
    }

    #[instrument(skip_all)]
    async fn fetch_actor(&self, actor_ref: &str) -> anyhow::Result<wasmcloud_runtime::Actor> {
        let setting = self.registry_settings.read().await;
        let actor = wasmcloud_host::fetch_actor(actor_ref, self.config.allow_file_load, &setting)
            .await
            .context("failed to fetch actor")?;

        let actor = wasmcloud_runtime::Actor::new(&self.runtime, actor)
            .context("failed to initialize actor")?;
        Ok(actor)
    }

    #[instrument(skip(self))]
    pub async fn reconcile_actor(
        &self,
        name: String,
        actor_ref: String,
        replica: usize,
    ) -> anyhow::Result<jwt::Claims<jwt::Actor>> {
        let actor = self.fetch_actor(&actor_ref).await?;
        let claims = actor.claims().context("claims missing")?.clone();

        if let Some(public_key) = self.actor_keys.read().await.get(&name) {
            if public_key != &claims.subject {
                return Err(anyhow!("actor public key is change"));
            }
        }

        if let hash_map::Entry::Vacant(entry) = self.actor_keys.write().await.entry(name.clone()) {
            entry.insert(claims.subject.clone());
        }

        let group = match self.actors.write().await.entry(claims.subject.clone()) {
            hash_map::Entry::Vacant(entry) => {
                let links = self.get_actor_links(claims.subject.clone()).await;
                let group = Arc::new(actor::ActorGroup {
                    public_key: claims.subject.clone(),
                    actors: RwLock::default(),
                    links: Arc::new(RwLock::new(links)),
                });
                entry.insert(group.clone());
                group.clone()
            }
            hash_map::Entry::Occupied(entry) => Arc::clone(entry.get()),
        };
        match group.actors.write().await.entry(name.clone()) {
            hash_map::Entry::Vacant(entry) => {
                if let Some(replica) = NonZeroUsize::new(replica) {
                    let actor = self
                        .start_actor(actor, actor_ref, replica, group.links.clone())
                        .await
                        .context("failed to start actor")?;
                    entry.insert(actor);
                }
            }
            hash_map::Entry::Occupied(entry) => {
                let actor = entry.get();

                let mut instances = actor.instances.write().await;
                let current = instances.len();
                if let Some(delta) = replica.checked_sub(current) {
                    if let Some(delta) = NonZeroUsize::new(delta) {
                        let mut delta = self
                            .instantiate_actor(
                                &claims,
                                &actor.image_ref,
                                delta,
                                actor.pool.clone(),
                                actor.handler.clone(),
                            )
                            .await
                            .context("failed to instantiate actor")?;
                        instances.append(&mut delta);
                    }
                } else if let Some(delta) = current.checked_sub(replica) {
                    info!("stop {delta} actor instances");
                    stream::iter(instances.drain(..delta))
                        .map(Ok)
                        .try_for_each_concurrent(None, |instance| {
                            instance.calls.abort();
                            async { Result::<(), anyhow::Error>::Ok(()) }
                        })
                        .await
                        .context("failed to uninstantiate actor")?;
                };
            }
        }
        Ok(claims)
    }

    #[instrument(skip(self))]
    pub async fn remove_actor(&self, name: String) -> anyhow::Result<()> {
        let public_key = if let Some(key) = self.actor_keys.read().await.get(&name) {
            key.clone()
        } else {
            info!("remove actor success");
            return Ok(());
        };

        if let hash_map::Entry::Occupied(entry) =
            self.actors.write().await.entry(public_key.clone())
        {
            let group = entry.get();
            if let hash_map::Entry::Occupied(entry) = group.actors.write().await.entry(name.clone())
            {
                let actor = entry.get();
                let mut instances = actor.instances.write().await;
                let count = instances.len();
                stream::iter(instances.drain(..count))
                    .map(Ok)
                    .try_for_each_concurrent(None, |instance| {
                        instance.calls.abort();
                        async { Result::<(), anyhow::Error>::Ok(()) }
                    })
                    .await
                    .context("failed to uninstantiate actor")?;

                drop(instances);
                entry.remove();
            }

            if group.actors.read().await.is_empty() {
                entry.remove();
                info!("all actors({public_key}) is removed");
            }
        };

        self.actor_keys.write().await.remove(&name);
        info!("remove actor success");
        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn start_actor(
        &self,
        actor: wasmcloud_runtime::Actor,
        actor_ref: String,
        replica: NonZeroUsize,
        links: Arc<RwLock<HashMap<String, HashMap<String, WasmCloudEntity>>>>,
    ) -> anyhow::Result<Arc<actor::Actor>> {
        let claims = actor.claims().context("claims missing")?;
        let origin = WasmCloudEntity {
            public_key: claims.subject.clone(),
            ..Default::default()
        };

        let handler = Handler {
            lattice_prefix: self.lattice_prefix.to_owned(),
            host_key: Arc::clone(&self.host_key),
            cluster_key: Arc::clone(&self.cluster_key),

            origin,
            claims: claims.clone(),
            nats: self.rpc_nats.clone(),
            chunk_endpoint: self.chunk_endpoint.clone(),

            links,
            targets: Arc::new(RwLock::default()),
            aliases: Arc::clone(&self.aliases),
        };

        let pool = ActorInstancePool::new(actor.clone(), Some(replica.clone()));
        let instances = self
            .instantiate_actor(claims, &actor_ref, replica, pool.clone(), handler.clone())
            .await
            .context("failed to instantiate actor")?;

        Ok(Arc::new(actor::Actor {
            image_ref: actor_ref,
            pool,
            instances: RwLock::new(instances),
            handler,
        }))
    }

    #[instrument(skip_all)]
    pub async fn instantiate_actor(
        &self,
        claims: &jwt::Claims<jwt::Actor>,
        actor_ref: impl AsRef<str>,
        count: NonZeroUsize,
        pool: ActorInstancePool,
        handler: Handler,
    ) -> anyhow::Result<Vec<Arc<actor::ActorInstance>>> {
        let actor_ref = actor_ref.as_ref();
        info!(
            subject = claims.subject,
            actor_ref = actor_ref,
            "instantiate {count} actor instances",
        );
        let instances = futures::stream::repeat(format!(
            "wasmbus.rpc.{lattice_prefix}.{subject}",
            lattice_prefix = self.lattice_prefix,
            subject = claims.subject
        ))
        .take(count.into())
        .then(|topic| {
            let pool = pool.clone();
            let handler = handler.clone();
            async move {
                let calls = self
                    .rpc_nats
                    .queue_subscribe(topic.clone(), topic)
                    .await
                    .context("failed to subscribe to actor call queue")?;

                let (calls_abort, calls_abort_reg) = AbortHandle::new_pair();
                let id = Ulid::new();

                let instance = Arc::new(actor::ActorInstance {
                    nats: self.rpc_nats.clone(),
                    pool,
                    id,
                    calls: calls_abort,
                    runtime: self.runtime.clone(),
                    handler: handler.clone(),
                    chunk_endpoint: self.chunk_endpoint.clone(),
                    valid_issuers: self.cluster_issuers.clone(),
                });

                let _calls = spawn({
                    let instance = Arc::clone(&instance);
                    Abortable::new(calls, calls_abort_reg).for_each_concurrent(None, move |msg| {
                        let instance = Arc::clone(&instance);
                        async move { instance.handle_message(msg).await }
                    })
                });
                anyhow::Result::<_>::Ok(instance)
            }
        })
        .try_collect()
        .await
        .context("failed to instantiate actor")?;

        Ok(instances)
    }

    #[instrument(skip(self))]
    pub async fn start_provider(
        &self,
        link_name: String,
        provider_ref: String,
    ) -> anyhow::Result<jwt::Claims<jwt::CapabilityProvider>> {
        let registry_setting = self.registry_settings.read().await;
        info!("wait fetch provider image");
        let (path, claims) = wasmcloud_host::fetch_provider(
            &provider_ref,
            &link_name,
            self.config.allow_file_load,
            &registry_setting,
        )
        .await
        .context("failed to fetch provider")?;

        let mut target = RequestTarget::from(claims.clone());
        target.link_name = Some(link_name.clone());

        let mut providers = self.providers.write().await;
        let provider::Provider { instances, .. } = providers
            .entry(claims.subject.clone())
            .or_insert(provider::Provider {
                claims: claims.clone(),
                image_ref: provider_ref.into(),
                instances: HashMap::default(),
            });

        if let hash_map::Entry::Vacant(entry) = instances.entry(link_name.clone()) {
            let id = Ulid::new();
            let invocation_seed = self
                .cluster_key
                .seed()
                .context("cluster key seed missing")?;

            let links = self.links.read().await;
            let link_definitions: Vec<_> = links
                .clone()
                .into_values()
                .filter(|ld| ld.provider_id == claims.subject && ld.link_name == link_name)
                .map(|ld| wasmcloud_core::LinkDefinition {
                    actor_id: ld.actor_id,
                    provider_id: ld.provider_id,
                    link_name: ld.link_name,
                    contract_id: ld.contract_id,
                    values: ld.values.into_iter().collect(),
                })
                .collect();

            let lattice_rpc_user_seed = self
                .config
                .prov_rpc_config
                .key
                .as_ref()
                .map(|key| key.seed())
                .transpose()
                .context("private key missing for provider RPC key")?;
            let default_rpc_timeout_ms = Some(
                self.config
                    .rpc_config
                    .timeout
                    .unwrap_or_default()
                    .as_millis()
                    .try_into()
                    .context("failed to convert rpc_timeout to u64")?,
            );

            let host_data = HostData {
                host_id: self.host_key.public_key(),
                lattice_rpc_prefix: self.config.lattice_prefix.clone(),
                link_name: link_name.to_string(),
                lattice_rpc_user_jwt: self.config.prov_rpc_config.jwt.clone().unwrap_or_default(),
                lattice_rpc_user_seed: lattice_rpc_user_seed.unwrap_or_default(),
                lattice_rpc_url: self.config.prov_rpc_config.url.to_string(),
                env_values: vec![],
                instance_id: Uuid::from_u128(id.into()).to_string(),
                config_json: None,
                provider_key: claims.subject.clone(),
                link_definitions,
                default_rpc_timeout_ms,
                cluster_issuers: self.cluster_issuers.clone(),
                invocation_seed,
                log_level: self.config.log_level.clone(),
                structured_logging: false,
                otel_config: OtelConfig {
                    traces_exporter: None,
                    exporter_otlp_endpoint: None,
                },
            };
            let host_data =
                serde_json::to_vec(&host_data).context("failed to serialize provider data")?;

            let mut child = process::Command::new(&path)
                .env_clear()
                .stdin(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .context("failed to spawn provider process")?;
            let mut stdin = child.stdin.take().context("failed to take stdin")?;
            stdin
                .write_all(STANDARD.encode(&host_data).as_bytes())
                .await
                .context("failed to write provider data")?;
            stdin
                .write_all(b"\r\n")
                .await
                .context("failed to write newline")?;
            stdin.shutdown().await.context("failed to close stdin")?;

            entry.insert(provider::ProviderInstance { id, child });
            info!("run provider, instance_id: {id}");
        } else {
            info!("provider is running");
        }
        Ok(claims.clone())
    }

    #[instrument(skip(self))]
    pub async fn stop_provider(&self, link_name: String, key: String) -> anyhow::Result<()> {
        let mut providers = self.providers.write().await;
        let hash_map::Entry::Occupied(mut entry) = providers.entry( key.clone()) else {
            info!("provider is stopped");
            return Ok(());
        };

        let provider = entry.get_mut();
        let instances = &mut provider.instances;
        if let hash_map::Entry::Occupied(entry) = instances.entry(link_name.clone()) {
            entry.remove();

            info!("send gracefully shut down requsest");
            if let Ok(payload) =
                serde_json::to_vec(&json!({ "host_id": self.host_key.public_key()}))
            {
                if let Err(e) = self
                    .prov_rpc_nats
                    .send_request(
                        format!(
                            "wasmbus.rpc.{}.{key}.{link_name}.shutdown",
                            self.config.lattice_prefix
                        ),
                        async_nats::Request::new()
                            .payload(payload.into())
                            .timeout(Some(Duration::from_secs(5))),
                    )
                    .await
                {
                    error!(
                        "Provider didn't gracefully shut down in time, shuting down forcefully: {e}"
                    );
                }
            }
        }
        if instances.is_empty() {
            info!("don't have provider({key})'s instances");
            entry.remove();
        }

        Ok(())
    }

    pub async fn get_actor_links(
        &self,
        key: String,
    ) -> HashMap<String, HashMap<String, WasmCloudEntity>> {
        self.links
            .read()
            .await
            .values()
            .filter(|ld| ld.actor_id == key)
            .fold(
                HashMap::<_, HashMap<_, _>>::default(),
                |mut links,
                 LinkDefinition {
                     link_name,
                     contract_id,
                     provider_id,
                     ..
                 }| {
                    links.entry(contract_id.clone()).or_default().insert(
                        link_name.clone(),
                        WasmCloudEntity {
                            link_name: link_name.clone(),
                            contract_id: contract_id.clone(),
                            public_key: provider_id.clone(),
                        },
                    );
                    links
                },
            )
    }

    #[instrument(skip(self))]
    pub async fn add_linkdef(&self, ld: LinkDefinition) -> anyhow::Result<()> {
        let id = linkdef_hash(&ld.actor_id, &ld.contract_id, &ld.link_name);
        if let hash_map::Entry::Vacant(entry) = self.links.write().await.entry(id.clone()) {
            entry.insert(ld.clone());
        } else {
            info!("skip linkdef");
            return Ok(());
        }

        info!("add linkdef");
        if let Some(actor) = self.actors.read().await.get(&ld.actor_id) {
            let mut links = actor.links.write().await;

            links.entry(ld.contract_id.clone()).or_default().insert(
                ld.link_name.clone(),
                WasmCloudEntity {
                    public_key: ld.provider_id.clone(),
                    link_name: ld.link_name.clone(),
                    contract_id: ld.contract_id.clone(),
                },
            );
        }

        let msgp = rmp_serde::to_vec(&ld).context("failed to encode link definition")?;
        let lattice_prefix = &self.config.lattice_prefix;
        self.prov_rpc_nats
            .publish(
                format!(
                    "wasmbus.rpc.{lattice_prefix}.{}.{}.linkdefs.put",
                    ld.provider_id, ld.link_name
                ),
                msgp.into(),
            )
            .await
            .context("failed to publish link deinition")?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn delete_linkdef(&self, ld: LinkDefinition) -> anyhow::Result<()> {
        let id = linkdef_hash(&ld.actor_id, &ld.contract_id, &ld.link_name);

        info!("delete linkdef");
        let ref ld @ LinkDefinition {
            ref actor_id,
            ref provider_id,
            ref contract_id,
            ref link_name,
            ..
        } = self
            .links
            .write()
            .await
            .remove(&id)
            .context("attempt to remove a non-exitent link")?;

        if let Some(actor) = self.actors.read().await.get(actor_id) {
            let mut links = actor.links.write().await;
            if let Some(links) = links.get_mut(contract_id) {
                links.remove(link_name);
            }
        }

        let msgp: Vec<u8> = rmp_serde::to_vec(ld).context("failed to encode link definition")?;
        let lattice_prefix: &String = &self.config.lattice_prefix;
        self.prov_rpc_nats
            .publish(
                format!("wasmbus.rpc.{lattice_prefix}.{provider_id}.{link_name}.linkdefs.del",),
                msgp.into(),
            )
            .await
            .context("failed to publish link definition deletion")?;
        Ok(())
    }
}

fn linkdef_hash(
    actor_id: impl AsRef<str>,
    contract_id: impl AsRef<str>,
    link_name: impl AsRef<str>,
) -> String {
    let mut hash = Sha256::default();
    hash.update(actor_id.as_ref());
    hash.update(contract_id.as_ref());
    hash.update(link_name.as_ref());
    hex::encode_upper(hash.finalize())
}
