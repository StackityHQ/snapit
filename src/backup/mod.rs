//! PostgreSQL backup engine: dump, compress, checksum, verify.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use tracing::{error, info};
use which::which;

use crate::config::{AppConfig, DatabaseEntry};
use crate::metadata::{new_backup_id, BackupRecord, BackupStatus, MetadataStore};
use crate::storage::upload_to_storages;
use crate::util::{
    backup_filename, format_bytes, format_duration, group_dirname, sha256_file, verify_checksum,
    JobLock,
};

#[derive(Debug, Clone)]
pub struct BackupRequest {
    pub databases: Vec<String>,
    pub group: Option<String>,
    pub storages: Vec<String>,
}

#[derive(Debug)]
pub struct BackupOutcome {
    pub records: Vec<BackupRecord>,
}

pub async fn run_backup(
    cfg: &AppConfig,
    store: &MetadataStore,
    req: BackupRequest,
) -> Result<BackupOutcome> {
    let lock_key = if let Some(ref g) = req.group {
        format!("group-{g}")
    } else {
        format!("db-{}", req.databases.join("-"))
    };
    let _lock = JobLock::acquire(&cfg.paths.lock_dir, &lock_key)?;

    let job_id = new_backup_id();
    info!(job_id = %job_id, "job started");

    let db_names = resolve_databases(cfg, &req)?;
    if db_names.is_empty() {
        bail!("no databases selected for backup");
    }

    let storage_names = resolve_storages(cfg, &req)?;
    let at = Utc::now();
    let group_dir = req
        .group
        .as_ref()
        .map(|g| cfg.paths.backups_dir.join(group_dirname(g, at)));

    if let Some(ref dir) = group_dir {
        fs::create_dir_all(dir)?;
    }

    let mut records = Vec::new();
    let mut any_fail = false;

    for db_name in &db_names {
        let db = cfg
            .find_database(db_name)
            .with_context(|| format!("database not found: {db_name}"))?;
        if !db.enabled {
            bail!("database '{db_name}' is disabled");
        }

        match backup_one_database(
            cfg,
            store,
            db,
            &storage_names,
            req.group.as_deref(),
            group_dir.as_deref(),
            &job_id,
            at,
        )
        .await
        {
            Ok(rec) => {
                info!(
                    database = %db_name,
                    name = %rec.name,
                    size = rec.size_bytes,
                    "database backup completed"
                );
                records.push(rec);
            }
            Err(e) => {
                error!(database = %db_name, error = %e, "database backup failed");
                any_fail = true;
                let fail = BackupRecord {
                    id: new_backup_id(),
                    name: backup_filename(db_name, at),
                    database: Some(db_name.clone()),
                    group_name: req.group.clone(),
                    created_at: at,
                    size_bytes: 0,
                    checksum: String::new(),
                    local_path: String::new(),
                    upload_status: "failed".into(),
                    storage_destinations: storage_names.clone(),
                    status: BackupStatus::Failed,
                    duration_ms: 0,
                    error: Some(sanitize_error(&e.to_string())),
                    job_id: Some(job_id.clone()),
                };
                let _ = store.insert(&fail);
                records.push(fail);
            }
        }
    }

    if any_fail {
        bail!("one or more database backups failed");
    }
    Ok(BackupOutcome { records })
}

fn resolve_databases(cfg: &AppConfig, req: &BackupRequest) -> Result<Vec<String>> {
    if let Some(ref g) = req.group {
        let group = cfg
            .find_group(g)
            .with_context(|| format!("group not found: {g}"))?;
        if !group.enabled {
            bail!("group '{g}' is disabled");
        }
        return Ok(group.databases.clone());
    }
    Ok(req.databases.clone())
}

fn resolve_storages(cfg: &AppConfig, req: &BackupRequest) -> Result<Vec<String>> {
    if !req.storages.is_empty() {
        for s in &req.storages {
            if cfg.find_storage(s).is_none() {
                bail!("storage not found: {s}");
            }
        }
        return Ok(req.storages.clone());
    }
    if let Some(ref g) = req.group {
        if let Some(group) = cfg.find_group(g) {
            if !group.storages.is_empty() {
                return Ok(group.storages.clone());
            }
        }
    }
    // Default: all enabled storages
    Ok(cfg.enabled_storages().map(|s| s.name.clone()).collect())
}

