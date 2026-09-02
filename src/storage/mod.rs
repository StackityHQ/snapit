//! S3-compatible object storage engine.

use anyhow::{bail, Context, Result};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::{BehaviorVersion, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use std::path::Path;
use tracing::info;

use crate::config::{AppConfig, StorageEntry};

pub async fn build_client(entry: &StorageEntry) -> Result<Client> {
    let creds = Credentials::new(
        &entry.access_key_id,
        &entry.secret_access_key,
        None,
        None,
        "snapit",
    );

    let mut builder = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(entry.region.clone()))
        .credentials_provider(creds)
        .force_path_style(entry.path_style);

    if !entry.endpoint.is_empty() {
        builder = builder.endpoint_url(&entry.endpoint);
    }

    let conf = builder.build();
    Ok(Client::from_conf(conf))
}

pub async fn test_storage(entry: &StorageEntry) -> Result<()> {
    let client = build_client(entry).await?;
    client
        .head_bucket()
        .bucket(&entry.bucket)
        .send()
        .await
        .with_context(|| format!("head_bucket failed for storage '{}'", entry.name))?;

    // Put and delete a tiny probe object
    let key = format!(".snapit-probe/{}", uuid::Uuid::new_v4());
    client
        .put_object()
        .bucket(&entry.bucket)
        .key(&key)
        .body(ByteStream::from_static(b"snapit-ok"))
        .send()
        .await
        .with_context(|| format!("put_object probe failed for '{}'", entry.name))?;

    client
        .delete_object()
        .bucket(&entry.bucket)
        .key(&key)
        .send()
        .await
        .with_context(|| format!("delete_object probe failed for '{}'", entry.name))?;

    info!(storage = %entry.name, "storage test ok");
    Ok(())
}

pub async fn upload_file(entry: &StorageEntry, local: &Path, object_key: &str) -> Result<()> {
    let client = build_client(entry).await?;
    let meta = std::fs::metadata(local)?;
    let stream = ByteStream::from_path(local)
        .await
        .with_context(|| format!("read {}", local.display()))?;

    client
        .put_object()
        .bucket(&entry.bucket)
        .key(object_key)
        .body(stream)
        .content_length(meta.len() as i64)
        .send()
        .await
        .with_context(|| format!("upload to '{}' failed", entry.name))?;

    // Verify object exists
    let head = client
        .head_object()
        .bucket(&entry.bucket)
        .key(object_key)
        .send()
        .await
        .with_context(|| format!("verify upload on '{}'", entry.name))?;

    if let Some(len) = head.content_length() {
        if len as u64 != meta.len() {
            bail!(
                "upload size mismatch on '{}': local {} remote {}",
                entry.name,
                meta.len(),
                len
            );
        }
    }

    info!(storage = %entry.name, key = %object_key, "uploaded");
    Ok(())
}

pub async fn upload_to_storages(
    cfg: &AppConfig,
    names: &[String],
    local: &Path,
    object_key: &str,
) -> Result<Vec<String>> {
    let mut uploaded = Vec::new();
    for name in names {
        let entry = cfg
            .find_storage(name)
            .with_context(|| format!("storage not found: {name}"))?;
        if !entry.enabled {
            bail!("storage '{name}' is disabled");
        }
        upload_file(entry, local, object_key).await?;
        uploaded.push(name.clone());
    }
    Ok(uploaded)
}

pub async fn download_file(entry: &StorageEntry, object_key: &str, dest: &Path) -> Result<()> {
    let client = build_client(entry).await?;
    let resp = client
        .get_object()
        .bucket(&entry.bucket)
        .key(object_key)
        .send()
        .await
        .with_context(|| format!("download from '{}'", entry.name))?;

    let bytes = resp.body.collect().await?.into_bytes();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("download.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_entry_fields() {
        let e = StorageEntry {
            name: "arvan".into(),
            provider: "s3".into(),
            region: "ir-thr-at1".into(),
            bucket: "bucket".into(),
            access_key_id: "key".into(),
            secret_access_key: "secret".into(),
            endpoint: "https://s3.example.com".into(),
            path_style: true,
            enabled: true,
            retention: None,
        };
        assert_eq!(e.name, "arvan");
        assert!(e.path_style);
    }
}
