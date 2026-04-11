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
    #[serde(default)]
    pub mkdeps: Vec<String>,
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
            if path == "/" {
                return root_index(state);
            }
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
            "/api/reindex" => reindex(headers, body, state),
            "/api/delete" => delete_pkg(headers, body, state),
            _ => Response::not_found(),
        },
        _ => Response::not_found(),
    }
}

fn root_index(state: &AppState) -> Response {
    let indexes = state.indexes.read().unwrap();
    let mut html = String::from(
        "<!doctype html>\
        <html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
        <title>Kominka Packages</title><style>\
        *{margin:0;padding:0;box-sizing:border-box}\
        body{font-family:system-ui,sans-serif;max-width:960px;margin:0 auto;padding:2rem 1rem;\
        color:#e0e0e0;background:#1a1a1a}\
        h1{font-size:1.4rem;margin-bottom:1.5rem;color:#fff}\
        h2{font-size:1.1rem;margin:1.5rem 0 .5rem;color:#ccc}\
        table{width:100%;border-collapse:collapse;font-size:.85rem}\
        th{text-align:left;padding:.3rem .5rem;border-bottom:1px solid #333;color:#888;font-weight:normal}\
        td{padding:.3rem .5rem;border-bottom:1px solid #222}\
        a{color:#6ba3f7;text-decoration:none}a:hover{text-decoration:underline}\
        .dep{color:#888}.mkdep{color:#666}\
        .empty{color:#666;padding:2rem 0}\
        @media(prefers-color-scheme:light){body{color:#222;background:#fff}\
        h1{color:#000}h2{color:#333}th{color:#666;border-color:#ddd}\
        td{border-color:#eee}a{color:#1a6be0}.dep{color:#555}.mkdep{color:#888}}\
        </style></head><body><h1>Kominka Packages</h1>",
    );

    let mut has_packages = false;
    for arch in KNOWN_ARCHES {
        let Some(idx) = indexes.get(*arch) else { continue };
        if idx.packages.is_empty() { continue; }
        has_packages = true;

        let mut names: Vec<&String> = idx.packages.keys().collect();
        names.sort();

        html.push_str(&format!(
            "<h2>{arch} <span class=dep>({} packages)</span></h2>\
            <table><tr><th>Package</th><th>Version</th><th>Dependencies</th><th>Build deps</th></tr>",
            names.len()
        ));

        for name in &names {
            let e = &idx.packages[*name];
            let tarball_url = format!("/{arch}/{name}/{}-{}.tar.gz", e.ver, e.rel);

            let dep_links: Vec<String> = e.deps.iter().map(|d| {
                if names.iter().any(|n| *n == d) {
                    format!("<a href=\"#\">{d}</a>")
                } else {
                    d.clone()
                }
            }).collect();
            let deps_html = if dep_links.is_empty() {
                "<span class=dep>\u{2014}</span>".to_string()
            } else {
                format!("<span class=dep>{}</span>", dep_links.join(", "))
            };

            let mkdep_links: Vec<String> = e.mkdeps.iter().map(|d| {
                if names.iter().any(|n| *n == d) {
                    format!("<a href=\"#\">{d}</a>")
                } else {
                    d.clone()
                }
            }).collect();
            let mkdeps_html = if mkdep_links.is_empty() {
                "<span class=mkdep>\u{2014}</span>".to_string()
            } else {
                format!("<span class=mkdep>{}</span>", mkdep_links.join(", "))
            };

            html.push_str(&format!(
                "<tr><td><a href=\"{tarball_url}\">{name}</a></td>\
                <td>{}-{}</td><td>{deps_html}</td><td>{mkdeps_html}</td></tr>",
                e.ver, e.rel
            ));
        }
        html.push_str("</table>");
    }

    if !has_packages {
        html.push_str("<p class=empty>No packages indexed yet.</p>");
    }

    html.push_str("</body></html>");
    Response { status: 200, content_type: "text/html", body: html.into_bytes() }
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
    let deps_raw = headers.get("x-deps").map(|s| s.as_str()).unwrap_or("");
    let mkdeps_raw = headers.get("x-mkdeps").map(|s| s.as_str()).unwrap_or("");

    if arch.is_empty() || pkg.is_empty() || ver.is_empty() || rel.is_empty() {
        return Response::bad_request("missing headers");
    }
    if !KNOWN_ARCHES.contains(&arch) {
        return Response::bad_request("unknown arch");
    }
    if !valid_pkg_name(pkg) {
        return Response::bad_request("invalid package name");
    }

    let parse_list = |raw: &str| -> Vec<String> {
        if raw.is_empty() {
            vec![]
        } else {
            raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
        }
    };
    let deps = parse_list(deps_raw);
    let mkdeps = parse_list(mkdeps_raw);

    let sha256_hex = s3::sha256_hex(body);

    let key = format!("{arch}/{pkg}/{ver}-{rel}.tar.gz");
    if let Err(e) = state.s3.put(&key, body.to_vec(), "application/octet-stream") {
        tracing::error!("upload failed: {e}");
        return Response::error("upload failed");
    }

    let entry = PackageEntry {
        ver: ver.to_string(),
        rel: rel.to_string(),
        deps,
        mkdeps,
        sha256: sha256_hex.clone(),
    };
    if let Err(e) = update_index(state, arch, pkg, entry) {
        tracing::error!("index update failed: {e}");
        return Response::error("index update failed");
    }

    tracing::info!("uploaded {key}");
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
        mkdeps: req.mkdeps,
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
    deps: Vec<String>,
    #[serde(default)]
    mkdeps: Vec<String>,
}

/// POST /api/reindex — register an already-uploaded tarball in the index
/// without re-uploading. Useful for rebuilding the index from existing R2 objects.
fn reindex(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::check_auth(&state.api_key_hash, headers) {
        return Response::unauthorized();
    }

    #[derive(serde::Deserialize)]
    struct ReindexRequest {
        arch: String,
        pkg: String,
        ver: String,
        rel: String,
        deps: Vec<String>,
        #[serde(default)]
        mkdeps: Vec<String>,
    }

    let req: ReindexRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return Response::bad_request("invalid json"),
    };

    if !KNOWN_ARCHES.contains(&req.arch.as_str()) {
        return Response::bad_request("unknown arch");
    }
    if !valid_pkg_name(&req.pkg) {
        return Response::bad_request("invalid package name");
    }

    // Fetch tarball from S3 to compute sha256.
    let key = format!("{}/{}/{}-{}.tar.gz", req.arch, req.pkg, req.ver, req.rel);
    let tarball = state.s3.get(&key);
    let sha256 = match tarball {
        Some(bytes) => s3::sha256_hex(&bytes),
        None => return Response::bad_request("tarball not found in R2"),
    };

    let entry = PackageEntry {
        ver: req.ver,
        rel: req.rel,
        deps: req.deps,
        mkdeps: req.mkdeps,
        sha256,
    };
    if let Err(e) = update_index(state, &req.arch, &req.pkg, entry) {
        tracing::error!("reindex failed: {e}");
        return Response::error("reindex failed");
    }

    tracing::info!("reindexed {}/{}", req.arch, req.pkg);
    Response::json(200, r#"{"ok":true}"#)
}

/// POST /api/delete — remove a package from the index (does not delete R2 object).
fn delete_pkg(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::check_auth(&state.api_key_hash, headers) {
        return Response::unauthorized();
    }
    #[derive(serde::Deserialize)]
    struct DeleteRequest { arch: String, pkg: String }
    let req: DeleteRequest = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return Response::bad_request("invalid json"),
    };
    if !KNOWN_ARCHES.contains(&req.arch.as_str()) {
        return Response::bad_request("unknown arch");
    }
    let mut indexes = state.indexes.write().unwrap();
    let removed = indexes.get_mut(&req.arch).map(|idx| idx.packages.remove(&req.pkg)).flatten().is_some();
    if removed {
        if let Some(idx) = indexes.get(&req.arch) {
            let _ = save_index(&state.s3, &req.arch, idx);
        }
        tracing::info!("deleted {}/{} from index", req.arch, req.pkg);
        Response::json(200, r#"{"ok":true}"#)
    } else {
        Response::not_found()
    }
}

fn update_index(state: &AppState, arch: &str, pkg: &str, entry: PackageEntry) -> Result<(), String> {
    let mut indexes = state.indexes.write().unwrap();
    let index = indexes.entry(arch.to_string()).or_insert_with(PackageIndex::new);
    index.packages.insert(pkg.to_string(), entry);
    save_index(&state.s3, arch, index)
}