#[allow(clippy::too_many_arguments)]
async fn backup_one_database(
    cfg: &AppConfig,
    store: &MetadataStore,
    db: &DatabaseEntry,
    storages: &[String],
    group: Option<&str>,
    group_dir: Option<&Path>,
    job_id: &str,
    at: chrono::DateTime<Utc>,
) -> Result<BackupRecord> {
    let start = Instant::now();
    let id = new_backup_id();
    let name = backup_filename(&db.name, at);
    let dest_dir = group_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cfg.paths.backups_dir.clone());
    fs::create_dir_all(&dest_dir)?;
    let final_path = dest_dir.join(&name);
    let tmp_path = dest_dir.join(format!(".{name}.partial"));

    let mut record = BackupRecord {
        id: id.clone(),
        name: name.clone(),
        database: Some(db.name.clone()),
        group_name: group.map(|s| s.to_string()),
        created_at: at,
        size_bytes: 0,
        checksum: String::new(),
        local_path: final_path.display().to_string(),
        upload_status: "pending".into(),
        storage_destinations: storages.to_vec(),
        status: BackupStatus::Running,
        duration_ms: 0,
        error: None,
        job_id: Some(job_id.to_string()),
    };
    store.insert(&record)?;

    info!(database = %db.name, "database backup started");

    // Cleanup partial on failure
    let cleanup = |path: &Path| {
        let _ = fs::remove_file(path);
    };

    if let Err(e) = pg_dump_compressed(db, &tmp_path) {
        cleanup(&tmp_path);
        record.status = BackupStatus::Failed;
        record.error = Some(sanitize_error(&e.to_string()));
        record.duration_ms = start.elapsed().as_millis() as u64;
        store.update(&record)?;
        return Err(e);
    }
    info!(database = %db.name, "compression completed");

    let checksum = match sha256_file(&tmp_path) {
        Ok(c) => c,
        Err(e) => {
            cleanup(&tmp_path);
            record.status = BackupStatus::Failed;
            record.error = Some(sanitize_error(&e.to_string()));
            store.update(&record)?;
            return Err(e);
        }
    };
    info!(database = %db.name, "checksum completed");

    // Atomic rename
    fs::rename(&tmp_path, &final_path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), final_path.display()))?;

    let size = fs::metadata(&final_path)?.len();
    record.size_bytes = size;
    record.checksum = checksum.clone();
    record.local_path = final_path.display().to_string();

    // Verify archive before upload
    verify_backup_file(&final_path, &checksum)?;

    let upload_result = if storages.is_empty() {
        Ok(vec![])
    } else {
        info!(database = %db.name, "upload started");
        let object_key = if let Some(g) = group {
            format!("{}/{}/{}", g, group_dirname(g, at), name)
        } else {
            format!("{}/{}", db.name, name)
        };
        upload_to_storages(cfg, storages, &final_path, &object_key).await
    };

    match upload_result {
        Ok(uploaded) => {
            info!(database = %db.name, "upload completed");
            record.upload_status = if uploaded.is_empty() {
                "local-only".into()
            } else {
                "uploaded".into()
            };
            record.storage_destinations = uploaded;
            record.status = BackupStatus::Success;
        }
        Err(e) => {
            error!(database = %db.name, error = %sanitize_error(&e.to_string()), "upload failed");
            record.upload_status = "failed".into();
            record.status = BackupStatus::Failed;
            record.error = Some(sanitize_error(&e.to_string()));
            record.duration_ms = start.elapsed().as_millis() as u64;
            store.update(&record)?;
            // Keep local file but report failure — upload failure is not success
            return Err(e.context("upload failed; backup not marked successful"));
        }
    }

    record.duration_ms = start.elapsed().as_millis() as u64;
    store.update(&record)?;
    info!(
        database = %db.name,
        size = %format_bytes(size),
        duration = %format_duration(start.elapsed()),
        "backup success"
    );
    Ok(record)
}

