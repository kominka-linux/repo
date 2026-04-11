use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth;
use crate::s3;

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
        Self { _version: 1, packages: HashMap::new() }
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn json(status: u16, body: &str) -> Self {
        Self { status, content_type: "application/json", body: body.as_bytes().to_vec() }
    }

    fn json_bytes(status: u16, body: Vec<u8>) -> Self {
        Self { status, content_type: "application/json", body }
    }

    fn octet(body: Vec<u8>) -> Self {
        Self { status: 200, content_type: "application/octet-stream", body }
    }

    fn not_found() -> Self {
        Self { status: 404, content_type: "text/plain", body: b"Not Found".to_vec() }
    }

    fn unauthorized() -> Self {
        Self::json(401, r#"{"error":"unauthorized"}"#)
    }

    fn bad_request(msg: &str) -> Self {
        Self::json(400, &format!(r#"{{"error":"{msg}"}}"#))
    }

    fn error(msg: &str) -> Self {
        Self::json(500, &format!(r#"{{"error":"{msg}"}}"#))
    }
}

pub const KNOWN_ARCHES: &[&str] = &["aarch64-linux-gnu", "x86_64-linux-gnu"];

fn valid_pkg_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'_'
        })
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

/// Load a packages.json from storage.
pub fn load_index(s3: &s3::Storage, arch: &str) -> Option<PackageIndex> {
    let key = format!("{arch}/packages.json");
    let bytes = s3.get(&key)?;
    serde_json::from_slice(&bytes).ok()
}

fn save_index(s3: &s3::Storage, arch: &str, index: &PackageIndex) -> Result<(), String> {
    let key = format!("{arch}/packages.json");
    let json = serde_json::to_vec_pretty(index).map_err(|e| format!("json: {e}"))?;
    s3.put(&key, json, "application/json")
}

/// Route a request to the appropriate handler.
pub fn route(
    method: &str,
    path: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    state: &AppState,
) -> Response {
    match method {
        "GET" => {
            if path == "/health" {
                return Response::json(200, r#"{"status":"ok"}"#);
            }
            if let Some(arch) = path.strip_prefix('/').and_then(|p| p.strip_suffix("/packages.json")) {
                return get_index(arch, state);
            }
            // /{arch}/{pkg}/{file}
            let parts: Vec<&str> = path.trim_start_matches('/').splitn(3, '/').collect();
            if parts.len() == 3 {
                return get_tarball(&parts[0], &parts[1], parts[2], state);
            }
            Response::not_found()
        }
        "POST" => match path {
            "/api/upload" => upload(headers, body, state),
            "/api/publish" => publish(headers, body, state),
            _ => Response::not_found(),
        },
        _ => Response::not_found(),
    }
}

fn get_index(arch: &str, state: &AppState) -> Response {
    if !KNOWN_ARCHES.contains(&arch) {
        return Response::not_found();
    }
    let indexes = state.indexes.read().unwrap();
    match indexes.get(arch) {
        Some(idx) => {
            let json = serde_json::to_vec_pretty(idx).unwrap();
            Response {
                status: 200,
                content_type: "application/json",
                body: json,
            }
        }
        None => Response::not_found(),
    }
}

fn get_tarball(arch: &str, pkg: &str, file: &str, state: &AppState) -> Response {
    if !KNOWN_ARCHES.contains(&arch) || !valid_pkg_name(pkg) {
        return Response::not_found();
    }
    let key = format!("{arch}/{pkg}/{file}");
    match state.s3.get(&key) {
        Some(bytes) => Response::octet(bytes),
        None => Response::not_found(),
    }
}

fn upload(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::check_auth(&state.api_key_hash, headers) {
        return Response::unauthorized();
    }

    let arch = headers.get("x-arch").map(|s| s.as_str()).unwrap_or("");
    let pkg = headers.get("x-pkg").map(|s| s.as_str()).unwrap_or("");
    let ver = headers.get("x-ver").map(|s| s.as_str()).unwrap_or("");
    let rel = headers.get("x-rel").map(|s| s.as_str()).unwrap_or("");
    let hash = headers.get("x-hash").map(|s| s.as_str()).unwrap_or("");
    let deps_raw = headers.get("x-deps").map(|s| s.as_str()).unwrap_or("");

    if arch.is_empty() || pkg.is_empty() || ver.is_empty() || rel.is_empty() || hash.is_empty() {
        return Response::bad_request("missing headers");
    }
    if !KNOWN_ARCHES.contains(&arch) {
        return Response::bad_request("unknown arch");
    }
    if !valid_pkg_name(pkg) {
        return Response::bad_request("invalid package name");
    }

    let deps: Vec<String> = if deps_raw.is_empty() {
        vec![]
    } else {
        deps_raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
    };

    let sha256_hex = s3::sha256_hex(body);

    let key = format!("{arch}/{pkg}/{hash}.tar.gz");
    if let Err(e) = state.s3.put(&key, body.to_vec(), "application/octet-stream") {
        tracing::error!("upload failed: {e}");
        return Response::error("upload failed");
    }

    let entry = PackageEntry {
        ver: ver.to_string(),
        rel: rel.to_string(),
        deps,
        hash: hash.to_string(),
        sha256: sha256_hex.clone(),
    };
    if let Err(e) = update_index(state, arch, pkg, entry) {
        tracing::error!("index update failed: {e}");
        return Response::error("index update failed");
    }

    tracing::info!("uploaded {arch}/{pkg}/{hash}.tar.gz");
    Response::json_bytes(201, format!(r#"{{"ok":true,"sha256":"{sha256_hex}"}}"#).into_bytes())
}

fn publish(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::check_auth(&state.api_key_hash, headers) {
        return Response::unauthorized();
    }

    let req: PublishRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return Response::bad_request("invalid json"),
    };

    if !KNOWN_ARCHES.contains(&req.arch.as_str()) {
        return Response::bad_request("unknown arch");
    }
    if !valid_pkg_name(&req.pkg) {
        return Response::bad_request("invalid package name");
    }

    let entry = PackageEntry {
        ver: req.ver,
        rel: req.rel,
        deps: req.deps,
        hash: req.hash.clone(),
        sha256: String::new(),
    };
    if let Err(e) = update_index(state, &req.arch, &req.pkg, entry) {
        tracing::error!("index update failed: {e}");
        return Response::error("index update failed");
    }

    tracing::info!("published metapackage {}/{}", req.arch, req.pkg);
    Response::json(201, r#"{"ok":true}"#)
}

#[derive(Deserialize)]
struct PublishRequest {
    arch: String,
    pkg: String,
    ver: String,
    rel: String,
    hash: String,
    deps: Vec<String>,
}

fn update_index(state: &AppState, arch: &str, pkg: &str, entry: PackageEntry) -> Result<(), String> {
    let mut indexes = state.indexes.write().unwrap();
    let index = indexes.entry(arch.to_string()).or_insert_with(PackageIndex::new);
    index.packages.insert(pkg.to_string(), entry);
    save_index(&state.s3, arch, index)
}
