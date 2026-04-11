mod auth;
mod packages;
mod s3;

use std::sync::Arc;

use axum::{Router, routing::get};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

/// Shared application state.
pub struct AppState {
    pub s3: s3::S3Client,
    pub api_key_hash: [u8; 32],
    /// In-memory package index per architecture, hydrated from S3 on startup.
    pub indexes: RwLock<std::collections::HashMap<String, packages::PackageIndex>>,
}

/// Configuration parsed from environment variables.
struct Config {
    listen_addr: String,
    s3_endpoint: String,
    s3_bucket: String,
    s3_access_key_id: String,
    s3_secret_access_key: String,
    s3_region: String,
    api_key: String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            listen_addr: env_or("LISTEN_ADDR", "127.0.0.1:3000"),
            s3_endpoint: env_required("S3_ENDPOINT"),
            s3_bucket: env_required("S3_BUCKET"),
            s3_access_key_id: env_required("S3_ACCESS_KEY_ID"),
            s3_secret_access_key: env_required("S3_SECRET_ACCESS_KEY"),
            s3_region: env_or("S3_REGION", "auto"),
            api_key: env_required("API_KEY"),
        }
    }
}

fn env_required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

async fn health() -> &'static str {
    r#"{"status":"ok"}"#
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Config::from_env();

    let api_key_hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(config.api_key.as_bytes());
        let result: [u8; 32] = h.finalize().into();
        result
    };

    let s3 = s3::S3Client::new(
        &config.s3_endpoint,
        &config.s3_bucket,
        &config.s3_access_key_id,
        &config.s3_secret_access_key,
        &config.s3_region,
    )
    .await;

    let state = Arc::new(AppState {
        s3,
        api_key_hash,
        indexes: RwLock::new(std::collections::HashMap::new()),
    });

    // Hydrate indexes from S3 on startup.
    for arch in ["aarch64-linux-gnu", "x86_64-linux-gnu"] {
        if let Some(idx) = packages::load_index_from_s3(&state.s3, arch).await {
            state.indexes.write().await.insert(arch.to_string(), idx);
            tracing::info!("loaded index for {arch}");
        }
    }

    let app = Router::new()
        .route("/health", get(health))
        .route("/{arch}/packages.json", get(packages::get_index))
        .route(
            "/{arch}/{pkg}/{file}",
            get(packages::get_tarball),
        )
        .route("/api/upload", axum::routing::post(packages::upload))
        .route("/api/publish", axum::routing::post(packages::publish))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.listen_addr)
        .await
        .expect("failed to bind");
    tracing::info!("listening on {}", config.listen_addr);
    axum::serve(listener, app).await.expect("server error");
}
