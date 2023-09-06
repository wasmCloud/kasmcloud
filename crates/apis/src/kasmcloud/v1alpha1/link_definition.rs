use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// use kube::core::{crd::CustomResourceExt, Resource};
use kube::CustomResource;

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "kasmcloud.io",
    version = "v1alpha1",
    kind = "Link",
    namespaced,
    status = "LinkStatus"
)]
#[kube(category = "kasmcloud")]
#[kube(printcolumn = r#"{"name":"ActorKey", "jsonPath": ".status.actorKey", "type": "string"}"#)]
#[kube(
    printcolumn = r#"{"name":"ProviderKey", "jsonPath": ".status.providerKey", "type": "string"}"#
)]
#[kube(printcolumn = r#"{"name":"ControctId", "jsonPath": ".spec.contractId", "type": "string"}"#)]
#[serde(rename_all = "camelCase")]
pub struct LinkSpec {
    pub provider: Source,
    pub actor: Source,
    pub contract_id: String,
    pub values: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub provider_key: String,
    pub actor_key: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub key: Option<String>,
    pub name: Option<String>,
}
