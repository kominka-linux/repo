use axum::http::{HeaderMap, StatusCode};
use sha2::{Digest, Sha256};

/// Validate Bearer token against the stored API key hash.
pub fn check_auth(api_key_hash: &[u8; 32], headers: &HeaderMap) -> Result<(), StatusCode> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = auth.strip_prefix("Bearer ").unwrap_or("");
    if token.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    if hash != *api_key_hash {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}
