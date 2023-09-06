use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// use kube::core::{crd::CustomResourceExt, Resource};
use kube::CustomResource;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "kasmcloud.io",
    version = "v1alpha1",
    kind = "Provider",
    namespaced,
    status = "ProviderStatus"
)]
#[kube(printcolumn = r#"{"name":"Desc", "jsonPath": ".status.descriptiveName", "type": "string"}"#)]
#[kube(printcolumn = r#"{"name":"PublicKey", "jsonPath": ".status.publicKey", "type": "string"}"#)]
#[kube(printcolumn = r#"{"name":"Link", "jsonPath": ".spec.link", "type": "string"}"#)]
#[kube(
    printcolumn = r#"{"name":"ControctId", "jsonPath": ".status.contractId", "type": "string"}"#
)]
#[kube(printcolumn = r#"{"name":"Image", "jsonPath": ".spec.image", "type": "string"}"#)]
#[kube(category = "kasmcloud")]
#[serde(rename_all = "camelCase")]
pub struct ProviderSpec {
    pub host: String,
    pub image: String,
    pub link: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub public_key: String,
    pub contract_id: String,
    pub descriptive_name: Option<String>,

    pub vendor: String,
    pub reversion: Option<i32>,
    pub version: Option<String>,
    pub claims: super::Claims,

    pub architecture_targets: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition>,
    pub instance_id: String,
}
