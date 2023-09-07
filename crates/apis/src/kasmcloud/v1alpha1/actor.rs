use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use kube::CustomResource;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "kasmcloud.io",
    version = "v1alpha1",
    kind = "Actor",
    namespaced,
    status = "ActorStatus"
)]
#[kube(category = "kasmcloud")]
#[serde(rename_all = "camelCase")]
pub struct ActorSpec {
    pub host: String,
    pub image: String,
    pub replica: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActorStatus {
    pub public_key: String,
    pub descriptive_name: Option<String>,

    pub caps: Option<Vec<String>>,
    pub capability_provider: Option<Vec<String>>,

    pub call_alias: Option<String>,
    pub version: Option<String>,
    pub reversion: Option<i32>,

    pub claims: super::Claims,

    pub conditions: Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition>,
    pub available_replicas: usize,
}
