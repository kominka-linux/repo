use std::collections::HashMap;
use std::sync::RwLock;

use kominka_repo::{AppState, packages, s3};

const API_KEY: &str = "test-secret-key-for-unit-tests";

fn test_state() -> AppState {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(API_KEY.as_bytes());
    AppState {
        s3: s3::Storage::memory(),
        api_key_hash: h.finalize().into(),
        indexes: RwLock::new(HashMap::new()),
    }
}

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn auth_headers(extra: &[(&str, &str)]) -> HashMap<String, String> {
    let mut h = headers(&[("authorization", &format!("Bearer {API_KEY}"))]);
    for (k, v) in extra {
        h.insert(k.to_string(), v.to_string());
    }
    h
}

fn body_json(resp: &packages::Response) -> serde_json::Value {
    serde_json::from_slice(&resp.body).unwrap()
}

fn upload_pkg(state: &AppState, arch: &str, pkg: &str, ver: &str, rel: &str, deps: &str, body: &[u8]) -> packages::Response {
    let h = auth_headers(&[
        ("x-arch", arch), ("x-pkg", pkg), ("x-ver", ver), ("x-rel", rel), ("x-deps", deps),
    ]);
    packages::route("POST", "/api/upload", &h, body, state)
}

#[test]
fn health_returns_ok() {
    let state = test_state();
    let resp = packages::route("GET", "/health", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(body_json(&resp)["status"], "ok");
}

#[test]
fn upload_rejects_bad_auth() {
    let state = test_state();
    let resp = packages::route("POST", "/api/upload", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 401);
    let h = headers(&[("authorization", "Bearer wrong-token")]);
    let resp = packages::route("POST", "/api/upload", &h, b"", &state);
    assert_eq!(resp.status, 401);
}

#[test]
fn upload_stores_tarball_and_updates_index() {
    let state = test_state();
    let tarball = b"fake tarball content";

    let resp = upload_pkg(&state, "aarch64-linux-gnu", "curl", "8.19.0", "6", "boringssl,zlib", tarball);
    assert_eq!(resp.status, 201);
    let body = body_json(&resp);
    assert_eq!(body["ok"], true);
    assert_eq!(body["sha256"].as_str().unwrap().len(), 64);

    // Verify index.
    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    let idx = body_json(&resp);
    assert_eq!(idx["_version"], 1);
    assert_eq!(idx["packages"]["curl"]["ver"], "8.19.0");
    assert_eq!(idx["packages"]["curl"]["rel"], "6");
    let deps = idx["packages"]["curl"]["deps"].as_array().unwrap();
    assert_eq!(deps, &["boringssl", "zlib"]);

    // Verify tarball download at {ver}-{rel} path.
    let resp = packages::route("GET", "/aarch64-linux-gnu/curl/8.19.0-6.tar.gz", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, tarball);
}

#[test]
fn upload_with_no_deps() {
    let state = test_state();
    let resp = upload_pkg(&state, "x86_64-linux-gnu", "zlib", "1.3.1", "1", "", b"data");
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert!(idx["packages"]["zlib"]["deps"].as_array().unwrap().is_empty());
}

#[test]
fn upload_rejects_missing_headers() {
    let state = test_state();
    let h = auth_headers(&[("x-arch", "aarch64-linux-gnu")]);
    let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
    assert_eq!(resp.status, 400);
    assert_eq!(body_json(&resp)["error"], "missing headers");
}

#[test]
fn upload_rejects_unknown_arch() {
    let state = test_state();
    let resp = upload_pkg(&state, "mips-unknown-linux", "curl", "1.0", "1", "", b"data");
    assert_eq!(resp.status, 400);
    assert_eq!(body_json(&resp)["error"], "unknown arch");
}

#[test]
fn upload_rejects_invalid_pkg_name() {
    let state = test_state();
    for bad_name in ["UPPER", "../etc/passwd", "", "-leading-dash"] {
        let resp = upload_pkg(&state, "aarch64-linux-gnu", bad_name, "1.0", "1", "", b"data");
        assert_eq!(resp.status, 400, "expected 400 for pkg name '{bad_name}'");
    }
}

#[test]
fn publish_registers_metapackage() {
    let state = test_state();
    let body = br#"{"arch":"aarch64-linux-gnu","pkg":"core","ver":"1.0","rel":"1","deps":["glibc","busybox"]}"#;
    let h = auth_headers(&[]);
    let resp = packages::route("POST", "/api/publish", &h, body, &state);
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"]["core"]["ver"], "1.0");
    assert_eq!(idx["packages"]["core"]["sha256"], "");
    assert_eq!(idx["packages"]["core"]["deps"].as_array().unwrap().len(), 2);
}

#[test]
fn publish_rejects_unknown_arch() {
    let state = test_state();
    let body = br#"{"arch":"bad-arch","pkg":"test","ver":"1.0","rel":"1","deps":[]}"#;
    let h = auth_headers(&[]);
    let resp = packages::route("POST", "/api/publish", &h, body, &state);
    assert_eq!(resp.status, 400);
}

#[test]
fn indexes_are_per_arch() {
    let state = test_state();
    let resp = upload_pkg(&state, "aarch64-linux-gnu", "curl", "1.0", "1", "", b"arm-data");
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 404);
}

