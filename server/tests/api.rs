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

// Health

#[test]
fn health_returns_ok() {
    let state = test_state();
    let resp = packages::route("GET", "/health", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    let body = body_json(&resp);
    assert_eq!(body["status"], "ok");
}

// Auth

#[test]
fn upload_rejects_bad_auth() {
    let state = test_state();
    // No token.
    let resp = packages::route("POST", "/api/upload", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 401);
    // Wrong token.
    let h = headers(&[("authorization", "Bearer wrong-token")]);
    let resp = packages::route("POST", "/api/upload", &h, b"", &state);
    assert_eq!(resp.status, 401);
}

// Upload + Index round-trip

#[test]
fn upload_stores_tarball_and_updates_index() {
    let state = test_state();
    let tarball = b"fake tarball content";

    let h = auth_headers(&[
        ("x-arch", "aarch64-linux-gnu"),
        ("x-pkg", "curl"),
        ("x-ver", "8.19.0"),
        ("x-rel", "6"),
        ("x-hash", "abc123"),
        ("x-deps", "boringssl,zlib"),
    ]);
    let resp = packages::route("POST", "/api/upload", &h, tarball, &state);
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
    assert_eq!(idx["packages"]["curl"]["hash"], "abc123");
    let deps = idx["packages"]["curl"]["deps"].as_array().unwrap();
    assert_eq!(deps.len(), 2);
    assert_eq!(deps[0], "boringssl");
    assert_eq!(deps[1], "zlib");

    // Verify tarball download.
    let resp = packages::route("GET", "/aarch64-linux-gnu/curl/abc123.tar.gz", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, tarball);
}

#[test]
fn upload_with_no_deps() {
    let state = test_state();
    let h = auth_headers(&[
        ("x-arch", "x86_64-linux-gnu"),
        ("x-pkg", "zlib"),
        ("x-ver", "1.3.1"),
        ("x-rel", "1"),
        ("x-hash", "def456"),
        ("x-deps", ""),
    ]);
    let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    let deps = idx["packages"]["zlib"]["deps"].as_array().unwrap();
    assert!(deps.is_empty());
}

// Upload validation

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
    let h = auth_headers(&[
        ("x-arch", "mips-unknown-linux"),
        ("x-pkg", "curl"),
        ("x-ver", "1.0"),
        ("x-rel", "1"),
        ("x-hash", "abc"),
    ]);
    let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
    assert_eq!(resp.status, 400);
    assert_eq!(body_json(&resp)["error"], "unknown arch");
}

#[test]
fn upload_rejects_invalid_pkg_name() {
    let state = test_state();
    for bad_name in ["UPPER", "../etc/passwd", "", "-leading-dash"] {
        let h = auth_headers(&[
            ("x-arch", "aarch64-linux-gnu"),
            ("x-pkg", bad_name),
            ("x-ver", "1.0"),
            ("x-rel", "1"),
            ("x-hash", "abc"),
        ]);
        let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
        assert_eq!(resp.status, 400, "expected 400 for pkg name '{bad_name}'");
    }
}

// Publish (metapackages)

#[test]
fn publish_registers_metapackage() {
    let state = test_state();
    let body = br#"{"arch":"aarch64-linux-gnu","pkg":"core","ver":"1.0","rel":"1","hash":"meta123","deps":["glibc","busybox"]}"#;
    let h = auth_headers(&[]);
    let resp = packages::route("POST", "/api/publish", &h, body, &state);
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"]["core"]["ver"], "1.0");
    assert_eq!(idx["packages"]["core"]["sha256"], "");
    let deps = idx["packages"]["core"]["deps"].as_array().unwrap();
    assert_eq!(deps.len(), 2);
}

#[test]
fn publish_rejects_unknown_arch() {
    let state = test_state();
    let body = br#"{"arch":"bad-arch","pkg":"test","ver":"1.0","rel":"1","hash":"abc","deps":[]}"#;
    let h = auth_headers(&[]);
    let resp = packages::route("POST", "/api/publish", &h, body, &state);
    assert_eq!(resp.status, 400);
}

// Arch isolation

#[test]
fn indexes_are_per_arch() {
    let state = test_state();
    let h = auth_headers(&[
        ("x-arch", "aarch64-linux-gnu"),
        ("x-pkg", "curl"),
        ("x-ver", "1.0"),
        ("x-rel", "1"),
        ("x-hash", "aaa"),
    ]);
    let resp = packages::route("POST", "/api/upload", &h, b"arm-data", &state);
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 404);
}

// Multiple uploads

#[test]
fn multiple_uploads_accumulate_in_index() {
    let state = test_state();
    for (pkg, hash) in [("curl", "h1"), ("zlib", "h2"), ("boringssl", "h3")] {
        let h = auth_headers(&[
            ("x-arch", "aarch64-linux-gnu"),
            ("x-pkg", pkg),
            ("x-ver", "1.0"),
            ("x-rel", "1"),
            ("x-hash", hash),
        ]);
        let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
        assert_eq!(resp.status, 201);
    }

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"].as_object().unwrap().len(), 3);
    assert_eq!(idx["packages"]["curl"]["hash"], "h1");
    assert_eq!(idx["packages"]["zlib"]["hash"], "h2");
    assert_eq!(idx["packages"]["boringssl"]["hash"], "h3");
}

// Upload overwrite

#[test]
fn upload_overwrites_package_in_index() {
    let state = test_state();

    let h = auth_headers(&[
        ("x-arch", "aarch64-linux-gnu"),
        ("x-pkg", "curl"),
        ("x-ver", "1.0"),
        ("x-rel", "1"),
        ("x-hash", "old"),
    ]);
    packages::route("POST", "/api/upload", &h, b"v1", &state);

    let h = auth_headers(&[
        ("x-arch", "aarch64-linux-gnu"),
        ("x-pkg", "curl"),
        ("x-ver", "2.0"),
        ("x-rel", "1"),
        ("x-hash", "new"),
    ]);
    packages::route("POST", "/api/upload", &h, b"v2", &state);

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"].as_object().unwrap().len(), 1);
    assert_eq!(idx["packages"]["curl"]["ver"], "2.0");
    assert_eq!(idx["packages"]["curl"]["hash"], "new");
}

// SHA-256 correctness

#[test]
fn upload_sha256_is_correct() {
    use sha2::{Digest, Sha256};

    let state = test_state();
    let data = b"deterministic test content";

    let expected = {
        let mut h = Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    };

    let h = auth_headers(&[
        ("x-arch", "aarch64-linux-gnu"),
        ("x-pkg", "test"),
        ("x-ver", "1.0"),
        ("x-rel", "1"),
        ("x-hash", "testhash"),
    ]);
    let resp = packages::route("POST", "/api/upload", &h, data, &state);
    assert_eq!(body_json(&resp)["sha256"], expected);
}

// Valid package names

#[test]
fn valid_package_names_accepted() {
    let state = test_state();
    for name in ["curl", "ca-certificates", "linux-headers", "e2fsprogs", "sudo-rs", "zlib1g"] {
        let h = auth_headers(&[
            ("x-arch", "aarch64-linux-gnu"),
            ("x-pkg", name),
            ("x-ver", "1.0"),
            ("x-rel", "1"),
            ("x-hash", "h"),
        ]);
        let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
        assert_eq!(resp.status, 201, "expected 201 for pkg name '{name}'");
    }
}

