use std::collections::HashMap;
use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use kominka_repo::{AppState, packages, s3};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let listen_addr = env_or("LISTEN_ADDR", "127.0.0.1:3000");
    let api_key = env_required("API_KEY");

    let api_key_hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(api_key.as_bytes());
        h.finalize().into()
    };

    let s3 = s3::Storage::s3(
        &env_required("S3_ENDPOINT"),
        &env_required("S3_BUCKET"),
        &env_required("S3_ACCESS_KEY_ID"),
        &env_required("S3_SECRET_ACCESS_KEY"),
        &env_or("S3_REGION", "auto"),
    );

    let state = Arc::new(AppState {
        s3,
        api_key_hash,
        indexes: std::sync::RwLock::new(HashMap::new()),
    });

    // Hydrate indexes from S3 on startup.
    for arch in packages::KNOWN_ARCHES {
        if let Some(idx) = packages::load_index(&state.s3, arch) {
            state.indexes.write().unwrap().insert(arch.to_string(), idx);
            tracing::info!("loaded index for {arch}");
        }
    }

    let server = tiny_http::Server::http(&listen_addr).expect("failed to bind");
    tracing::info!("listening on {listen_addr}");

    for mut request in server.incoming_requests() {
        let state = state.clone();
        std::thread::spawn(move || {
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            let headers: HashMap<String, String> = request
                .headers()
                .iter()
                .map(|h| (h.field.as_str().as_str().to_lowercase(), h.value.as_str().to_string()))
                .collect();

            let mut body = Vec::new();
            let _ = request.as_reader().read_to_end(&mut body);

            let resp = packages::route(&method, &url, &headers, &body, &state);

            let header = tiny_http::Header::from_bytes(
                &b"Content-Type"[..],
                resp.content_type.as_bytes(),
            )
            .unwrap();

            let response = tiny_http::Response::from_data(resp.body)
                .with_status_code(resp.status)
                .with_header(header);

            if let Some(arch_path) = url.strip_prefix('/') {
                if arch_path.ends_with("/packages.json") && resp.status == 200 {
                    let cc = tiny_http::Header::from_bytes(
                        &b"Cache-Control"[..],
                        &b"public, max-age=60"[..],
                    )
                    .unwrap();
                    let _ = request.respond(response.with_header(cc));
                    return;
                }
            }

            let _ = request.respond(response);
        });
    }
}

fn env_required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
