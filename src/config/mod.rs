//! Configuration loading and validation.

pub mod discover;
pub mod paths;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use discover::ConfigOrigin;
use paths::Paths;

/// Top-level application configuration loaded from JSON files.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub databases: DatabasesConfig,
    pub storage: StorageConfig,
    pub groups: GroupsConfig,
    pub schedules: SchedulesConfig,
    pub paths: Paths,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabasesConfig {
    pub databases: Vec<DatabaseEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseEntry {
    pub name: String,
    #[serde(default = "default_driver")]
    pub driver: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub retention: Option<RetentionPolicy>,
}

fn default_driver() -> String {
    "postgres".into()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageConfig {
    pub storages: Vec<StorageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEntry {
    pub name: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub endpoint: String,
    #[serde(default)]
    pub path_style: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub retention: Option<RetentionPolicy>,
}

fn default_provider() -> String {
    "s3".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GroupsConfig {
    pub groups: Vec<BackupGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupGroup {
    pub name: String,
    pub databases: Vec<String>,
    #[serde(default)]
    pub storages: Vec<String>,
    #[serde(default)]
    pub retention: Option<RetentionPolicy>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulesConfig {
    pub schedules: Vec<ScheduleEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub name: String,
    /// Cron-like: "daily", "hourly", or systemd OnCalendar expression
    pub when: String,
    #[serde(default)]
    pub databases: Vec<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub storages: Vec<String>,
    #[serde(default)]
    pub retention: Option<RetentionPolicy>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum RetentionPolicy {
    KeepLast { keep_last: u32 },
    KeepDays { keep_days: u32 },
}

impl RetentionPolicy {
    pub fn keep_last(n: u32) -> Self {
        Self::KeepLast { keep_last: n }
    }

    pub fn keep_days(n: u32) -> Self {
        Self::KeepDays { keep_days: n }
    }
}

impl AppConfig {
    pub fn empty(paths: Paths) -> Self {
        Self {
            databases: DatabasesConfig::default(),
            storage: StorageConfig::default(),
            groups: GroupsConfig::default(),
            schedules: SchedulesConfig::default(),
            paths,
        }
    }

    /// Load configuration from discovered files. Missing files become empty lists.
    pub fn load(paths: &Paths) -> Result<Self> {
        let databases = load_optional_databases(&paths.databases_file())?;
        let storage = load_optional(&paths.storage_file())?.unwrap_or_default();
        let groups = load_optional(&paths.groups_file())?.unwrap_or_default();
        let schedules = load_optional(&paths.schedules_file())?.unwrap_or_default();

        let cfg = Self {
            databases,
            storage,
            groups,
            schedules,
            paths: paths.clone(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn databases_file_exists(&self) -> bool {
        self.paths.databases_file().is_file()
    }

    /// Persist databases to the discovered file, creating it if needed.
    pub fn save_databases(&self) -> Result<()> {
        self.paths.ensure_config_dir()?;
        save_json(&self.paths.databases_file(), &self.databases)
    }

    pub fn save_storage(&self) -> Result<()> {
        self.paths.ensure_config_dir()?;
        save_json(&self.paths.storage_file(), &self.storage)
    }

    pub fn save_groups(&self) -> Result<()> {
        self.paths.ensure_config_dir()?;
        save_json(&self.paths.groups_file(), &self.groups)
    }

    pub fn save_schedules(&self) -> Result<()> {
        self.paths.ensure_config_dir()?;
        save_json(&self.paths.schedules_file(), &self.schedules)
    }

    /// Create empty JSON config files at the discovered location.
    pub fn init_files(&self) -> Result<()> {
        self.paths.ensure_config_dir()?;
        if !self.paths.databases_file().exists() {
            save_json(&self.paths.databases_file(), &self.databases)?;
        }
        if !self.paths.storage_file().exists() {
            save_json(&self.paths.storage_file(), &self.storage)?;
        }
        if !self.paths.groups_file().exists() {
            save_json(&self.paths.groups_file(), &self.groups)?;
        }
        if !self.paths.schedules_file().exists() {
            save_json(&self.paths.schedules_file(), &self.schedules)?;
        }
        Ok(())
    }

    pub fn origin(&self) -> &ConfigOrigin {
        &self.paths.origin
    }

    pub fn validate(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for db in &self.databases.databases {
            if db.name.is_empty() {
                bail!("database entry has empty name");
            }
            if !names.insert(db.name.clone()) {
                bail!("duplicate database name: {}", db.name);
            }
            if db.driver != "postgres" && db.driver != "pgsql" {
                bail!(
                    "database '{}': unsupported driver '{}' (only postgres)",
                    db.name,
                    db.driver
                );
            }
            if db.host.is_empty() || db.database.is_empty() || db.username.is_empty() {
                bail!("database '{}': host/database/username required", db.name);
            }
        }

        let mut storage_names = std::collections::HashSet::new();
        for s in &self.storage.storages {
            if s.name.is_empty() {
                bail!("storage entry has empty name");
            }
            if !storage_names.insert(s.name.clone()) {
                bail!("duplicate storage name: {}", s.name);
            }
            if s.bucket.is_empty() || s.endpoint.is_empty() {
                bail!("storage '{}': bucket and endpoint required", s.name);
            }
        }

        for g in &self.groups.groups {
            if g.name.is_empty() {
                bail!("group has empty name");
            }
            for db in &g.databases {
                if self.find_database(db).is_none() {
                    bail!("group '{}': unknown database '{}'", g.name, db);
                }
            }
            for st in &g.storages {
                if self.find_storage(st).is_none() {
                    bail!("group '{}': unknown storage '{}'", g.name, st);
                }
            }
        }

        for sch in &self.schedules.schedules {
            if sch.name.is_empty() {
                bail!("schedule has empty name");
            }
            if sch.when.is_empty() {
                bail!("schedule '{}': when is required", sch.name);
            }
            if let Some(ref g) = sch.group {
                if self.find_group(g).is_none() {
                    bail!("schedule '{}': unknown group '{}'", sch.name, g);
                }
            }
            for db in &sch.databases {
                if self.find_database(db).is_none() {
                    bail!("schedule '{}': unknown database '{}'", sch.name, db);
                }
            }
        }

        Ok(())
    }

    pub fn find_database(&self, name: &str) -> Option<&DatabaseEntry> {
        self.databases.databases.iter().find(|d| d.name == name)
    }

    pub fn find_storage(&self, name: &str) -> Option<&StorageEntry> {
        self.storage.storages.iter().find(|s| s.name == name)
    }

    pub fn find_group(&self, name: &str) -> Option<&BackupGroup> {
        self.groups.groups.iter().find(|g| g.name == name)
    }

    pub fn enabled_databases(&self) -> impl Iterator<Item = &DatabaseEntry> {
        self.databases.databases.iter().filter(|d| d.enabled)
    }

    pub fn enabled_storages(&self) -> impl Iterator<Item = &StorageEntry> {
        self.storage.storages.iter().filter(|s| s.enabled)
    }

    /// Redacted view for display — never includes secrets.
    pub fn show_safe(&self) -> serde_json::Value {
        let dbs: Vec<_> = self
            .databases
            .databases
            .iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "driver": d.driver,
                    "host": d.host,
                    "port": d.port,
                    "database": d.database,
                    "username": d.username,
                    "password": "***",
                    "enabled": d.enabled,
                    "retention": d.retention,
                })
            })
            .collect();
        let storages: Vec<_> = self
            .storage
            .storages
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "provider": s.provider,
                    "region": s.region,
                    "bucket": s.bucket,
                    "access_key_id": "***",
                    "secret_access_key": "***",
                    "endpoint": s.endpoint,
                    "path_style": s.path_style,
                    "enabled": s.enabled,
                    "retention": s.retention,
                })
            })
            .collect();
        serde_json::json!({
            "origin": self.paths.origin.as_label(),
            "databases_file": self.paths.databases_file(),
            "config_dir": self.paths.config_dir,
            "data_dir": self.paths.data_dir,
            "log_dir": self.paths.log_dir,
            "databases": dbs,
            "storages": storages,
            "groups": self.groups.groups,
            "schedules": self.schedules.schedules,
        })
    }
}

pub fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let value: T =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(value)
}

pub fn load_optional<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<Option<T>> {
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(load_json(path)?))
}

