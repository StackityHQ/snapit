//! Backup file naming conventions.

use chrono::{DateTime, Local, Utc};

/// Format: `{name}_{YYYY-MM-DD_HH-MM-SS}.sql.gz`
pub fn backup_filename(db_name: &str, at: DateTime<Utc>) -> String {
    let local: DateTime<Local> = DateTime::from(at);
    format!(
        "{}_{}.sql.gz",
        sanitize_name(db_name),
        local.format("%Y-%m-%d_%H-%M-%S")
    )
}

/// Format: `{group}_{YYYY-MM-DD_HH-MM-SS}`
pub fn group_dirname(group_name: &str, at: DateTime<Utc>) -> String {
    let local: DateTime<Local> = DateTime::from(at);
    format!(
        "{}_{}",
        sanitize_name(group_name),
        local.format("%Y-%m-%d_%H-%M-%S")
    )
}

pub fn timestamp_now() -> DateTime<Utc> {
    Utc::now()
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn backup_name_format() {
        let at = Utc.with_ymd_and_hms(2026, 9, 22, 20, 0, 0).unwrap();
        let name = backup_filename("samane", at);
        assert!(name.starts_with("samane_"));
        assert!(name.ends_with(".sql.gz"));
        assert!(name.contains("2026-09-22"));
    }

    #[test]
    fn group_name_format() {
        let at = Utc.with_ymd_and_hms(2026, 9, 22, 20, 0, 0).unwrap();
        let name = group_dirname("production", at);
        assert!(name.starts_with("production_"));
        assert!(name.contains("2026-09-22"));
    }

    #[test]
    fn sanitize_special_chars() {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let name = backup_filename("my db/prod", at);
        assert!(!name.contains(' '));
        assert!(!name.contains('/'));
    }
}
