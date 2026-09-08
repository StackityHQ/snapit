//! Safe PostgreSQL restore workflow.

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::info;
use which::which;

use crate::config::{AppConfig, DatabaseEntry};
use crate::metadata::{BackupRecord, MetadataStore};
use crate::util::verify_checksum;

pub struct RestoreOptions {
    pub backup_id: String,
    pub target_database: Option<String>,
    pub confirm: bool,
    pub yes: bool,
}

pub fn preview(cfg: &AppConfig, store: &MetadataStore, opts: &RestoreOptions) -> Result<String> {
    let rec = store.require(&opts.backup_id)?;
    let target_name = opts
        .target_database
        .clone()
        .or_else(|| rec.database.clone())
        .context("no target database")?;
    let target = cfg
        .find_database(&target_name)
        .with_context(|| format!("target database not found: {target_name}"))?;

    Ok(format!(
        "Restore preview\n\
         ────────────────\n\
         Backup ID     {}\n\
         Backup name   {}\n\
         Source DB     {}\n\
         Target name   {}\n\
         Target host   {}:{}\n\
         Target DB     {}\n\
         Local file    {}\n\
         Size          {} bytes\n\
         \n\
         WARNING: This will restore into '{}' on {}:{}/{}.\n\
         Existing objects may be overwritten or conflict.\n\
         This is a destructive operation.",
        rec.id,
        rec.name,
        rec.database.as_deref().unwrap_or("-"),
        target.name,
        target.host,
        target.port,
        target.database,
        rec.local_path,
        rec.size_bytes,
        target.database,
        target.host,
        target.port,
        target.database,
    ))
}

pub fn restore(cfg: &AppConfig, store: &MetadataStore, opts: &RestoreOptions) -> Result<()> {
    let rec = store.require(&opts.backup_id)?;
    let target_name = opts
        .target_database
        .clone()
        .or_else(|| rec.database.clone())
        .context("no target database")?;
    let target = cfg
        .find_database(&target_name)
        .with_context(|| format!("target database not found: {target_name}"))?
        .clone();

    if !opts.yes {
        bail!("restore aborted: pass --yes after reviewing the preview to confirm");
    }

    let path = Path::new(&rec.local_path);
    if !path.exists() {
        bail!("backup file missing: {}", path.display());
    }
    verify_checksum(path, &rec.checksum)?;

    info!(
        backup_id = %rec.id,
        target = %target.name,
        "restore started"
    );

    run_psql_restore(&target, path)?;
    info!(backup_id = %rec.id, "restore completed");
    Ok(())
}

fn run_psql_restore(db: &DatabaseEntry, archive: &Path) -> Result<()> {
    let psql = which("psql").context("psql not found in PATH")?;

    let mut env: HashMap<String, String> = HashMap::new();
    env.insert("PGPASSWORD".into(), db.password.clone());
    env.insert("PGCONNECT_TIMEOUT".into(), "30".into());

    let mut child = Command::new(&psql)
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
            "-v",
            "ON_ERROR_STOP=1",
        ])
        .envs(&env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn psql")?;

    let stdin = child.stdin.take().context("psql stdin")?;
    let file = File::open(archive)?;
    let mut decoder = GzDecoder::new(BufReader::new(file));
    let mut writer = BufWriter::new(stdin);
    std::io::copy(&mut decoder, &mut writer).context("stream restore")?;
    writer.flush()?;
    drop(writer);

    let output = child.wait_with_output().context("wait psql")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("psql restore failed: {}", scrub(&stderr));
    }
    Ok(())
}

fn scrub(s: &str) -> String {
    s.replace(&std::env::var("PGPASSWORD").unwrap_or_default(), "***")
}

/// Describe restore impact for UI (no secrets).
pub fn describe(rec: &BackupRecord, target: &DatabaseEntry) -> String {
    format!(
        "Restore {} → {}@{}:{}/{}",
        rec.name, target.name, target.host, target.port, target.database
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_has_no_password() {
        let rec = BackupRecord {
            id: "1".into(),
            name: "x.sql.gz".into(),
            database: Some("samane".into()),
            group_name: None,
            created_at: chrono::Utc::now(),
            size_bytes: 1,
            checksum: "a".into(),
            local_path: "/tmp/x".into(),
            upload_status: "ok".into(),
            storage_destinations: vec![],
            status: crate::metadata::BackupStatus::Success,
            duration_ms: 1,
            error: None,
            job_id: None,
        };
        let db = DatabaseEntry {
            name: "samane".into(),
            driver: "postgres".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            database: "postgres".into(),
            username: "u".into(),
            password: "SUPERSECRET".into(),
            enabled: true,
            retention: None,
        };
        let d = describe(&rec, &db);
        assert!(!d.contains("SUPERSECRET"));
    }
}
