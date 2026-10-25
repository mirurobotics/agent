//! Real-S3 checks for uploads. The backend's credentials allow only the calls a
//! multipart upload of one object needs, so these tests upload the way the
//! agent does, with credentials scoped the same way, and read each object back.

// standard crates
use std::collections::HashMap;

// internal crates
use crate::s3::harnesses::{env_var, real_bucket_tests, RealBucket};
use miru_agent::s3::{aws, sdk, Config, Credentials, Object, Options};

// external crates
use aws_sdk_s3::config::{Credentials as AwsCredentials, Region};
use aws_sdk_s3::Client;
use serde_json::json;

/// The test bucket, and credentials that can read objects in it and assume
/// the upload role.
struct Aws {
    bucket: String,
    region: String,
    /// The role each upload's credentials come from.
    upload_role: String,
    source: AwsCredentials,
}

impl RealBucket for Aws {
    fn load() -> Self {
        let source = AwsCredentials::new(
            env_var("AWS_ACCESS_KEY_ID"),
            env_var("AWS_SECRET_ACCESS_KEY"),
            Some(env_var("AWS_SESSION_TOKEN")),
            None,
            "env",
        );
        Self {
            bucket: env_var("AWS_IT_BUCKET"),
            region: env_var("AWS_REGION"),
            upload_role: env_var("AWS_IT_UPLOAD_ROLE"),
            source,
        }
    }

    fn name(&self) -> &str {
        &self.bucket
    }

    async fn upload_options(&self, obj: &Object) -> Options {
        let cfg = Config {
            creds: self.upload_credentials(obj).await,
            region: self.region.clone(),
        };
        let config = aws::config(cfg, sdk::http_client(), false);
        Options { config, gcs: false }
    }

    async fn read(&self, obj: &Object) -> (HashMap<String, String>, Vec<u8>) {
        let config = aws_sdk_s3::Config::builder()
            .region(Region::new(self.region.clone()))
            .credentials_provider(self.source.clone())
            .build();
        let resp = Client::from_conf(config)
            .get_object()
            .bucket(&obj.bucket)
            .key(&obj.key)
            .send()
            .await
            .unwrap();
        let metadata = resp.metadata().cloned().unwrap_or_default();
        let stored = resp.body.collect().await.unwrap().to_vec();
        (metadata, stored)
    }
}

impl Aws {
    /// Assumes the upload role with a session policy that allows uploading
    /// `obj` alone, as the backend does for each upload.
    async fn upload_credentials(&self, obj: &Object) -> Credentials {
        let policy = json!({
            "Version": "2012-10-17",
            "Statement": [{
                "Effect": "Allow",
                "Action": [
                    "s3:PutObject",
                    "s3:AbortMultipartUpload",
                    "s3:ListMultipartUploadParts",
                ],
                "Resource": format!("arn:aws:s3:::{}/{}", obj.bucket, obj.key),
            }],
        });
        let config = aws_sdk_sts::Config::builder()
            .region(Region::new(self.region.clone()))
            .credentials_provider(self.source.clone())
            .build();
        let resp = aws_sdk_sts::Client::from_conf(config)
            .assume_role()
            .role_arn(&self.upload_role)
            .role_session_name("agent-ci")
            .policy(policy.to_string())
            .send()
            .await
            .unwrap();
        let creds = resp.credentials().unwrap();
        Credentials {
            access_key_id: creds.access_key_id().to_string(),
            secret_access_key: creds.secret_access_key().to_string(),
            session_token: creds.session_token().to_string(),
        }
    }
}

real_bucket_tests!(Aws);
