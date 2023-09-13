use core::time::Duration;
use std::collections::{hash_map, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use chrono::{DateTime, LocalResult, TimeZone, Utc};
use clap::Parser;
use config::{Config, Environment, File};
use futures::{future::ready, join, StreamExt};
use nkeys::KeyPair;
use serde_derive::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;
use url::Url;

use k8s_openapi::apimachinery::pkg::apis::meta::v1 as metav1;
use kube::api::{Patch, PatchParams};
use kube::core::Resource;
use kube::runtime::controller::Action;
use kube::runtime::watcher::Config as WatchConfig;
use kube::runtime::{finalizer, Controller};
use kube::{Api, Client, ResourceExt};
use wasmcloud_control_interface::LinkDefinition;

use kasmcloud_apis::v1alpha1;
use kasmcloud_host::*;

#[derive(Debug, Parser)]
#[allow(clippy::struct_excessive_bools)]
#[command(version, about, long_about = None)]
struct Args {
    #[clap(long = "config", default_value = "/etc/kasmcloud/config.yaml")]
    config: String,
}

#[derive(Debug, Default, Deserialize)]
struct KasmCloudConfig {
    nats_host: String,
    nats_port: u16,
    nats_jwt: Option<String>,
    nats_seed: Option<String>,
    // lattice_prefix: String,
    host_seed: Option<String>,
    cluster_seed: Option<String>,
    cluster_issuers: Option<Vec<String>>,
    js_domain: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = Args::parse();

    let mut builder = Config::builder()
        .set_default("nats_host", "127.0.0.1")?
        .set_default("nats_port", "4222")?
        .add_source(Environment::with_prefix("kasmcloud"));

    if Path::new(&args.config).exists() {
        builder = builder.add_source(File::with_name(&args.config))
    }
    let config: KasmCloudConfig = builder.build()?.try_deserialize()?;

    let host_key = config
        .host_seed
        .as_deref()
        .map(KeyPair::from_seed)
        .transpose()
        .context("failed to contruct host key pair from seed")?
        .map(Arc::new);
    let cluster_key = config
        .cluster_seed
        .as_deref()
        .map(KeyPair::from_seed)
        .transpose()
        .context("failed to contruct cluster key pair from seed")?
        .map(Arc::new);
    let nats_key = config
        .nats_seed
        .as_deref()
        .map(KeyPair::from_seed)
        .transpose()
        .context("failed to contruct NATS key pair from seed")?
        .map(Arc::new);

    let rpc_config = NatsConfig {
        url: Url::parse(&format!("nats://{}:{}", config.nats_host, config.nats_port,))
            .context("failed to parse nats")?,
        jwt: config.nats_jwt,
        key: nats_key,
        tls: false,
        timeout: Some(Duration::from_secs(2)),
    };
    let prov_rpc_config = NatsConfig {
        timeout: None,
        ..rpc_config.clone()
    };

    let namespace = "default".to_string();
    let config = HostConfig {
        rpc_config,
        prov_rpc_config,
        js_domain: config.js_domain,

        host_key,
        lattice_prefix: namespace.clone(),

        cluster_key,
        cluster_issuers: config.cluster_issuers,

        log_level: Some(wasmcloud_core::logging::Level::Debug),
        allow_file_load: true,
    };
    let host = Host::new(config)
        .await
        .context("failed new actor handler")?;

    let client = Client::try_default().await?;
    let actors: Api<v1alpha1::Actor> = Api::namespaced(client.clone(), &namespace);
    let providers: Api<v1alpha1::Provider> = Api::namespaced(client.clone(), &namespace);
    let links: Api<v1alpha1::Link> = Api::namespaced(client.clone(), &namespace);

    let ctx = Arc::new(Ctx {
        actor_client: actors.clone(),
        provider_client: providers.clone(),
        link_client: links.clone(),
        host,
        links: RwLock::default(),
    });

    let links = Controller::new(links, WatchConfig::default())
        .run(
            |link, ctx| async move {
                let links = ctx.link_client.clone();
                finalizer::finalizer(&links, "kasmcloud-host/cleanup", link, |event| async {
                    match event {
                        finalizer::Event::Apply(link) => add_link(link, ctx).await,
                        finalizer::Event::Cleanup(link) => delete_link(link, ctx).await,
                    }
                })
                .await
            },
            |_obj, _err, _| Action::requeue(Duration::from_secs(2)),
            Arc::clone(&ctx),
        )
        .for_each(|_| ready(()));

    let actors = Controller::new(actors, WatchConfig::default())
        .run(
            |actor, ctx| async move {
                let actors = ctx.actor_client.clone();
                finalizer::finalizer(&actors, "kasmcloud-host/cleanup", actor, |event| async {
                    match event {
                        finalizer::Event::Apply(actor) => reconcile_actor(actor, ctx).await,
                        finalizer::Event::Cleanup(actor) => delete_actor(actor, ctx).await,
                    }
                })
                .await
            },
            |_obj, _err, _| Action::requeue(Duration::from_secs(2)),
            Arc::clone(&ctx),
        )
        .for_each(|_| ready(()));

    let providers = Controller::new(providers, WatchConfig::default())
        .run(
            |actor, ctx| async move {
                let providers = ctx.provider_client.clone();
                finalizer::finalizer(&providers, "kasmcloud-host/cleanup", actor, |event| async {
                    match event {
                        finalizer::Event::Apply(actor) => reconcile_provider(actor, ctx).await,
                        finalizer::Event::Cleanup(actor) => delete_provider(actor, ctx).await,
                    }
                })
                .await
            },
            |_obj, _err, _| Action::requeue(Duration::from_secs(2)),
            Arc::clone(&ctx),
        )
        .for_each(|_| ready(()));

    join!(actors, providers, links);

    Ok(())
}

// TODO(Iceber): add reflector store
struct Ctx {
    actor_client: Api<v1alpha1::Actor>,
    provider_client: Api<v1alpha1::Provider>,
    link_client: Api<v1alpha1::Link>,
    host: Host,

    links: RwLock<HashMap<String, (LinkDefinition, HashSet<String>)>>,
}

#[derive(Debug, Error)]
enum Error {}

async fn reconcile_actor(actor: Arc<v1alpha1::Actor>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    println!("applied actor: {:#?}", actor.name_any());

    match ctx
        .host
        .reconcile_actor(
            actor.name_any(),
            actor.spec.image.clone(),
            actor.spec.replicas as usize,
        )
        .await
    {
        Ok(claims) => {
            let public_key = claims.subject.clone();
            let issued_at =
                if let LocalResult::Single(dt) = Utc.timestamp_opt(claims.issued_at as i64, 0) {
                    dt
                } else {
                    DateTime::<Utc>::MIN_UTC
                };
            let mut c = v1alpha1::Claims {
                issuer: claims.issuer.clone(),
                subject: claims.subject.clone(),
                issued_at: metav1::Time(issued_at),
                not_before: None,
                expires: None,
            };
            if let Some(t) = claims.not_before {
                c.not_before = Some(metav1::Time(
                    if let LocalResult::Single(dt) = Utc.timestamp_opt(t as i64, 0) {
                        dt
                    } else {
                        DateTime::<Utc>::MIN_UTC
                    },
                ))
            }
            if let Some(t) = claims.expires {
                c.expires = Some(metav1::Time(
                    if let LocalResult::Single(dt) = Utc.timestamp_opt(t as i64, 0) {
                        dt
                    } else {
                        DateTime::<Utc>::MIN_UTC
                    },
                ))
            }

            let mut status = v1alpha1::ActorStatus {
                public_key,
                claims: c,
                descriptive_name: None,
                caps: None,
                capability_provider: None,
                call_alias: None,
                version: None,
                reversion: None,
                conditions: Vec::new(),
                available_replicas: actor.spec.replicas,
            };
            if let Some(meta) = claims.metadata {
                status.descriptive_name = meta.name;
                if meta.provider {
                    status.caps = meta.caps;
                } else {
                    status.capability_provider = meta.caps;
                }
                status.reversion = meta.rev;
                status.version = meta.ver;
                status.call_alias = meta.call_alias;
            }

            let data = serde_json::json!({ "status": status });
            if let Some(s) = actor.status.clone() {
                if s == status {
                    return Ok(Action::await_change());
                }
            }

            if let Err(err) = ctx
                .actor_client
                .patch_status(
                    actor.name_any().as_str(),
                    &PatchParams::default(),
                    &Patch::Merge(&data),
                )
                .await
            {
                println!("patch actor failed: {:?}", err);
            }
        }
        Err(err) => {
            println!("reconcile actor failed: {:?}", err);
        }
    }

    Ok(Action::await_change())
}

async fn delete_actor(actor: Arc<v1alpha1::Actor>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    println!("delete actor: {:#?}", actor.name_any());

    if let Err(err) = ctx.host.remove_actor(actor.name_any()).await {
        println!("delete actor failed: {:?}", err);
        Ok(Action::requeue(Duration::from_secs(60)))
    } else {
        Ok(Action::await_change())
    }
}

async fn add_link(link: Arc<v1alpha1::Link>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    println!("add link: {}", link.name_any());
    let spec = &link.spec;

    let mut ld = LinkDefinition::default();
    ld.contract_id = spec.contract_id.clone();
    ld.values = spec.values.clone();
    if !spec.link_name.is_empty() {
        ld.link_name = spec.link_name.clone();
    } else if !spec.provider.key.is_empty() {
        println!("please set provider's link name");
        return Ok(Action::await_change());
    }

    if let Some(status) = &link.status {
        if !status.provider_key.is_empty()
            && !status.actor_key.is_empty()
            && !status.link_name.is_empty()
        {
            ld.actor_id = status.actor_key.clone();
            ld.provider_id = status.provider_key.clone();
            ld.link_name = status.link_name.clone();
        }
    }
    if ld.actor_id.is_empty() {
        let key = &spec.actor.key;
        let name = &spec.actor.name;
        if !key.is_empty() {
            ld.actor_id = key.clone();
        } else if !name.is_empty() {
            match ctx.actor_client.get(name).await {
                Ok(actor) => {
                    if let Some(status) = actor.status {
                        if status.public_key == "" {
                            println!("actor:{name}  public key is empty");
                            return Ok(Action::requeue(Duration::from_secs(10)));
                        } else {
                            ld.actor_id = status.public_key
                        }
                    } else {
                        println!("actor:{name} status is None");
                        return Ok(Action::requeue(Duration::from_secs(10)));
                    }
                }
                Err(err) => {
                    println!("get actor:{name} failed: {:?}", err);
                    return Ok(Action::requeue(Duration::from_secs(60)));
                }
            }
        } else {
            // TODO(Iceber): Add log or error
            println!("link({})'s actor is empty", link.name_any());
            return Ok(Action::await_change());
        };
    }
    if ld.provider_id.is_empty() || ld.link_name.is_empty() {
        let key = &spec.provider.key;
        let name = &spec.provider.name;
        if !name.is_empty() {
            match ctx.provider_client.get(name).await {
                Ok(provider) => {
                    if !ld.link_name.is_empty() && ld.link_name != provider.spec.link {
                        // TODO(Iceber): update conditions
                        println!(
                            "linkdefinitions({}) not match provider's link({})",
                            link.name_any(),
                            provider.spec.link
                        );
                        return Ok(Action::await_change());
                    }

                    if let Some(status) = provider.status {
                        if status.public_key.is_empty() {
                            println!("provider:{name} public key is empty");
                            return Ok(Action::requeue(Duration::from_secs(10)));
                        } else if !key.is_empty() && status.public_key != key.clone() {
                            println!("provider:{name} public key is not match provider.key");
                            return Ok(Action::requeue(Duration::from_secs(10)));
                        } else {
                            ld.provider_id = status.public_key;
                            ld.link_name = provider.spec.link;
                        }
                    } else {
                        println!("provider:{name} status is None");
                        return Ok(Action::requeue(Duration::from_secs(10)));
                    }
                }
                Err(err) => {
                    println!("get provider:{name} failed: {:?}", err);
                    return Ok(Action::requeue(Duration::from_secs(60)));
                }
            }
        } else if !key.is_empty() {
            ld.provider_id = key.clone();
        } else {
            println!("link({})'s provider is empty", link.name_any());
            return Ok(Action::await_change());
        };
    }

    let id = format!("{}.{}.{}", ld.contract_id, ld.actor_id, ld.link_name);
    match ctx.links.write().await.entry(id.to_string()) {
        hash_map::Entry::Vacant(entry) => {
            entry.insert((ld.clone(), HashSet::from([link.name_any()])));
            if let Err(err) = ctx.host.add_linkdef(ld.clone()).await {
                println!("add linkdef failed: {:?}", err);
            }
        }
        hash_map::Entry::Occupied(mut entry) => {
            // TODO(iceber):
            // 1. if the provider_id has changed, then you need to error or
            // otherwise handle the change.
            // 2. if ld.values is not equal group.link.values, return error.
            let (_, ref_names) = entry.get_mut();
            ref_names.insert(link.name_any());
        }
    }

    let status = v1alpha1::LinkStatus {
        provider_key: ld.provider_id.clone(),
        actor_key: ld.actor_id.clone(),
        link_name: ld.link_name.clone(),
        conditions: Vec::new(),
    };
    let data = serde_json::json!({ "status": status });

    if let Some(s) = link.status.clone() {
        if s == status {
            return Ok(Action::await_change());
        }
    }

    if let Err(err) = ctx
        .link_client
        .patch_status(
            link.name_any().as_str(),
            &PatchParams::default(),
            &Patch::Merge(&data),
        )
        .await
    {
        println!("patch link failed: {:?}", err);
    }
    Ok(Action::await_change())
}

async fn delete_link(link: Arc<v1alpha1::Link>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    println!("delete link: {}", link.name_any());
    if let Some(status) = &link.status {
        if !status.provider_key.is_empty()
            && !status.actor_key.is_empty()
            && !status.link_name.is_empty()
        {
            let id = format!(
                "{}.{}.{}",
                link.spec.contract_id, status.actor_key, status.link_name
            );
            if let hash_map::Entry::Occupied(mut entry) = ctx.links.write().await.entry(id.clone())
            {
                let (ld, ref_names) = entry.get_mut();
                ref_names.remove(&link.name_any());
                if ref_names.is_empty() {
                    if let Err(err) = ctx.host.delete_linkdef(ld.clone()).await {
                        println!("delete linkdef failed: {:?}", err);
                        return Ok(Action::requeue(Duration::from_secs(10)));
                    }
                    entry.remove();
                }
            }
        }
    }
    Ok(Action::await_change())
}

async fn reconcile_provider(g: Arc<v1alpha1::Provider>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    println!("applied provider: {:#?}", g.name_any());

    let spec = g.spec.clone();
    match ctx
        .host
        .start_provider(spec.link.clone(), spec.image.clone())
        .await
    {
        Ok(claims) => {
            let public_key = claims.subject.clone();
            let issued_at =
                if let LocalResult::Single(dt) = Utc.timestamp_opt(claims.issued_at as i64, 0) {
                    dt
                } else {
                    DateTime::<Utc>::MIN_UTC
                };
            let mut c = v1alpha1::Claims {
                issuer: claims.issuer.clone(),
                subject: claims.subject.clone(),
                issued_at: metav1::Time(issued_at),
                not_before: None,
                expires: None,
            };
            if let Some(t) = claims.not_before {
                c.not_before = Some(metav1::Time(
                    if let LocalResult::Single(dt) = Utc.timestamp_opt(t as i64, 0) {
                        dt
                    } else {
                        DateTime::<Utc>::MIN_UTC
                    },
                ))
            }
            if let Some(t) = claims.expires {
                c.expires = Some(metav1::Time(
                    if let LocalResult::Single(dt) = Utc.timestamp_opt(t as i64, 0) {
                        dt
                    } else {
                        DateTime::<Utc>::MIN_UTC
                    },
                ))
            }

            let mut status = v1alpha1::ProviderStatus {
                public_key,
                descriptive_name: None,
                contract_id: "".to_string(),

                vendor: "".to_string(),
                reversion: None,
                version: None,
                claims: c,

                architecture_targets: Vec::new(),

                conditions: Vec::new(),
                instance_id: "".to_string(),
            };
            if let Some(meta) = claims.metadata {
                status.descriptive_name = meta.name;
                status.reversion = meta.rev;
                status.version = meta.ver;
                status.vendor = meta.vendor;
                status.contract_id = meta.capid;
                status.architecture_targets = meta.target_hashes.into_keys().collect();
                status.architecture_targets.sort_by(|x, y| x.cmp(&y));
            }

            let data = serde_json::json!({ "status": status });

            if let Some(s) = g.status.clone() {
                println!("status is not empty");
                println!("old provider data: {:?}", &s);
                println!("provider data: {:?}", &status);
                if s == status {
                    return Ok(Action::await_change());
                }
            }

            if let Err(err) = ctx
                .provider_client
                .patch_status(
                    g.name_any().as_str(),
                    &PatchParams::default(),
                    &Patch::Merge(&data),
                )
                .await
            {
                println!("patch provider failed: {:?}", err);
            }
        }
        Err(err) => {
            println!("reconcile provider failed: {:?}", err);
        }
    }

    Ok(Action::await_change())
}

async fn delete_provider(
    provider: Arc<v1alpha1::Provider>,
    ctx: Arc<Ctx>,
) -> Result<Action, Error> {
    let status = if let Some(status) = provider.status.clone() {
        if status.public_key == "" {
            return Ok(Action::await_change());
        }
        status
    } else {
        return Ok(Action::await_change());
    };
    println!("handle deleted provider");

    if let Err(err) = ctx
        .host
        .stop_provider(provider.spec.link.clone(), status.public_key.clone())
        .await
    {
        println!("delete provider failed: {:?}", err);
    }

    Ok(Action::await_change())
}
