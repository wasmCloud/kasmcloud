use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use kube::CustomResource;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "kasmcloud.io",
    version = "v1alpha1",
    kind = "Actor",
    namespaced,
    status = "ActorStatus",
    scale = r#"{"specReplicasPath":".spec.replicas", "statusReplicasPath":".status.replicas"}"#
)]
#[kube(category = "kasmcloud")]
#[serde(rename_all = "camelCase")]
pub struct ActorSpec {
    pub host: String,
    pub image: String,
    pub replicas: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActorStatus {
    pub public_key: String,
    pub descriptive_name: Option<String>,

    pub caps: Option<Vec<String>>,
    pub capability_provider: Option<Vec<String>>,
    pub call_alias: Option<String>,
    pub claims: super::Claims,

    pub version: Option<String>,
    pub reversion: Option<i32>,

    pub conditions: Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition>,
    pub replicas: i32,
    pub available_replicas: i32,
}
