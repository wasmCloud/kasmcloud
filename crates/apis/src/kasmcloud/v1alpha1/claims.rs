use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct Claims {
    pub issuer: String,
    pub subject: String,
    pub not_before: Option<u64>,
    pub issued_at: u64,
    pub expires: Option<u64>,
}
