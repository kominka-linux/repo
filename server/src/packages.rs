use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::auth;
use crate::s3::S3Client;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageEntry {
    pub ver: String,
    pub rel: String,
    pub deps: Vec<String>,
    pub hash: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageIndex {
    pub _version: u32,
    pub packages: HashMap<String, PackageEntry>,
}

impl PackageIndex {
    fn new() -> Self {
        Self {
            _version: 1,
            packages: HashMap::new(),
        }
    }
}

/// Load a packages.json from S3. Returns None if it doesn't exist.
pub async fn load_index_from_s3(s3: &S3Client, arch: &str) -> Option<PackageIndex> {
    let key = format!("{arch}/packages.json");
    let bytes = s3.get(&key).await?;
    serde_json::from_slice(&bytes).ok()
}

/// Save a packages.json to S3.
async fn save_index_to_s3(s3: &S3Client, arch: &str, index: &PackageIndex) -> Result<(), String> {
    let key = format!("{arch}/packages.json");
    let json = serde_json::to_vec_pretty(index).map_err(|e| format!("json: {e}"))?;
    s3.put(&key, json, "application/json").await
}

const KNOWN_ARCHES: &[&str] = &["aarch64-linux-gnu", "x86_64-linux-gnu"];

fn valid_pkg_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'_')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

// GET /{arch}/packages.json
pub async fn get_index(
    State(state): State<Arc<AppState>>,
    Path(arch): Path<String>,
) -> Response {
    if !KNOWN_ARCHES.contains(&arch.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let indexes = state.indexes.read().await;
    if let Some(idx) = indexes.get(&arch) {
        let json = serde_json::to_vec_pretty(idx).unwrap();
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "public, max-age=60"),
            ],
            json,
        )
            .into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

// GET /{arch}/{pkg}/{file}
pub async fn get_tarball(
    State(state): State<Arc<AppState>>,
    Path((arch, pkg, file)): Path<(String, String, String)>,
) -> Response {
    if !KNOWN_ARCHES.contains(&arch.as_str()) || !valid_pkg_name(&pkg) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let key = format!("{arch}/{pkg}/{file}");
    match state.s3.get(&key).await {
        Some(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// POST /api/upload
pub async fn upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    // Validate auth.
    if let Err(status) = auth::check_auth(&state.api_key_hash, &headers) {
        return status.into_response();
    }

    // Extract metadata from headers.
    let arch = header_str(&headers, "x-arch");
    let pkg = header_str(&headers, "x-pkg");
    let ver = header_str(&headers, "x-ver");
    let rel = header_str(&headers, "x-rel");
    let hash = header_str(&headers, "x-hash");
    let deps_raw = header_str(&headers, "x-deps");

    if arch.is_empty() || pkg.is_empty() || ver.is_empty() || rel.is_empty() || hash.is_empty() {
        return (StatusCode::BAD_REQUEST, r#"{"error":"missing headers"}"#).into_response();
    }
    if !KNOWN_ARCHES.contains(&arch.as_str()) {
        return (StatusCode::BAD_REQUEST, r#"{"error":"unknown arch"}"#).into_response();
    }
    if !valid_pkg_name(&pkg) {
        return (StatusCode::BAD_REQUEST, r#"{"error":"invalid package name"}"#).into_response();
    }

    let deps: Vec<String> = if deps_raw.is_empty() {
        vec![]
    } else {
        deps_raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    // Read body and compute SHA-256.
    let bytes = match axum::body::to_bytes(body, 500 * 1024 * 1024).await {
        Ok(b) => b.to_vec(),
        Err(_) => {
            return (StatusCode::BAD_REQUEST, r#"{"error":"body too large"}"#).into_response();
        }
    };

    let sha256_hex = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex::encode(hasher.finalize())
    };

    // Upload tarball to S3.
    let key = format!("{arch}/{pkg}/{hash}.tar.gz");
    if let Err(e) = state.s3.put(&key, bytes, "application/octet-stream").await {
        tracing::error!("upload failed: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, r#"{"error":"upload failed"}"#)
            .into_response();
    }

    // Update index.
    let entry = PackageEntry {
        ver,
        rel,
        deps,
        hash: hash.clone(),
        sha256: sha256_hex.clone(),
    };
    if let Err(e) = update_index(&state, &arch, &pkg, entry).await {
        tracing::error!("index update failed: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, r#"{"error":"index update failed"}"#)
            .into_response();
    }

    tracing::info!("uploaded {arch}/{pkg}/{hash}.tar.gz");
    (
        StatusCode::CREATED,
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"ok":true,"sha256":"{sha256_hex}"}}"#),
    )
        .into_response()
}

/// POST /api/publish — register a metapackage (no tarball).
pub async fn publish(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<PublishRequest>,
) -> Response {
    if let Err(status) = auth::check_auth(&state.api_key_hash, &headers) {
        return status.into_response();
    }

    if !KNOWN_ARCHES.contains(&body.arch.as_str()) {
        return (StatusCode::BAD_REQUEST, r#"{"error":"unknown arch"}"#).into_response();
    }
    if !valid_pkg_name(&body.pkg) {
        return (StatusCode::BAD_REQUEST, r#"{"error":"invalid package name"}"#).into_response();
    }

    let entry = PackageEntry {
        ver: body.ver,
        rel: body.rel,
        deps: body.deps,
        hash: body.hash.clone(),
        sha256: String::new(),
    };
    if let Err(e) = update_index(&state, &body.arch, &body.pkg, entry).await {
        tracing::error!("index update failed: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, r#"{"error":"index update failed"}"#)
            .into_response();
    }

    tracing::info!("published metapackage {}/{}", body.arch, body.pkg);
    (
        StatusCode::CREATED,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true}"#,
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct PublishRequest {
    pub arch: String,
    pub pkg: String,
    pub ver: String,
    pub rel: String,
    pub hash: String,
    pub deps: Vec<String>,
}

async fn update_index(
    state: &AppState,
    arch: &str,
    pkg: &str,
    entry: PackageEntry,
) -> Result<(), String> {
    let mut indexes = state.indexes.write().await;
    let index = indexes
        .entry(arch.to_string())
        .or_insert_with(PackageIndex::new);
    index.packages.insert(pkg.to_string(), entry);
    save_index_to_s3(&state.s3, arch, index).await
}

fn header_str(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .fold(String::new(), |mut s, b| {
                use std::fmt::Write;
                write!(s, "{b:02x}").unwrap();
                s
            })
    }
}
