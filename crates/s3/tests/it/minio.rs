//! The same lifecycle against a real S3-compatible service, when one is configured:
//!
//! ```sh
//! PHOTOCRAFT_S3_TEST_ENDPOINT=http://127.0.0.1:9000 PHOTOCRAFT_S3_TEST_BUCKET=pcraft-test \
//! PHOTOCRAFT_S3_TEST_ACCESS_KEY=… PHOTOCRAFT_S3_TEST_SECRET_KEY=… \
//! cargo test -p photocraft-s3 --test it minio
//! ```
//!
//! `PHOTOCRAFT_S3_TEST_REGION` defaults to `us-east-1`. Use a throwaway bucket: each run writes
//! under a new prefix and deletes nothing. Without the variables the test passes without
//! running.

use photocraft_s3::{Credentials, S3Config};

use crate::bundle_lifecycle;

#[test]
fn minio_or_another_real_service_keeps_bundles_and_refuses_lost_updates() {
    let var = |name: &str| std::env::var(format!("PHOTOCRAFT_S3_TEST_{name}")).ok();
    let (Some(endpoint), Some(bucket), Some(access_key_id), Some(secret_access_key)) = (var("ENDPOINT"), var("BUCKET"), var("ACCESS_KEY"), var("SECRET_KEY"))
    else {
        eprintln!("PHOTOCRAFT_S3_TEST_* not set: skipping the real-service test");
        return;
    };
    let config = S3Config {
        endpoint,
        region: var("REGION").unwrap_or_else(|| "us-east-1".into()),
        bucket,
        credentials: Credentials { access_key_id, secret_access_key, session_token: None },
        virtual_hosted: false,
    };
    let run = format!("photocraft-s3-test/{}-{}/", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    bundle_lifecycle(&config, &format!("{run}pool/"), &run);
}
