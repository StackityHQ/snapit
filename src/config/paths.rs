//! Default filesystem paths for Snapit.

use std::path::PathBuf;

use super::discover::{discover, ConfigOrigin, DiscoveredConfig, DiscoveryOptions};

/// Resolve runtime paths. Discovery prefers a project-local `database.json`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub metadata_db: PathBuf,
    pub lock_dir: PathBuf,
    pub databases_path: PathBuf,
    pub storage_path: PathBuf,
    pub groups_path: PathBuf,
    pub schedules_path: PathBuf,
    pub origin: ConfigOrigin,
}

impl Paths {
    pub fn resolve() -> Self {
        Self::resolve_with(&DiscoveryOptions::default())
    }

    pub fn resolve_with(opts: &DiscoveryOptions) -> Self {
        Self::from_discovered(discover(opts))
    }

    pub fn from_discovered(found: DiscoveredConfig) -> Self {
        let backups_dir = found.data_dir.join("backups");
        let metadata_db = found.data_dir.join("metadata.db");
        let lock_dir = found.data_dir.join("locks");

        Self {
            config_dir: found.config_dir,
            data_dir: found.data_dir,
            log_dir: found.log_dir,
            backups_dir,
            metadata_db,
            lock_dir,
            databases_path: found.databases_file,
            storage_path: found.storage_file,
            groups_path: found.groups_file,
            schedules_path: found.schedules_file,
            origin: found.origin,
        }
    }

    pub fn databases_file(&self) -> PathBuf {
        self.databases_path.clone()
    }

    pub fn storage_file(&self) -> PathBuf {
        self.storage_path.clone()
    }

    pub fn groups_file(&self) -> PathBuf {
        self.groups_path.clone()
    }

    pub fn schedules_file(&self) -> PathBuf {
        self.schedules_path.clone()
    }

    pub fn ensure_runtime_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.backups_dir)?;
        std::fs::create_dir_all(&self.lock_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }

    pub fn ensure_config_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::discover::DiscoveryOptions;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn default_paths_without_project_file() {
        let dir = TempDir::new().unwrap();
        let p = Paths::resolve_with(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            prefer_global: true,
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(p.config_dir, PathBuf::from("/var/backupSystem"));
        assert_eq!(p.data_dir, PathBuf::from("/var/lib/backup-system"));
        assert_eq!(p.log_dir, PathBuf::from("/var/log/backup-system"));
        assert!(p.databases_file().ends_with("databases.json"));
    }

    #[test]
    fn override_paths_via_config_dir() {
        let dir = TempDir::new().unwrap();
        let p = Paths::resolve_with(&DiscoveryOptions {
            config_dir: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(p.config_dir, dir.path());
        assert!(p.databases_file().starts_with(dir.path()));
    }

    #[test]
    fn project_database_json_becomes_config_root() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("database.json"), "{\"databases\":[]}\n").unwrap();
        let p = Paths::resolve_with(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(p.origin, ConfigOrigin::ProjectLocal);
        assert_eq!(p.databases_file(), dir.path().join("database.json"));
        assert_eq!(p.data_dir, dir.path().join(".snapit"));
    }
}