#[test]
fn multiple_uploads_accumulate_in_index() {
    let state = test_state();
    for pkg in ["curl", "zlib", "boringssl"] {
        let resp = upload_pkg(&state, "aarch64-linux-gnu", pkg, "1.0", "1", "", b"data");
        assert_eq!(resp.status, 201);
    }

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"].as_object().unwrap().len(), 3);
}

#[test]
fn upload_overwrites_package_in_index() {
    let state = test_state();
    upload_pkg(&state, "aarch64-linux-gnu", "curl", "1.0", "1", "", b"v1");
    upload_pkg(&state, "aarch64-linux-gnu", "curl", "2.0", "1", "", b"v2");

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"].as_object().unwrap().len(), 1);
    assert_eq!(idx["packages"]["curl"]["ver"], "2.0");
}

#[test]
fn upload_sha256_is_correct() {
    use sha2::{Digest, Sha256};
    let state = test_state();
    let data = b"deterministic test content";
    let expected = format!("{:x}", Sha256::digest(data));

    let resp = upload_pkg(&state, "aarch64-linux-gnu", "test", "1.0", "1", "", data);
    assert_eq!(body_json(&resp)["sha256"], expected);
}

// Regression: upload body must survive the round-trip intact.
// The S3 GET layer had a 10MB silent limit (read_to_vec) that truncated
// large tarballs to 0 bytes without error.
#[test]
fn upload_download_body_is_intact() {
    use sha2::{Digest, Sha256};

    let state = test_state();
    // Use a body large enough to exercise any size-related paths (1MB).
    let data: Vec<u8> = (0u8..=255).cycle().take(1024 * 1024).collect();
    let expected_sha = format!("{:x}", Sha256::digest(&data));

    let resp = upload_pkg(&state, "aarch64-linux-gnu", "bigpkg", "1.0", "1", "", &data);
    assert_eq!(resp.status, 201);
    assert_eq!(body_json(&resp)["sha256"], expected_sha);

    // Download and verify body is exactly what was uploaded.
    let resp = packages::route("GET", "/aarch64-linux-gnu/bigpkg/1.0-1.tar.gz", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.len(), data.len());
    assert_eq!(format!("{:x}", Sha256::digest(&resp.body)), expected_sha);
}

#[test]
fn valid_package_names_accepted() {
    let state = test_state();
    for name in ["curl", "ca-certificates", "linux", "e2fsprogs", "sudo-rs", "zlib1g"] {
        let resp = upload_pkg(&state, "aarch64-linux-gnu", name, "1.0", "1", "", b"data");
        assert_eq!(resp.status, 201, "expected 201 for pkg name '{name}'");
    }
}
