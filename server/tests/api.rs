use std::collections::HashMap;
use std::sync::RwLock;

use kominka_repo::{AppState, packages, s3};

const TEST_USER: &str = "testuser";
const TEST_TOKEN: &str = "aaaaaabbbbbbccccccddddddeeeeeeffffffffaaaaaaabbbbbbccccccddddddee";

fn make_state() -> AppState {
    let db = kominka_repo::db::Db::open(":memory:").expect("db");
    db.create_user("test-user-id", TEST_USER).expect("create user");
    db.seed_token("test-user-id", "test", TEST_TOKEN).expect("seed token");

    AppState {
        s3: s3::Storage::memory(),
        db: std::sync::Mutex::new(db),
        webauthn: kominka_repo::webauthn::RelyingParty::new(
            "test.example.com",
            "https://test.example.com",
            "Test",
        ),
        jwks: None,
        allowed_users: vec![TEST_USER.to_string()],
        indexes: RwLock::new(HashMap::new()),
        upload_lock: std::sync::Mutex::new(()),
        secure_cookies: false,
        r2_public_url: None,
    }
}

/// Creates a browser session and returns the cookie header string.
fn make_browser_session(state: &AppState) -> String {
    let session = state.db.lock().unwrap()
        .create_browser_session("test-user-id")
        .expect("browser session");
    format!("kominka_session={session}")
}

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn auth_headers(extra: &[(&str, &str)]) -> HashMap<String, String> {
    let mut h = headers(&[("authorization", &format!("Bearer {TEST_TOKEN}"))]);
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
    let state = make_state();
    let resp = packages::route("GET", "/health", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(body_json(&resp)["status"], "ok");
}

#[test]
fn upload_rejects_bad_auth() {
    let state = make_state();
    let resp = packages::route("POST", "/api/upload", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 401);
    let h = headers(&[("authorization", "Bearer wrong-token")]);
    let resp = packages::route("POST", "/api/upload", &h, b"", &state);
    assert_eq!(resp.status, 401);
}

#[test]
fn upload_stores_tarball_and_updates_index() {
    let state = make_state();
    let tarball = b"fake tarball content";

    let resp = upload_pkg(&state, "aarch64-linux-gnu", "curl", "8.19.0", "6", "boringssl,zlib", tarball);
    assert_eq!(resp.status, 201);
    let body = body_json(&resp);
    assert_eq!(body["ok"], true);
    assert_eq!(body["sha256"].as_str().unwrap().len(), 64);

    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    let idx = body_json(&resp);
    assert_eq!(idx["_version"], 1);
    assert_eq!(idx["packages"]["curl"]["ver"], "8.19.0");
    assert_eq!(idx["packages"]["curl"]["rel"], "6");
    let deps = idx["packages"]["curl"]["deps"].as_array().unwrap();
    assert_eq!(deps, &["boringssl", "zlib"]);

    let resp = packages::route("GET", "/aarch64-linux-gnu/curl/8.19.0-6.tar.gz", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, tarball);
}

#[test]
fn upload_with_no_deps() {
    let state = make_state();
    let resp = upload_pkg(&state, "x86_64-linux-gnu", "zlib", "1.3.1", "1", "", b"data");
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert!(idx["packages"]["zlib"]["deps"].as_array().unwrap().is_empty());
}

#[test]
fn upload_rejects_missing_headers() {
    let state = make_state();
    let h = auth_headers(&[("x-arch", "aarch64-linux-gnu")]);
    let resp = packages::route("POST", "/api/upload", &h, b"data", &state);
    assert_eq!(resp.status, 400);
    assert_eq!(body_json(&resp)["error"], "missing headers");
}

#[test]
fn upload_rejects_unknown_arch() {
    let state = make_state();
    let resp = upload_pkg(&state, "mips-unknown-linux", "curl", "1.0", "1", "", b"data");
    assert_eq!(resp.status, 400);
    assert_eq!(body_json(&resp)["error"], "unknown arch");
}

#[test]
fn upload_rejects_invalid_pkg_name() {
    let state = make_state();
    for bad_name in ["UPPER", "../etc/passwd", "", "-leading-dash"] {
        let resp = upload_pkg(&state, "aarch64-linux-gnu", bad_name, "1.0", "1", "", b"data");
        assert_eq!(resp.status, 400, "expected 400 for pkg name '{bad_name}'");
    }
}

#[test]
fn publish_registers_metapackage() {
    let state = make_state();
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
    let state = make_state();
    let body = br#"{"arch":"bad-arch","pkg":"test","ver":"1.0","rel":"1","deps":[]}"#;
    let h = auth_headers(&[]);
    let resp = packages::route("POST", "/api/publish", &h, body, &state);
    assert_eq!(resp.status, 400);
}

#[test]
fn indexes_are_per_arch() {
    let state = make_state();
    let resp = upload_pkg(&state, "aarch64-linux-gnu", "curl", "1.0", "1", "", b"arm-data");
    assert_eq!(resp.status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 404);
}

#[test]
fn multiple_uploads_accumulate_in_index() {
    let state = make_state();
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
    let state = make_state();
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
    let state = make_state();
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

    let state = make_state();
    let data: Vec<u8> = (0u8..=255).cycle().take(1024 * 1024).collect();
    let expected_sha = format!("{:x}", Sha256::digest(&data));

    let resp = upload_pkg(&state, "aarch64-linux-gnu", "bigpkg", "1.0", "1", "", &data);
    assert_eq!(resp.status, 201);
    assert_eq!(body_json(&resp)["sha256"], expected_sha);

    let resp = packages::route("GET", "/aarch64-linux-gnu/bigpkg/1.0-1.tar.gz", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body.len(), data.len());
    assert_eq!(format!("{:x}", Sha256::digest(&resp.body)), expected_sha);
}

#[test]
fn valid_package_names_accepted() {
    let state = make_state();
    for name in ["curl", "ca-certificates", "linux", "e2fsprogs", "sudo-rs", "zlib1g"] {
        let resp = upload_pkg(&state, "aarch64-linux-gnu", name, "1.0", "1", "", b"data");
        assert_eq!(resp.status, 201, "expected 201 for pkg name '{name}'");
    }
}

// ── R2 redirect tests ─────────────────────────────────────────────────────

#[test]
fn tarball_redirects_to_r2_when_configured() {
    let mut state = make_state();
    state.r2_public_url = Some("https://pub-abc.r2.dev".to_string());
    upload_pkg(&state, "aarch64-linux-gnu", "curl", "8.0.0", "1", "", b"data");

    let resp = packages::route("GET", "/aarch64-linux-gnu/curl/8.0.0-1.tar.gz", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 302);
    let loc = resp.extra_headers.iter().find(|(k, _)| *k == "Location").map(|(_, v)| v.as_str());
    assert_eq!(loc, Some("https://pub-abc.r2.dev/aarch64-linux-gnu/curl/8.0.0-1.tar.gz"));
}

#[test]
fn tarball_rejects_path_traversal() {
    let state = make_state();
    for bad in [
        "/aarch64-linux-gnu/curl/../../../etc/passwd",
        "/aarch64-linux-gnu/curl/../../secret.tar.gz",
    ] {
        let resp = packages::route("GET", bad, &headers(&[]), b"", &state);
        assert_eq!(resp.status, 404, "expected 404 for '{bad}'");
    }
}

// ── Auth / settings tests ──────────────────────────────────────────────────

#[test]
fn auth_page_returns_html() {
    let state = make_state();
    let resp = packages::route("GET", "/auth", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    assert!(resp.content_type.starts_with("text/html"));
}

#[test]
fn settings_page_redirects_without_session() {
    let state = make_state();
    let resp = packages::route("GET", "/auth/settings", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 302);
    let loc = resp.extra_headers.iter().find(|(k, _)| *k == "Location").map(|(_, v)| v.as_str());
    assert_eq!(loc, Some("/auth"));
}

#[test]
fn settings_page_accessible_with_browser_session() {
    let state = make_state();
    let cookie = make_browser_session(&state);
    let resp = packages::route("GET", "/auth/settings", &headers(&[("cookie", &cookie)]), b"", &state);
    assert_eq!(resp.status, 200);
    assert!(resp.content_type.starts_with("text/html"));
    // Page should contain the username
    let body = String::from_utf8_lossy(&resp.body);
    assert!(body.contains(TEST_USER));
}

#[test]
fn create_token_requires_browser_session() {
    let state = make_state();
    let resp = packages::route("POST", "/auth/tokens", &headers(&[]), br#"{"name":"test"}"#, &state);
    assert_eq!(resp.status, 401);
}

#[test]
fn create_token_returns_usable_api_token() {
    let state = make_state();
    let cookie = make_browser_session(&state);
    let h = headers(&[("cookie", &cookie)]);

    let resp = packages::route("POST", "/auth/tokens", &h, br#"{"name":"ci","expires_days":7}"#, &state);
    assert_eq!(resp.status, 200);
    let token = body_json(&resp)["token"].as_str().expect("token field").to_string();
    assert_eq!(token.len(), 64, "token should be 64 hex chars");

    // The new token must authenticate API calls
    let auth_h = headers(&[("authorization", &format!("Bearer {token}"))]);
    let pkg_body = br#"{"arch":"aarch64-linux-gnu","pkg":"test","ver":"1","rel":"1","deps":[],"mkdeps":[]}"#;
    let resp = packages::route("POST", "/api/publish", &auth_h, pkg_body, &state);
    assert_eq!(resp.status, 201, "new token should authenticate publish");
}

#[test]
fn delete_token_revokes_api_access() {
    let state = make_state();
    let cookie = make_browser_session(&state);
    let h = headers(&[("cookie", &cookie)]);

    // Create a fresh token
    let resp = packages::route("POST", "/auth/tokens", &h, br#"{"name":"ephemeral"}"#, &state);
    assert_eq!(resp.status, 200);
    let token = body_json(&resp)["token"].as_str().unwrap().to_string();

    // Verify it works
    let auth_h = headers(&[("authorization", &format!("Bearer {token}"))]);
    let pkg_body = br#"{"arch":"aarch64-linux-gnu","pkg":"t","ver":"1","rel":"1","deps":[],"mkdeps":[]}"#;
    let resp = packages::route("POST", "/api/publish", &auth_h, pkg_body, &state);
    assert_eq!(resp.status, 201);

    // Look up the token ID
    let token_id = {
        let db = state.db.lock().unwrap();
        db.list_tokens("test-user-id").unwrap()
            .into_iter()
            .find(|t| t.name == "ephemeral")
            .expect("token should be listed")
            .id
    };

    // Delete it
    let del_body = format!(r#"{{"id":"{token_id}"}}"#);
    let resp = packages::route("POST", "/auth/tokens/delete", &h, del_body.as_bytes(), &state);
    assert_eq!(resp.status, 200);

    // Token must no longer authenticate
    let resp = packages::route("POST", "/api/publish", &auth_h, pkg_body, &state);
    assert_eq!(resp.status, 401, "deleted token must be rejected");
}

#[test]
fn logout_invalidates_browser_session() {
    let state = make_state();
    let cookie = make_browser_session(&state);
    let h = headers(&[("cookie", &cookie)]);

    // Confirm session is live
    let resp = packages::route("GET", "/auth/settings", &h, b"", &state);
    assert_eq!(resp.status, 200);

    // Logout
    let resp = packages::route("GET", "/auth/logout", &h, b"", &state);
    assert_eq!(resp.status, 302);

    // Same cookie must no longer grant access — server invalidated the session, not just the cookie
    let resp = packages::route("GET", "/auth/settings", &h, b"", &state);
    assert_eq!(resp.status, 302, "session must be invalidated server-side after logout");
}

#[test]
fn session_cookie_does_not_authenticate_api_calls() {
    // Browser sessions are for the settings UI only, not for Bearer-token API auth.
    let state = make_state();
    let cookie = make_browser_session(&state);
    // Pass the session cookie value as a Bearer token — must be rejected
    let session_val = cookie.strip_prefix("kominka_session=").unwrap();
    let h = headers(&[("authorization", &format!("Bearer {session_val}"))]);
    let pkg_body = br#"{"arch":"aarch64-linux-gnu","pkg":"t","ver":"1","rel":"1","deps":[],"mkdeps":[]}"#;
    let resp = packages::route("POST", "/api/publish", &h, pkg_body, &state);
    assert_eq!(resp.status, 401, "browser session token must not work as a Bearer token");
}

#[test]
fn root_shows_signin_link_when_unauthenticated() {
    let state = make_state();
    let resp = packages::route("GET", "/", &headers(&[]), b"", &state);
    assert_eq!(resp.status, 200);
    let body = String::from_utf8_lossy(&resp.body);
    assert!(body.contains("sign in"), "should show sign-in link when not logged in");
    assert!(!body.contains("sign out"), "should not show sign-out when not logged in");
}

#[test]
fn root_shows_settings_link_when_authenticated() {
    let state = make_state();
    let cookie = make_browser_session(&state);
    let resp = packages::route("GET", "/", &headers(&[("cookie", &cookie)]), b"", &state);
    assert_eq!(resp.status, 200);
    let body = String::from_utf8_lossy(&resp.body);
    assert!(body.contains("settings"), "should show settings link when logged in");
    assert!(body.contains("sign out"), "should show sign-out when logged in");
    assert!(body.contains(TEST_USER), "should show username when logged in");
}

// ── Upload serialization tests ────────────────────────────────────────────────

#[test]
fn upload_same_package_twice_is_idempotent() {
    let state = make_state();
    let r1 = upload_pkg(&state, "aarch64-linux-gnu", "curl", "1.0", "1", "", b"data");
    let r2 = upload_pkg(&state, "aarch64-linux-gnu", "curl", "1.0", "1", "", b"data");
    assert_eq!(r1.status, 201);
    assert_eq!(r2.status, 201);
    // Package appears exactly once in the index.
    let resp = packages::route("GET", "/aarch64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    assert_eq!(body_json(&resp)["packages"].as_object().unwrap().len(), 1);
}

#[test]
fn concurrent_uploads_of_different_packages_both_land_in_index() {
    use std::sync::Arc;
    let state = Arc::new(make_state());

    let s1 = state.clone();
    let t1 = std::thread::spawn(move || {
        upload_pkg(&s1, "x86_64-linux-gnu", "pkg-a", "1.0", "1", "", b"data-a")
    });
    let s2 = state.clone();
    let t2 = std::thread::spawn(move || {
        upload_pkg(&s2, "x86_64-linux-gnu", "pkg-b", "1.0", "1", "", b"data-b")
    });

    assert_eq!(t1.join().unwrap().status, 201);
    assert_eq!(t2.join().unwrap().status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let pkgs = body_json(&resp);
    let pkg_map = pkgs["packages"].as_object().unwrap();
    assert!(pkg_map.contains_key("pkg-a"), "pkg-a missing from index");
    assert!(pkg_map.contains_key("pkg-b"), "pkg-b missing from index");
}

#[test]
fn concurrent_uploads_of_same_package_both_succeed() {
    use std::sync::Arc;
    let state = Arc::new(make_state());

    let s1 = state.clone();
    let t1 = std::thread::spawn(move || {
        upload_pkg(&s1, "x86_64-linux-gnu", "zlib", "1.3", "1", "", b"tarball")
    });
    let s2 = state.clone();
    let t2 = std::thread::spawn(move || {
        upload_pkg(&s2, "x86_64-linux-gnu", "zlib", "1.3", "1", "", b"tarball")
    });

    assert_eq!(t1.join().unwrap().status, 201);
    assert_eq!(t2.join().unwrap().status, 201);

    let resp = packages::route("GET", "/x86_64-linux-gnu/packages.json", &headers(&[]), b"", &state);
    let idx = body_json(&resp);
    assert_eq!(idx["packages"]["zlib"]["ver"], "1.3");
    // Package appears exactly once despite two concurrent uploads.
    assert_eq!(idx["packages"].as_object().unwrap().len(), 1);
}
