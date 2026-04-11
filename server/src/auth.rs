use sha2::{Digest, Sha256};

/// Validate Bearer token against the stored API key hash.
pub fn check_auth(api_key_hash: &[u8; 32], headers: &std::collections::HashMap<String, String>) -> bool {
    let auth = headers.get("authorization").map(|s| s.as_str()).unwrap_or("");
    let token = match auth.strip_prefix("Bearer ") {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    hash == *api_key_hash
}
