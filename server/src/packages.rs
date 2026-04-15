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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_size: Option<u64>,
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
    pub extra_headers: Vec<(&'static str, String)>,
}

impl Response {
    pub fn json(status: u16, body: &str) -> Self {
        Self { status, content_type: "application/json", body: body.as_bytes().to_vec(), extra_headers: vec![] }
    }

    pub fn json_bytes(status: u16, body: Vec<u8>) -> Self {
        Self { status, content_type: "application/json", body, extra_headers: vec![] }
    }

    pub fn html(body: Vec<u8>) -> Self {
        Self { status: 200, content_type: "text/html; charset=utf-8", body, extra_headers: vec![] }
    }

    pub fn octet(body: Vec<u8>) -> Self {
        Self { status: 200, content_type: "application/octet-stream", body, extra_headers: vec![] }
    }

    pub fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            content_type: "text/plain",
            body: vec![],
            extra_headers: vec![("Location", location.to_string())],
        }
    }

    pub fn not_found() -> Self {
        Self { status: 404, content_type: "text/plain", body: b"Not Found".to_vec(), extra_headers: vec![] }
    }

    pub fn unauthorized() -> Self {
        Self::json(401, r#"{"error":"unauthorized"}"#)
    }

    pub fn bad_request(msg: &str) -> Self {
        Self::json(400, &format!(r#"{{"error":"{msg}"}}"#))
    }

    pub fn error(msg: &str) -> Self {
        Self::json(500, &format!(r#"{{"error":"{msg}"}}"#))
    }

    /// Chain a Set-Cookie header onto any response.
    pub fn with_set_cookie(mut self, value: String) -> Self {
        self.extra_headers.push(("Set-Cookie", value));
        self
    }
}

pub const KNOWN_ARCHES: &[&str] = &["aarch64-linux-gnu", "x86_64-linux-gnu"];

fn fmt_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", bytes / 1024)
    }
}

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
                return root_index(headers, state);
            }
            if path == "/health" {
                return Response::json(200, r#"{"status":"ok"}"#);
            }
            if path == "/auth" || path.starts_with("/auth?") {
                return crate::webauthn_handlers::auth_page();
            }
            if path.starts_with("/auth/poll") {
                let query = path.find('?').map(|i| &path[i + 1..]).unwrap_or("");
                return crate::webauthn_handlers::poll_session(query, state);
            }
            if path == "/auth/logout" {
                return crate::webauthn_handlers::logout(&headers, &state);
            }
            if path == "/auth/settings" {
                return crate::webauthn_handlers::settings_page(&headers, &state);
            }
            if let Some(arch) = path.strip_prefix('/').and_then(|p| p.strip_suffix("/packages.json")) {
                return get_index(arch, state);
            }
            // /src/{pkg}/{ver}-{rel}.tar.bz2  — processed source tarballs (arch-independent)
            // /{arch}/{pkg}/{ver}-{rel}.tar.gz — binary tarballs
            let parts: Vec<&str> = path.trim_start_matches('/').splitn(3, '/').collect();
            if parts.len() == 3 {
                if parts[0] == "src" {
                    return get_source_tarball(parts[1], parts[2], state);
                }
                return get_tarball(&parts[0], &parts[1], parts[2], state);
            }
            Response::not_found()
        }
        "POST" => match path {
            "/auth/register/options" => crate::webauthn_handlers::register_options(body, state),
            "/auth/register/verify" => crate::webauthn_handlers::register_verify(body, state),
            "/auth/authenticate/options" => crate::webauthn_handlers::authenticate_options(state),
            "/auth/authenticate/verify" => crate::webauthn_handlers::authenticate_verify(body, state),
            "/auth/tokens" => crate::webauthn_handlers::create_token_api(body, headers, state),
            "/auth/tokens/delete" => crate::webauthn_handlers::delete_token_api(body, headers, state),
            "/api/upload" => upload(headers, body, state),
            "/api/upload-src" => upload_src(headers, body, state),
            "/api/backfill-sizes" => backfill_sizes(headers, state),
            "/api/upload-url" => upload_url(headers, state),
            "/api/update-index" => update_index_endpoint(headers, state),
            "/api/publish" => publish(headers, body, state),
            "/api/reindex" => reindex(headers, body, state),
            "/api/delete" => delete_pkg(headers, body, state),
            _ => Response::not_found(),
        },
        _ => Response::not_found(),
    }
}

