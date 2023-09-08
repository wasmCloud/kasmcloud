use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use k8s_openapi::apimachinery::pkg::apis::meta::v1 as metav1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Claims {
    pub issuer: String,
    pub subject: String,
    pub issued_at: metav1::Time,
    pub not_before: Option<metav1::Time>,
    pub expires: Option<metav1::Time>,
}
