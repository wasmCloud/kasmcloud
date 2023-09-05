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
#[kube(printcolumn = r#"{"name":"ActorKey", "jsonPath": ".status.actor_key", "type": "string"}"#)]
#[kube(
    printcolumn = r#"{"name":"ProviderKey", "jsonPath": ".status.provider_key", "type": "string"}"#
)]
#[kube(printcolumn = r#"{"name":"ControctId", "jsonPath": ".spec.contract_id", "type": "string"}"#)]
pub struct LinkSpec {
    pub provider: Source,
    pub actor: Source,
    pub contract_id: String,
    pub values: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct LinkStatus {
    pub provider_key: String,
    pub actor_key: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Source {
    pub key: Option<String>,
    pub name: Option<String>,
}