/// Accept `{ "databases": [...] }`, a bare array, or a single database object.
pub fn load_optional_databases(path: &Path) -> Result<DatabasesConfig> {
    if !path.is_file() {
        return Ok(DatabasesConfig::default());
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_databases_json(&raw).with_context(|| format!("parse {}", path.display()))
}

pub fn parse_databases_json(raw: &str) -> Result<DatabasesConfig> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    if value.get("databases").is_some() {
        return Ok(serde_json::from_value(value)?);
    }
    if value.is_array() {
        return Ok(DatabasesConfig {
            databases: serde_json::from_value(value)?,
        });
    }
    if value.is_object() {
        return Ok(DatabasesConfig {
            databases: vec![serde_json::from_value(value)?],
        });
    }
    bail!("database config must be an object or array");
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, format!("{raw}\n"))?;
    fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn sample_db(name: &str, database: &str) -> DatabaseEntry {
        DatabaseEntry {
            name: name.into(),
            driver: "postgres".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            database: database.into(),
            username: "user".into(),
            password: "secret".into(),
            enabled: true,
            retention: Some(RetentionPolicy::keep_last(7)),
        }
    }

    #[test]
    fn parse_databases() {
        let json = r#"{
            "databases": [
                {
                    "name": "samane",
                    "driver": "postgres",
                    "host": "95.38.176.204",
                    "port": 5832,
                    "database": "postgres",
                    "username": "delta",
                    "password": "secret"
                }
            ]
        }"#;
        let cfg: DatabasesConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.databases.len(), 1);
        assert_eq!(cfg.databases[0].name, "samane");
        assert!(cfg.databases[0].enabled);
    }

    #[test]
    fn parse_storage() {
        let json = r#"{
            "storages": [{
                "name": "arvan",
                "provider": "s3",
                "region": "ir-thr-at1",
                "bucket": "test-bucket",
                "access_key_id": "key",
                "secret_access_key": "secret",
                "endpoint": "https://s3.example.com",
                "path_style": true
            }]
        }"#;
        let cfg: StorageConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.storages[0].name, "arvan");
        assert!(cfg.storages[0].path_style);
    }

    fn test_paths(root: PathBuf) -> Paths {
        Paths {
            config_dir: root.clone(),
            data_dir: root.clone(),
            log_dir: root.clone(),
            backups_dir: root.join("backups"),
            metadata_db: root.join("m.db"),
            lock_dir: root.join("locks"),
            databases_path: root.join("database.json"),
            storage_path: root.join("storage.json"),
            groups_path: root.join("groups.json"),
            schedules_path: root.join("schedules.json"),
            origin: ConfigOrigin::Uninitialized,
        }
    }

    #[test]
    fn validate_rejects_duplicate_db() {
        let paths = test_paths(PathBuf::from("/tmp"));
        let cfg = AppConfig {
            databases: DatabasesConfig {
                databases: vec![sample_db("a", "db1"), sample_db("a", "db2")],
            },
            storage: StorageConfig::default(),
            groups: GroupsConfig::default(),
            schedules: SchedulesConfig::default(),
            paths,
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn show_safe_redacts_secrets() {
        let dir = TempDir::new().unwrap();
        let paths = test_paths(dir.path().to_path_buf());
        let cfg = AppConfig {
            databases: DatabasesConfig {
                databases: vec![sample_db("samane", "postgres")],
            },
            storage: StorageConfig {
                storages: vec![StorageEntry {
                    name: "arvan".into(),
                    provider: "s3".into(),
                    region: "ir".into(),
                    bucket: "b".into(),
                    access_key_id: "REALKEY".into(),
                    secret_access_key: "REALSECRET".into(),
                    endpoint: "https://s3.example.com".into(),
                    path_style: true,
                    enabled: true,
                    retention: None,
                }],
            },
            groups: GroupsConfig::default(),
            schedules: SchedulesConfig::default(),
            paths,
        };
        let safe = cfg.show_safe().to_string();
        assert!(!safe.contains("REALKEY"));
        assert!(!safe.contains("REALSECRET"));
        assert!(safe.contains("***"));
    }

    #[test]
    fn retention_serde() {
        let p = RetentionPolicy::keep_last(14);
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("keep_last"));
        let back: RetentionPolicy = serde_json::from_str(&j).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn parse_single_database_object() {
        let json = r#"{
            "name": "app",
            "host": "127.0.0.1",
            "port": 5432,
            "database": "app",
            "username": "app",
            "password": "secret"
        }"#;
        let cfg = parse_databases_json(json).unwrap();
        assert_eq!(cfg.databases.len(), 1);
        assert_eq!(cfg.databases[0].name, "app");
    }

    #[test]
    fn parse_bare_database_array() {
        let json = r#"[{
            "name": "app",
            "host": "127.0.0.1",
            "port": 5432,
            "database": "app",
            "username": "app",
            "password": "secret"
        }]"#;
        let cfg = parse_databases_json(json).unwrap();
        assert_eq!(cfg.databases.len(), 1);
    }

    #[test]
    fn load_missing_files_is_empty() {
        let dir = TempDir::new().unwrap();
        let paths = test_paths(dir.path().to_path_buf());
        let cfg = AppConfig::load(&paths).unwrap();
        assert!(cfg.databases.databases.is_empty());
        assert!(cfg.storage.storages.is_empty());
    }
}
