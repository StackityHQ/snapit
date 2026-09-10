//! Retention policy evaluation and cleanup.

use anyhow::Result;
use chrono::{Duration, Utc};
use tracing::info;

use crate::config::{AppConfig, RetentionPolicy};
use crate::metadata::{BackupRecord, MetadataStore};

/// Decide which successful backups may be deleted under the effective policy.
/// Never deletes a backup still required by another applicable policy.
pub fn candidates_for_deletion(
    records: &[BackupRecord],
    policies: &[RetentionPolicy],
) -> Vec<String> {
    if policies.is_empty() || records.is_empty() {
        return Vec::new();
    }

    // A backup is protected if ANY policy still requires it.
    let mut protected = std::collections::HashSet::new();

    for policy in policies {
        match policy {
            RetentionPolicy::KeepLast { keep_last } => {
                for r in records.iter().take(*keep_last as usize) {
                    protected.insert(r.id.clone());
                }
            }
            RetentionPolicy::KeepDays { keep_days } => {
                let cutoff = Utc::now() - Duration::days(*keep_days as i64);
                for r in records {
                    if r.created_at >= cutoff {
                        protected.insert(r.id.clone());
                    }
                }
            }
        }
    }

    records
        .iter()
        .filter(|r| !protected.contains(&r.id))
        .map(|r| r.id.clone())
        .collect()
}

pub fn effective_policies(
    cfg: &AppConfig,
    database: Option<&str>,
    group: Option<&str>,
    storages: &[String],
) -> Vec<RetentionPolicy> {
    let mut policies = Vec::new();

    if let Some(db_name) = database {
        if let Some(db) = cfg.find_database(db_name) {
            if let Some(ref p) = db.retention {
                policies.push(p.clone());
            }
        }
    }

    if let Some(g_name) = group {
        if let Some(g) = cfg.find_group(g_name) {
            if let Some(ref p) = g.retention {
                policies.push(p.clone());
            }
        }
    }

    for s in storages {
        if let Some(st) = cfg.find_storage(s) {
            if let Some(ref p) = st.retention {
                policies.push(p.clone());
            }
        }
    }

    policies
}

pub fn apply_retention(
    cfg: &AppConfig,
    store: &MetadataStore,
    database: Option<&str>,
    group: Option<&str>,
) -> Result<Vec<String>> {
    let records = if let Some(g) = group {
        store.list_for_group(g)?
    } else if let Some(db) = database {
        store.list_for_database(db)?
    } else {
        return Ok(Vec::new());
    };

    let storages: Vec<String> = records
        .first()
        .map(|r| r.storage_destinations.clone())
        .unwrap_or_default();

    let policies = effective_policies(cfg, database, group, &storages);
    if policies.is_empty() {
        return Ok(Vec::new());
    }

    let to_delete = candidates_for_deletion(&records, &policies);
    let mut deleted = Vec::new();
    for id in to_delete {
        let rec = store.require(&id)?;
        let path = std::path::PathBuf::from(&rec.local_path);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        store.delete(&id)?;
        info!(backup_id = %id, "retention cleanup");
        deleted.push(id);
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::BackupStatus;
    use chrono::TimeZone;

    fn rec(id: &str, days_ago: i64) -> BackupRecord {
        BackupRecord {
            id: id.into(),
            name: format!("{id}.sql.gz"),
            database: Some("samane".into()),
            group_name: None,
            created_at: Utc::now() - Duration::days(days_ago),
            size_bytes: 1,
            checksum: "x".into(),
            local_path: "/tmp/x".into(),
            upload_status: "uploaded".into(),
            storage_destinations: vec![],
            status: BackupStatus::Success,
            duration_ms: 1,
            error: None,
            job_id: None,
        }
    }

    #[test]
    fn keep_last_protects_newest() {
        let records = vec![rec("a", 0), rec("b", 1), rec("c", 2), rec("d", 3)];
        let policies = vec![RetentionPolicy::keep_last(2)];
        let del = candidates_for_deletion(&records, &policies);
        assert_eq!(del, vec!["c".to_string(), "d".to_string()]);
    }

    #[test]
    fn keep_days_protects_recent() {
        let records = vec![rec("a", 1), rec("b", 10), rec("c", 40)];
        let policies = vec![RetentionPolicy::keep_days(14)];
        let del = candidates_for_deletion(&records, &policies);
        assert_eq!(del, vec!["c".to_string()]);
    }

    #[test]
    fn multiple_policies_union_protection() {
        // keep_last 1 would delete b,c — but keep_days 30 protects b
        let records = vec![rec("a", 0), rec("b", 5), rec("c", 60)];
        let policies = vec![
            RetentionPolicy::keep_last(1),
            RetentionPolicy::keep_days(30),
        ];
        let del = candidates_for_deletion(&records, &policies);
        assert_eq!(del, vec!["c".to_string()]);
    }

    #[test]
    fn empty_policies_delete_nothing() {
        let records = vec![rec("a", 0)];
        let del = candidates_for_deletion(&records, &[]);
        assert!(del.is_empty());
    }

    #[test]
    fn fixed_timestamp_keep_days() {
        let old = BackupRecord {
            id: "old".into(),
            name: "old.sql.gz".into(),
            database: Some("db".into()),
            group_name: None,
            created_at: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            size_bytes: 1,
            checksum: "x".into(),
            local_path: "/tmp/x".into(),
            upload_status: "uploaded".into(),
            storage_destinations: vec![],
            status: BackupStatus::Success,
            duration_ms: 1,
            error: None,
            job_id: None,
        };
        let del = candidates_for_deletion(&[old], &[RetentionPolicy::keep_days(7)]);
        assert_eq!(del, vec!["old".to_string()]);
    }
}
