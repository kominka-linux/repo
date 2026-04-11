pub mod auth;
pub mod packages;
pub mod s3;

use std::collections::HashMap;
use std::sync::RwLock;

pub struct AppState {
    pub s3: s3::Storage,
    pub api_key_hash: [u8; 32],
    pub indexes: RwLock<HashMap<String, packages::PackageIndex>>,
}
