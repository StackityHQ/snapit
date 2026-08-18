//! Default filesystem paths for Snapit.

use std::env;
use std::path::PathBuf;

/// Resolve runtime paths. Override with SNAPIT_HOME / SNAPIT_DATA / SNAPIT_LOG.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub metadata_db: PathBuf,
    pub lock_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let config_dir = env::var_os("SNAPIT_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/backupSystem"));

        let data_dir = env::var_os("SNAPIT_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/lib/backup-system"));

        let log_dir = env::var_os("SNAPIT_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/var/log/backup-system"));

        let backups_dir = data_dir.join("backups");
        let metadata_db = data_dir.join("metadata.db");
        let lock_dir = data_dir.join("locks");

        Self {
            config_dir,
            data_dir,
            log_dir,
            backups_dir,
            metadata_db,
            lock_dir,
        }
    }

    pub fn databases_file(&self) -> PathBuf {
        self.config_dir.join("databases.json")
    }

    pub fn storage_file(&self) -> PathBuf {
        self.config_dir.join("storage.json")
    }

    pub fn groups_file(&self) -> PathBuf {
        self.config_dir.join("groups.json")
    }

    pub fn schedules_file(&self) -> PathBuf {
        self.config_dir.join("schedules.json")
    }

    pub fn ensure_runtime_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.backups_dir)?;
        std::fs::create_dir_all(&self.lock_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env mutations across tests
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_paths() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: single-threaded under mutex for this test process section
        unsafe {
            env::remove_var("SNAPIT_HOME");
            env::remove_var("SNAPIT_DATA");
            env::remove_var("SNAPIT_LOG");
        }
        let p = Paths::resolve();
        assert_eq!(p.config_dir, PathBuf::from("/var/backupSystem"));
        assert_eq!(p.data_dir, PathBuf::from("/var/lib/backup-system"));
        assert_eq!(p.log_dir, PathBuf::from("/var/log/backup-system"));
        assert!(p.databases_file().ends_with("databases.json"));
    }

    #[test]
    fn override_paths() {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            env::set_var("SNAPIT_HOME", "/tmp/snapit-cfg");
            env::set_var("SNAPIT_DATA", "/tmp/snapit-data");
            env::set_var("SNAPIT_LOG", "/tmp/snapit-log");
        }
        let p = Paths::resolve();
        assert_eq!(p.config_dir, PathBuf::from("/tmp/snapit-cfg"));
        assert_eq!(p.data_dir, PathBuf::from("/tmp/snapit-data"));
        assert_eq!(p.log_dir, PathBuf::from("/tmp/snapit-log"));
        unsafe {
            env::remove_var("SNAPIT_HOME");
            env::remove_var("SNAPIT_DATA");
            env::remove_var("SNAPIT_LOG");
        }
    }
}
