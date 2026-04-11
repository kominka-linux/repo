use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::primitives::ByteStream;

/// Thin wrapper around the S3 SDK client.
pub struct S3Client {
    client: Client,
    bucket: String,
}

impl S3Client {
    pub async fn new(
        endpoint: &str,
        bucket: &str,
        access_key_id: &str,
        secret_access_key: &str,
        region: &str,
    ) -> Self {
        let creds = Credentials::new(access_key_id, secret_access_key, None, None, "env");
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(endpoint)
            .region(Region::new(region.to_string()))
            .credentials_provider(creds)
            .force_path_style(true)
            .build();
        Self {
            client: Client::from_conf(config),
            bucket: bucket.to_string(),
        }
    }

    /// Get an object's bytes. Returns None if the key does not exist.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await;
        match resp {
            Ok(output) => {
                let bytes = output.body.collect().await.ok()?.into_bytes();
                Some(bytes.to_vec())
            }
            Err(_) => None,
        }
    }

    /// Check whether an object exists.
    pub async fn head(&self, key: &str) -> bool {
        self.client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .is_ok()
    }

    /// Upload bytes to a key.
    pub async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<(), String> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(body))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| format!("S3 put failed: {e}"))?;
        Ok(())
    }
}
