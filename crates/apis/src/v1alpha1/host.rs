use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use kube::CustomResource;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "kasmcloud.io",
    version = "v1alpha1",
    kind = "KasmCloudHost",
    namespaced,
    status = "KasmCloudHostStatus"
)]
#[kube(category = "kasmcloud")]
#[serde(rename_all = "camelCase")]
pub struct KasmCloudHostSpec {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KasmCloudHostStatus {
    pub kube_node_name: String,
    pub public_key: String,
    pub cluster_public_key: String,
    pub cluster_issuers: Vec<String>,

    pub pre_instance: String,
    pub instance: String,

    pub providers: HostProviderStatus,
    pub actors: HostActorStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostProviderStatus {
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostActorStatus {
    pub count: u64,
    pub instance_count: u64,
}
