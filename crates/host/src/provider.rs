use std::collections::HashMap;

use tokio::process::Child;
use ulid::Ulid;

use wascap::jwt;

pub struct Provider {
    pub claims: jwt::Claims<jwt::CapabilityProvider>,
    pub instances: HashMap<String, ProviderInstance>,
    pub image_ref: String,
}

pub struct ProviderInstance {
    pub child: Child,
    pub id: Ulid,
}