/// Stream pg_dump -> gzip without loading entire dump into memory.
pub fn pg_dump_compressed(db: &DatabaseEntry, dest: &Path) -> Result<()> {
    let pg_dump = which("pg_dump").context("pg_dump not found in PATH")?;

    let mut env: HashMap<String, String> = HashMap::new();
    env.insert("PGPASSWORD".into(), db.password.clone());
    // Prevent pg from prompting
    env.insert("PGCONNECT_TIMEOUT".into(), "30".into());

    let mut child = Command::new(&pg_dump)
        .args([
            "-h",
            &db.host,
            "-p",
            &db.port.to_string(),
            "-U",
            &db.username,
            "-d",
            &db.database,
            "--no-password",
            "--format=plain",
            "--no-owner",
            "--no-acl",
        ])
        .envs(&env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn pg_dump")?;

    let stdout = child.stdout.take().context("pg_dump stdout")?;
    let out_file = File::create(dest).with_context(|| format!("create {}", dest.display()))?;
    let mut encoder = GzEncoder::new(BufWriter::new(out_file), Compression::default());

    let mut reader = BufReader::new(stdout);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).context("read pg_dump")?;
        if n == 0 {
            break;
        }
        encoder.write_all(&buf[..n]).context("write gzip")?;
    }
    encoder.finish().context("finish gzip")?;

    let output = child.wait_with_output().context("wait pg_dump")?;
    if !output.status.success() {
        let _ = fs::remove_file(dest);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "pg_dump failed for database '{}': {}",
            db.name,
            sanitize_error(&stderr)
        );
    }
    Ok(())
}

pub fn verify_backup_file(path: &Path, expected_checksum: &str) -> Result<()> {
    if !path.exists() {
        bail!("backup file missing: {}", path.display());
    }
    verify_checksum(path, expected_checksum)?;

    // Ensure gzip opens and starts with SQL-looking content or custom dump header
    let file = File::open(path)?;
    let mut decoder = GzDecoder::new(BufReader::new(file));
    let mut header = [0u8; 256];
    let n = decoder
        .read(&mut header)
        .context("decompress backup header")?;
    if n == 0 {
        bail!("backup archive is empty");
    }
    let text = String::from_utf8_lossy(&header[..n]);
    let ok = text.contains("--")
        || text.contains("PostgreSQL")
        || text.contains("CREATE")
        || text.contains("SET ")
        || text.contains("pg_dump");
    if !ok {
        bail!("backup does not look like a PostgreSQL dump");
    }
    info!(path = %path.display(), "verification ok");
    Ok(())
}

pub fn verify_record(store: &MetadataStore, id: &str) -> Result<()> {
    let rec = store.require(id)?;
    let path = PathBuf::from(&rec.local_path);
    verify_backup_file(&path, &rec.checksum)?;
    Ok(())
}

pub fn delete_backup(store: &MetadataStore, id: &str, remove_file: bool) -> Result<BackupRecord> {
    let rec = store.require(id)?;
    if remove_file {
        let path = PathBuf::from(&rec.local_path);
        if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    store.delete(&rec.id)?;
    Ok(rec)
}

fn sanitize_error(msg: &str) -> String {
    let mut out = msg.to_string();
    for pattern in ["password=", "Password=", "PGPASSWORD="] {
        let mut search_from = 0;
        while let Some(rel) = out[search_from..].find(pattern) {
            let idx = search_from + rel;
            let rest_start = idx + pattern.len();
            let rest = &out[rest_start..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '\'' || c == '"')
                .unwrap_or(rest.len());
            out.replace_range(rest_start..rest_start + end, "***");
            search_from = rest_start + 3;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn verify_valid_dump_archive() {
        let mut tmp = NamedTempFile::new().unwrap();
        {
            let mut enc = GzEncoder::new(&mut tmp, Compression::fast());
            writeln!(enc, "--\n-- PostgreSQL database dump\n--").unwrap();
            writeln!(enc, "SET statement_timeout = 0;").unwrap();
            enc.finish().unwrap();
        }
        let path = tmp.path();
        let sum = sha256_file(path).unwrap();
        verify_backup_file(path, &sum).unwrap();
    }

    #[test]
    fn sanitize_hides_password() {
        let s = sanitize_error("connection failed password=hunter2 host=x");
        assert!(!s.contains("hunter2"));
    }
}
