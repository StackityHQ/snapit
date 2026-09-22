//! SQLite metadata store for backup history.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    Pending,
    Running,
    Success,
    Failed,
    Partial,
}

impl BackupStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Partial => "partial",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "success" => Self::Success,
            "failed" => Self::Failed,
            "partial" => Self::Partial,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub name: String,
    pub database: Option<String>,
    pub group_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
    pub checksum: String,
    pub local_path: String,
    pub upload_status: String,
    pub storage_destinations: Vec<String>,
    pub status: BackupStatus,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub job_id: Option<String>,
}

pub struct MetadataStore {
    conn: Connection,
}

impl MetadataStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open metadata db {}", path.display()))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS backups (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                database_name TEXT,
                group_name TEXT,
                created_at TEXT NOT NULL,
                size_bytes INTEGER NOT NULL DEFAULT 0,
                checksum TEXT NOT NULL DEFAULT '',
                local_path TEXT NOT NULL DEFAULT '',
                upload_status TEXT NOT NULL DEFAULT 'pending',
                storage_destinations TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL DEFAULT 'pending',
                duration_ms INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                job_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_backups_created ON backups(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_backups_db ON backups(database_name);
            CREATE INDEX IF NOT EXISTS idx_backups_status ON backups(status);
            ",
        )?;
        Ok(Self { conn })
    }

    pub fn insert(&self, record: &BackupRecord) -> Result<()> {
        let storages = serde_json::to_string(&record.storage_destinations)?;
        self.conn.execute(
            "INSERT INTO backups (
                id, name, database_name, group_name, created_at, size_bytes, checksum,
                local_path, upload_status, storage_destinations, status, duration_ms, error, job_id
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                record.id,
                record.name,
                record.database,
                record.group_name,
                record.created_at.to_rfc3339(),
                record.size_bytes as i64,
                record.checksum,
                record.local_path,
                record.upload_status,
                storages,
                record.status.as_str(),
                record.duration_ms as i64,
                record.error,
                record.job_id,
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, record: &BackupRecord) -> Result<()> {
        let storages = serde_json::to_string(&record.storage_destinations)?;
        let n = self.conn.execute(
            "UPDATE backups SET
                name=?2, database_name=?3, group_name=?4, created_at=?5, size_bytes=?6,
                checksum=?7, local_path=?8, upload_status=?9, storage_destinations=?10,
                status=?11, duration_ms=?12, error=?13, job_id=?14
             WHERE id=?1",
            params![
                record.id,
                record.name,
                record.database,
                record.group_name,
                record.created_at.to_rfc3339(),
                record.size_bytes as i64,
                record.checksum,
                record.local_path,
                record.upload_status,
                storages,
                record.status.as_str(),
                record.duration_ms as i64,
                record.error,
                record.job_id,
            ],
        )?;
        if n == 0 {
            bail!("backup not found: {}", record.id);
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<BackupRecord>> {
        self.conn
            .query_row(
                "SELECT id, name, database_name, group_name, created_at, size_bytes, checksum,
                        local_path, upload_status, storage_destinations, status, duration_ms, error, job_id
                 FROM backups WHERE id=?1 OR name=?1",
                params![id],
                row_to_record,
            )
            .optional()
            .context("query backup")
    }

    pub fn require(&self, id: &str) -> Result<BackupRecord> {
        self.get(id)?
            .with_context(|| format!("backup not found: {id}"))
    }

    pub fn list(&self, limit: usize) -> Result<Vec<BackupRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, database_name, group_name, created_at, size_bytes, checksum,
                    local_path, upload_status, storage_destinations, status, duration_ms, error, job_id
             FROM backups ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_for_database(&self, database: &str) -> Result<Vec<BackupRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, database_name, group_name, created_at, size_bytes, checksum,
                    local_path, upload_status, storage_destinations, status, duration_ms, error, job_id
             FROM backups
             WHERE database_name=?1 AND status='success'
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![database], row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_for_group(&self, group: &str) -> Result<Vec<BackupRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, database_name, group_name, created_at, size_bytes, checksum,
                    local_path, upload_status, storage_destinations, status, duration_ms, error, job_id
             FROM backups
             WHERE group_name=?1 AND status='success'
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![group], row_to_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM backups WHERE id=?1", params![id])?;
        if n == 0 {
            bail!("backup not found: {id}");
        }
        Ok(())
    }

    pub fn count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM backups", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn last_backup(&self) -> Result<Option<BackupRecord>> {
        self.conn
            .query_row(
                "SELECT id, name, database_name, group_name, created_at, size_bytes, checksum,
                        local_path, upload_status, storage_destinations, status, duration_ms, error, job_id
                 FROM backups ORDER BY created_at DESC LIMIT 1",
                [],
                row_to_record,
            )
            .optional()
            .context("last backup")
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<BackupRecord> {
    let created_at_s: String = row.get(4)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let storages_s: String = row.get(9)?;
    let storage_destinations: Vec<String> = serde_json::from_str(&storages_s).unwrap_or_default();
    let status_s: String = row.get(10)?;
    let size: i64 = row.get(5)?;
    let duration: i64 = row.get(11)?;
    Ok(BackupRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        database: row.get(2)?,
        group_name: row.get(3)?,
        created_at,
        size_bytes: size.max(0) as u64,
        checksum: row.get(6)?,
        local_path: row.get(7)?,
        upload_status: row.get(8)?,
        storage_destinations,
        status: BackupStatus::parse(&status_s),
        duration_ms: duration.max(0) as u64,
        error: row.get(12)?,
        job_id: row.get(13)?,
    })
}

pub fn new_backup_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn insert_get_list_delete() {
        let dir = TempDir::new().unwrap();
        let store = MetadataStore::open(&dir.path().join("m.db")).unwrap();
        let id = new_backup_id();
        let rec = BackupRecord {
            id: id.clone(),
            name: "samane_2026-09-22_23-30-00.sql.gz".into(),
            database: Some("samane".into()),
            group_name: None,
            created_at: Utc::now(),
            size_bytes: 1024,
            checksum: "abc".into(),
            local_path: "/tmp/x.sql.gz".into(),
            upload_status: "uploaded".into(),
            storage_destinations: vec!["arvan".into()],
            status: BackupStatus::Success,
            duration_ms: 1000,
            error: None,
            job_id: None,
        };
        store.insert(&rec).unwrap();
        let got = store.require(&id).unwrap();
        assert_eq!(got.name, rec.name);
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.list(10).unwrap().len(), 1);
        store.delete(&id).unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }
}