pub(crate) fn session_cookie(headers: &HashMap<String, String>) -> Option<String> {
    headers.get("cookie").and_then(|h| {
        h.split(';').find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            if k.trim() == "kominka_session" { Some(v.trim().to_string()) } else { None }
        })
    })
}

fn root_index(headers: &HashMap<String, String>, state: &AppState) -> Response {
    let username = session_cookie(headers)
        .and_then(|t| state.db.lock().unwrap().verify_browser_session(&t).ok().flatten());

    let userbar = match &username {
        Some(name) => format!(
            "<span class=userbar>{name} <a href=\"/auth/settings\">(settings)</a> \u{00b7} <a href=\"/auth/logout\">sign out</a></span>"
        ),
        None => "<a href=\"/auth\" class=signin>sign in</a>".to_string(),
    };

    let indexes = state.indexes.read().unwrap();

    // Collect all package names across all arches, sorted.
    let mut all_names: Vec<&str> = {
        let mut set = std::collections::BTreeSet::new();
        for arch in KNOWN_ARCHES {
            if let Some(idx) = indexes.get(*arch) {
                for name in idx.packages.keys() {
                    set.insert(name.as_str());
                }
            }
        }
        set.into_iter().collect()
    };
    // BTreeSet is already sorted; collect preserves order.
    let _ = &mut all_names; // suppress unused-mut

    let mut html = format!(
        "<!doctype html>\
        <html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
        <title>Kominka Packages</title><style>\
        *{{margin:0;padding:0;box-sizing:border-box}}\
        body{{font-family:system-ui,sans-serif;max-width:960px;margin:0 auto;padding:2rem 1rem;\
        color:#e0e0e0;background:#1a1a1a}}\
        header{{display:flex;justify-content:space-between;align-items:baseline;margin-bottom:1.5rem}}\
        h1{{font-size:1.4rem;color:#fff}}\
        .signin{{font-size:.8rem;color:#666}}\
        .userbar{{font-size:.8rem;color:#888}}.userbar a{{color:#888;text-decoration:none}}.userbar a:hover{{text-decoration:underline}}\
        .count{{font-size:.85rem;color:#666;margin-bottom:.75rem}}\
        table{{width:100%;border-collapse:collapse;font-size:.85rem}}\
        th{{text-align:left;padding:.3rem .5rem;border-bottom:1px solid #333;color:#888;font-weight:normal}}\
        td{{padding:.4rem .5rem;border-bottom:1px solid #222;vertical-align:top}}\
        a{{color:#6ba3f7;text-decoration:none}}a:hover{{text-decoration:underline}}\
        .dep{{color:#888}}.mkdep{{color:#666}}\
        .arch-row{{display:block;font-size:.8rem;color:#999;margin-top:.15rem}}\
        .arch-row a{{color:#aac8f7}}\
        .empty{{color:#666;padding:2rem 0}}\
        @media(prefers-color-scheme:light){{body{{color:#222;background:#fff}}\
        h1{{color:#000}}th{{color:#666;border-color:#ddd}}\
        td{{border-color:#eee}}a{{color:#1a6be0}}.dep{{color:#555}}.mkdep{{color:#888}}\
        .signin{{color:#999}}.userbar,.userbar a{{color:#aaa}}\
        .arch-row{{color:#666}}.arch-row a{{color:#1a6be0}}.count{{color:#999}}}}\
        </style></head><body>\
        <header><h1>Kominka Packages</h1>{userbar}</header>",
    );

    if all_names.is_empty() {
        html.push_str("<p class=empty>No packages indexed yet.</p>");
    } else {
        html.push_str(&format!(
            "<p class=count>{} packages</p>\
            <table><tr><th>Package</th><th>Source</th><th>Dependencies</th><th>Build deps</th></tr>",
            all_names.len()
        ));

        for name in &all_names {
            // Entries per arch, in KNOWN_ARCHES order.
            let arch_entries: Vec<(&str, &PackageEntry)> = KNOWN_ARCHES.iter()
                .filter_map(|arch| indexes.get(*arch)?.packages.get(*name).map(|e| (*arch, e)))
                .collect();

            if arch_entries.is_empty() { continue; }

            // Package cell: name then one line per arch.
            let mut pkg_cell = format!("<strong>{name}</strong>");
            for (arch, e) in &arch_entries {
                let short = arch.strip_suffix("-linux-gnu").unwrap_or(arch);
                let tarball_url = format!("/{arch}/{name}/{}-{}.tar.gz", e.ver, e.rel);
                let size_str = e.size.map(|sz| format!(" <span class=dep>{}</span>", fmt_size(sz))).unwrap_or_default();
                pkg_cell.push_str(&format!(
                    "<span class=arch-row><a href=\"{tarball_url}\">{}-{} {short}</a>{size_str}</span>",
                    e.ver, e.rel
                ));
            }

            // Source cell: arch-independent — use first entry that has src_sha256.
            let src_cell = match arch_entries.iter().find(|(_, e)| e.src_sha256.is_some()) {
                Some((_, e)) => {
                    let src_url = format!("/src/{name}/{}-{}.tar.bz2", e.ver, e.rel);
                    match e.src_size {
                        Some(sz) => format!("<a href=\"{src_url}\">download</a> <span class=dep>({})</span>", fmt_size(sz)),
                        None     => format!("<a href=\"{src_url}\">download</a>"),
                    }
                }
                None => "<span class=dep>\u{2014}</span>".to_string(),
            };

            // Deps — use first entry (same across arches for well-formed packages).
            let (_, first) = arch_entries[0];
            let deps_html = if first.deps.is_empty() {
                "<span class=dep>\u{2014}</span>".to_string()
            } else {
                let links: Vec<String> = first.deps.iter().map(|d| {
                    if all_names.iter().any(|n| *n == d.as_str()) {
                        format!("<a href=\"#\">{d}</a>")
                    } else {
                        d.clone()
                    }
                }).collect();
                format!("<span class=dep>{}</span>", links.join(", "))
            };
            let mkdeps_html = if first.mkdeps.is_empty() {
                "<span class=mkdep>\u{2014}</span>".to_string()
            } else {
                let links: Vec<String> = first.mkdeps.iter().map(|d| {
                    if all_names.iter().any(|n| *n == d.as_str()) {
                        format!("<a href=\"#\">{d}</a>")
                    } else {
                        d.clone()
                    }
                }).collect();
                format!("<span class=mkdep>{}</span>", links.join(", "))
            };

            html.push_str(&format!(
                "<tr><td>{pkg_cell}</td><td>{src_cell}</td><td>{deps_html}</td><td>{mkdeps_html}</td></tr>"
            ));
        }

        html.push_str("</table>");
    }

    html.push_str("</body></html>");
    Response::html(html.into_bytes())
}

fn get_index(arch: &str, state: &AppState) -> Response {
    if !KNOWN_ARCHES.contains(&arch) {
        return Response::not_found();
    }
    let indexes = state.indexes.read().unwrap();
    match indexes.get(arch) {
        Some(idx) => {
            let json = serde_json::to_vec_pretty(idx).unwrap();
            Response::json_bytes(200, json)
        }
        None => Response::not_found(),
    }
}

fn get_tarball(arch: &str, pkg: &str, file: &str, state: &AppState) -> Response {
    if !KNOWN_ARCHES.contains(&arch) || !valid_pkg_name(pkg) {
        return Response::not_found();
    }
    if !file.ends_with(".tar.gz") || file.contains("..") || file.contains('/') {
        return Response::not_found();
    }
    // Redirect to R2's public URL when configured — avoids proxying bytes through
    // the server (double bandwidth + full-file RAM buffer per download).
    if let Some(base) = &state.r2_public_url {
        return Response::redirect(&format!("{base}/{arch}/{pkg}/{file}"));
    }
    let key = format!("{arch}/{pkg}/{file}");
    match state.s3.get(&key) {
        Some(bytes) => Response::octet(bytes),
        None => Response::not_found(),
    }
}

fn upload(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::authenticated(headers, state) {
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
    let _upload_guard = state.upload_lock.lock().unwrap();
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
        size: Some(body.len() as u64),
        src_sha256: None,
        src_size: None,
    };
    if let Err(e) = update_index(state, arch, pkg, entry) {
        tracing::error!("index update failed: {e}");
        return Response::error("index update failed");
    }

    tracing::info!("uploaded {key}");
    Response::json_bytes(201, format!(r#"{{"ok":true,"sha256":"{sha256_hex}"}}"#).into_bytes())
}

/// Return a presigned R2 PUT URL so large uploads bypass the Cloudflare proxy
/// (which has a 100MB request body limit). The caller PUTs the tarball directly
/// to R2, then calls /api/update-index with the sha256 to register the package.
fn upload_url(headers: &HashMap<String, String>, state: &AppState) -> Response {
    if !auth::authenticated(headers, state) {
        return Response::unauthorized();
    }

    let arch = headers.get("x-arch").map(|s| s.as_str()).unwrap_or("");
    let pkg = headers.get("x-pkg").map(|s| s.as_str()).unwrap_or("");
    let ver = headers.get("x-ver").map(|s| s.as_str()).unwrap_or("");
    let rel = headers.get("x-rel").map(|s| s.as_str()).unwrap_or("");

    if arch.is_empty() || pkg.is_empty() || ver.is_empty() || rel.is_empty() {
        return Response::bad_request("missing headers");
    }
    if !KNOWN_ARCHES.contains(&arch) {
        return Response::bad_request("unknown arch");
    }
    if !valid_pkg_name(pkg) {
        return Response::bad_request("invalid package name");
    }

    let key = format!("{arch}/{pkg}/{ver}-{rel}.tar.gz");
    match state.s3.presign_put(&key, 3600) {
        Some(url) => Response::json_bytes(200, format!(r#"{{"url":"{url}"}}"#).into_bytes()),
        None => Response::error("presigned URLs not supported for this storage backend"),
    }
}

/// Register a package in the index after a presigned direct-to-R2 upload.
/// Caller must provide X-Sha256 header with the hex sha256 of the uploaded file.
fn update_index_endpoint(headers: &HashMap<String, String>, state: &AppState) -> Response {
    if !auth::authenticated(headers, state) {
        return Response::unauthorized();
    }

    let arch = headers.get("x-arch").map(|s| s.as_str()).unwrap_or("");
    let pkg = headers.get("x-pkg").map(|s| s.as_str()).unwrap_or("");
    let ver = headers.get("x-ver").map(|s| s.as_str()).unwrap_or("");
    let rel = headers.get("x-rel").map(|s| s.as_str()).unwrap_or("");
    let sha256 = headers.get("x-sha256").map(|s| s.as_str()).unwrap_or("");
    let deps_raw = headers.get("x-deps").map(|s| s.as_str()).unwrap_or("");
    let mkdeps_raw = headers.get("x-mkdeps").map(|s| s.as_str()).unwrap_or("");
    let size: Option<u64> = headers.get("x-size").and_then(|s| s.parse().ok());

    if arch.is_empty() || pkg.is_empty() || ver.is_empty() || rel.is_empty() || sha256.is_empty() {
        return Response::bad_request("missing headers");
    }
    if !KNOWN_ARCHES.contains(&arch) {
        return Response::bad_request("unknown arch");
    }
    if !valid_pkg_name(pkg) {
        return Response::bad_request("invalid package name");
    }
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Response::bad_request("invalid sha256");
    }

    let parse_list = |raw: &str| -> Vec<String> {
        if raw.is_empty() { vec![] }
        else { raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() }
    };

    let entry = PackageEntry {
        ver: ver.to_string(),
        rel: rel.to_string(),
        deps: parse_list(deps_raw),
        mkdeps: parse_list(mkdeps_raw),
        sha256: sha256.to_string(),
        size,
        src_sha256: None,
        src_size: None,
    };
    if let Err(e) = update_index(state, arch, pkg, entry) {
        tracing::error!("index update failed: {e}");
        return Response::error("index update failed");
    }

    let key = format!("{arch}/{pkg}/{ver}-{rel}.tar.gz");
    tracing::info!("registered {key} sha256={sha256}");
    Response::json_bytes(201, br#"{"ok":true}"#.to_vec())
}

fn publish(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::authenticated(headers, state) {
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
        size: None,
        src_sha256: None,
        src_size: None,
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
    if !auth::authenticated(headers, state) {
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
    let (sha256, size) = match tarball {
        Some(bytes) => (s3::sha256_hex(&bytes), Some(bytes.len() as u64)),
        None => return Response::bad_request("tarball not found in R2"),
    };

    let entry = PackageEntry {
        ver: req.ver,
        rel: req.rel,
        deps: req.deps,
        mkdeps: req.mkdeps,
        sha256,
        size,
        src_sha256: None,
        src_size: None,
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
    if !auth::authenticated(headers, state) {
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

fn get_source_tarball(pkg: &str, file: &str, state: &AppState) -> Response {
    if !valid_pkg_name(pkg) {
        return Response::not_found();
    }
    if !file.ends_with(".tar.bz2") || file.contains("..") || file.contains('/') {
        return Response::not_found();
    }
    if let Some(base) = &state.r2_public_url {
        return Response::redirect(&format!("{base}/src/{pkg}/{file}"));
    }
    let key = format!("src/{pkg}/{file}");
    match state.s3.get(&key) {
        Some(bytes) => Response::octet(bytes),
        None => Response::not_found(),
    }
}

fn upload_src(headers: &HashMap<String, String>, body: &[u8], state: &AppState) -> Response {
    if !auth::authenticated(headers, state) {
        return Response::unauthorized();
    }

    let pkg = headers.get("x-pkg").map(|s| s.as_str()).unwrap_or("");
    let ver = headers.get("x-ver").map(|s| s.as_str()).unwrap_or("");
    let rel = headers.get("x-rel").map(|s| s.as_str()).unwrap_or("");

    if pkg.is_empty() || ver.is_empty() || rel.is_empty() {
        return Response::bad_request("missing headers");
    }
    if !valid_pkg_name(pkg) {
        return Response::bad_request("invalid package name");
    }

    let sha256_hex = s3::sha256_hex(body);
    let key = format!("src/{pkg}/{ver}-{rel}.tar.bz2");

    let _upload_guard = state.upload_lock.lock().unwrap();
    if let Err(e) = state.s3.put(&key, body.to_vec(), "application/octet-stream") {
        tracing::error!("source upload failed: {e}");
        return Response::error("source upload failed");
    }

    let src_size = body.len() as u64;

    // Update src_sha256/src_size in the package index for all architectures (sources are arch-independent).
    let mut indexed = false;
    let mut indexes = state.indexes.write().unwrap();
    for arch in KNOWN_ARCHES {
        if let Some(idx) = indexes.get_mut(*arch) {
            if let Some(entry) = idx.packages.get_mut(pkg) {
                if entry.ver == ver && entry.rel == rel {
                    entry.src_sha256 = Some(sha256_hex.clone());
                    entry.src_size = Some(src_size);
                    if let Err(e) = save_index(&state.s3, arch, idx) {
                        tracing::error!("upload-src: save index {arch}: {e}");
                    } else {
                        indexed = true;
                    }
                } else {
                    tracing::warn!(
                        "upload-src: {pkg} in {arch} index is at {}-{}, got {ver}-{rel} — index not updated",
                        entry.ver, entry.rel
                    );
                }
            }
        }
    }

    if !indexed {
        tracing::warn!("uploaded source {key} but found no matching index entry to stamp");
    } else {
        tracing::info!("uploaded source {key} sha256={sha256_hex}");
    }
    Response::json_bytes(201, format!(r#"{{"ok":true,"indexed":{indexed}}}"#).into_bytes())
}

/// POST /api/backfill-sizes — fill in missing `size`/`src_size` for all index
/// entries by issuing S3 HEAD requests. Safe to call multiple times; skips
/// entries that already have a value.
fn backfill_sizes(headers: &HashMap<String, String>, state: &AppState) -> Response {
    if !auth::authenticated(headers, state) {
        return Response::unauthorized();
    }

    let mut updated = 0u32;
    let mut indexes = state.indexes.write().unwrap();

    for arch in KNOWN_ARCHES {
        let Some(idx) = indexes.get_mut(*arch) else { continue };
        let mut dirty = false;

        for (name, entry) in idx.packages.iter_mut() {
            if entry.size.is_none() && !entry.sha256.is_empty() {
                let key = format!("{arch}/{name}/{}-{}.tar.gz", entry.ver, entry.rel);
                if let Some(sz) = state.s3.object_size(&key) {
                    entry.size = Some(sz);
                    dirty = true;
                    updated += 1;
                }
            }
            // If src_sha256 is missing entirely, try to fetch the source tarball
            // from S3 (it may have been uploaded before the index was ready).
            if entry.src_sha256.is_none() {
                let key = format!("src/{name}/{}-{}.tar.bz2", entry.ver, entry.rel);
                if let Some(bytes) = state.s3.get(&key) {
                    entry.src_sha256 = Some(s3::sha256_hex(&bytes));
                    entry.src_size = Some(bytes.len() as u64);
                    dirty = true;
                    updated += 1;
                }
            } else if entry.src_size.is_none() {
                let key = format!("src/{name}/{}-{}.tar.bz2", entry.ver, entry.rel);
                if let Some(sz) = state.s3.object_size(&key) {
                    entry.src_size = Some(sz);
                    dirty = true;
                    updated += 1;
                }
            }
        }

        if dirty {
            if let Err(e) = save_index(&state.s3, arch, idx) {
                tracing::error!("backfill-sizes: save index {arch}: {e}");
            }
        }
    }

    tracing::info!("backfill-sizes: updated {updated} entries");
    Response::json_bytes(200, format!(r#"{{"ok":true,"updated":{updated}}}"#).into_bytes())
}

fn update_index(state: &AppState, arch: &str, pkg: &str, entry: PackageEntry) -> Result<(), String> {
    let mut indexes = state.indexes.write().unwrap();
    let index = indexes.entry(arch.to_string()).or_insert_with(PackageIndex::new);
    index.packages.insert(pkg.to_string(), entry);
    save_index(&state.s3, arch, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Mutex, RwLock};

    fn make_state() -> AppState {
        AppState {
            s3: s3::Storage::memory(),
            db: Mutex::new(crate::db::Db::open(":memory:").unwrap()),
            webauthn: crate::webauthn::RelyingParty::new("localhost", "http://localhost", "Test"),
            jwks: None,
            allowed_users: vec![],
            indexes: RwLock::new(HashMap::new()),
            upload_lock: Mutex::new(()),
            secure_cookies: false,
            r2_public_url: None,
        }
    }

    fn auth_headers(state: &AppState) -> HashMap<String, String> {
        let db = state.db.lock().unwrap();
        let _ = db.create_user("uid", "testuser"); // ignore if already exists
        let token = db.create_token("uid", "cli", None).unwrap();
        drop(db);
        let mut h = HashMap::new();
        h.insert("authorization".into(), format!("Bearer {token}"));
        h
    }

    fn upload_pkg(state: &AppState, arch: &str, pkg: &str, body: &[u8]) -> Response {
        let mut h = auth_headers(state);
        h.insert("x-arch".into(), arch.into());
        h.insert("x-pkg".into(), pkg.into());
        h.insert("x-ver".into(), "1.0".into());
        h.insert("x-rel".into(), "1".into());
        route("POST", "/api/upload", &h, body, state)
    }

    fn upload_src(state: &AppState, pkg: &str, body: &[u8]) -> Response {
        let mut h = auth_headers(state);
        h.insert("x-pkg".into(), pkg.into());
        h.insert("x-ver".into(), "1.0".into());
        h.insert("x-rel".into(), "1".into());
        route("POST", "/api/upload-src", &h, body, state)
    }

    #[test]
    fn fmt_size_kb() {
        assert_eq!(fmt_size(1024), "1 KB");
        assert_eq!(fmt_size(512), "0 KB");
        assert_eq!(fmt_size(793_600), "775 KB");
    }

    #[test]
    fn fmt_size_mb() {
        assert_eq!(fmt_size(1_048_576), "1.0 MB");
        assert_eq!(fmt_size(15_728_640), "15.0 MB");
    }

    #[test]
    fn upload_sets_size_in_index() {
        let state = make_state();
        let body = b"fake tarball data";
        let resp = upload_pkg(&state, "x86_64-linux-gnu", "zlib", body);
        assert_eq!(resp.status, 201);

        let indexes = state.indexes.read().unwrap();
        let entry = &indexes["x86_64-linux-gnu"].packages["zlib"];
        assert_eq!(entry.size, Some(body.len() as u64));
        assert!(entry.src_sha256.is_none());
        assert!(entry.src_size.is_none());
    }

    #[test]
    fn upload_src_sets_src_sha256_and_src_size() {
        let state = make_state();
        upload_pkg(&state, "x86_64-linux-gnu", "zlib", b"bin");

        let src_body = b"source tarball bytes";
        let resp = upload_src(&state, "zlib", src_body);
        assert_eq!(resp.status, 201);

        let indexes = state.indexes.read().unwrap();
        let entry = &indexes["x86_64-linux-gnu"].packages["zlib"];
        assert_eq!(entry.src_size, Some(src_body.len() as u64));
        assert!(entry.src_sha256.is_some());
    }

    #[test]
    fn upload_src_updates_all_arches() {
        let state = make_state();
        upload_pkg(&state, "x86_64-linux-gnu", "zlib", b"bin-x86");
        upload_pkg(&state, "aarch64-linux-gnu", "zlib", b"bin-arm");

        upload_src(&state, "zlib", b"shared source");

        let indexes = state.indexes.read().unwrap();
        for arch in KNOWN_ARCHES {
            let entry = &indexes[*arch].packages["zlib"];
            assert!(entry.src_sha256.is_some(), "{arch} missing src_sha256");
            assert_eq!(entry.src_size, Some(13), "{arch} wrong src_size");
        }
    }

    #[test]
    fn upload_src_wrong_ver_is_ignored() {
        let state = make_state();
        upload_pkg(&state, "x86_64-linux-gnu", "zlib", b"bin");

        // Upload source for a different version — should not update the index entry.
        let mut h = auth_headers(&state);
        h.insert("x-pkg".into(), "zlib".into());
        h.insert("x-ver".into(), "9.9".into());
        h.insert("x-rel".into(), "1".into());
        let resp = route("POST", "/api/upload-src", &h, b"src", &state);
        assert_eq!(resp.status, 201);

        let indexes = state.indexes.read().unwrap();
        let entry = &indexes["x86_64-linux-gnu"].packages["zlib"];
        assert!(entry.src_sha256.is_none());
    }

    #[test]
    fn update_index_endpoint_parses_x_size() {
        let state = make_state();
        // First upload the tarball so S3 has it (update-index does not require the body,
        // but the endpoint itself needs auth and valid headers).
        let body = b"tarball contents for presign path";
        let key = "x86_64-linux-gnu/mylib/2.0-3.tar.gz";
        state.s3.put(key, body.to_vec(), "application/octet-stream").unwrap();
        let sha256 = s3::sha256_hex(body);

        let mut h = auth_headers(&state);
        h.insert("x-arch".into(), "x86_64-linux-gnu".into());
        h.insert("x-pkg".into(), "mylib".into());
        h.insert("x-ver".into(), "2.0".into());
        h.insert("x-rel".into(), "3".into());
        h.insert("x-sha256".into(), sha256);
        h.insert("x-size".into(), body.len().to_string());

        let resp = route("POST", "/api/update-index", &h, b"", &state);
        assert_eq!(resp.status, 201);

        let indexes = state.indexes.read().unwrap();
        let entry = &indexes["x86_64-linux-gnu"].packages["mylib"];
        assert_eq!(entry.size, Some(body.len() as u64));
    }

    #[test]
    fn get_source_tarball_roundtrip() {
        let state = make_state();
        upload_pkg(&state, "x86_64-linux-gnu", "zlib", b"bin");
        let src = b"source data";
        upload_src(&state, "zlib", src);

        let resp = route("GET", "/src/zlib/1.0-1.tar.bz2", &HashMap::new(), b"", &state);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, src);
    }

    #[test]
    fn root_index_shows_size_and_src_link() {
        let state = make_state();
        upload_pkg(&state, "x86_64-linux-gnu", "zlib", b"bin");
        upload_src(&state, "zlib", b"src");

        let resp = route("GET", "/", &HashMap::new(), b"", &state);
        assert_eq!(resp.status, 200);
        let html = String::from_utf8(resp.body).unwrap();
        assert!(html.contains("/src/zlib/1.0-1.tar.bz2"), "missing src link");
        assert!(html.contains(">download<"), "missing src anchor text");
        assert!(html.contains("0 KB"), "missing size label");
    }

    #[test]
    fn upload_requires_auth() {
        let state = make_state();
        let resp = route("POST", "/api/upload", &HashMap::new(), b"data", &state);
        assert_eq!(resp.status, 401);
    }

    #[test]
    fn upload_src_requires_auth() {
        let state = make_state();
        let resp = route("POST", "/api/upload-src", &HashMap::new(), b"data", &state);
        assert_eq!(resp.status, 401);
    }

    #[test]
    fn get_source_tarball_not_found() {
        let state = make_state();
        let resp = route("GET", "/src/missing/1.0-1.tar.bz2", &HashMap::new(), b"", &state);
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn get_source_tarball_rejects_bad_extension() {
        let state = make_state();
        let resp = route("GET", "/src/zlib/1.0-1.tar.gz", &HashMap::new(), b"", &state);
        assert_eq!(resp.status, 404);
    }
}
