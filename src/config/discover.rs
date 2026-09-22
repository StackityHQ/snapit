//! Discover database configuration from the project directory, env, or global home.
//!
//! `databases.json` under `/var/backupSystem` is only one possible source.
//! Snapit also accepts a project-local `database.json` (or `databases.json`)
//! and can create that file through the CLI.

use std::env;
use std::path::{Path, PathBuf};

/// How the active configuration location was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOrigin {
    /// `--config` / `SNAPIT_CONFIG` / `SNAPIT_DATABASES`.
    Explicit,
    /// Found `database.json` or `databases.json` by walking from the working directory.
    ProjectLocal,
    /// `SNAPIT_HOME` directory.
    SnapitHome,
    /// Default global directory (`/var/backupSystem`) when that file already exists.
    Global,
    /// Nothing on disk yet; writes will create the pending file.
    Uninitialized,
}

impl ConfigOrigin {
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::ProjectLocal => "project-local",
            Self::SnapitHome => "SNAPIT_HOME",
            Self::Global => "global",
            Self::Uninitialized => "uninitialized",
        }
    }
}

/// Optional overrides supplied by the CLI or tests.
#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// Explicit database config file (`--config` / `SNAPIT_CONFIG`).
    pub config_file: Option<PathBuf>,
    /// Explicit config directory (`--config-dir`).
    pub config_dir: Option<PathBuf>,
    /// Prefer the global `/var/backupSystem` location when creating files.
    pub prefer_global: bool,
    /// Working directory used for project-local discovery. Defaults to cwd.
    pub cwd: Option<PathBuf>,
    /// Read `SNAPIT_*` process environment. Disable in unit tests.
    pub use_process_env: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            config_file: None,
            config_dir: None,
            prefer_global: false,
            cwd: None,
            use_process_env: true,
        }
    }
}

/// Filenames accepted for database configuration, in preference order.
pub const DATABASE_FILENAMES: &[&str] = &["database.json", "databases.json"];

const CONFIG_SUBDIRS: &[&str] = &["", "snapit", ".snapit"];

/// Markers that the current directory is an application project.
const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "composer.json",
    "artisan",
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "Gemfile",
    "pom.xml",
];

#[derive(Debug, Clone)]
pub struct DiscoveredConfig {
    pub origin: ConfigOrigin,
    /// Directory that owns config JSON files.
    pub config_dir: PathBuf,
    /// Database file that exists, or the path that will be created on write.
    pub databases_file: PathBuf,
    pub storage_file: PathBuf,
    pub groups_file: PathBuf,
    pub schedules_file: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl DiscoveredConfig {
    pub fn sibling(&self, name: &str) -> PathBuf {
        self.config_dir.join(name)
    }
}

/// Resolve configuration locations without requiring any file to exist.
pub fn discover(opts: &DiscoveryOptions) -> DiscoveredConfig {
    let cwd = opts
        .cwd
        .clone()
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let snapit_home = opts.config_dir.clone().or_else(|| {
        if opts.use_process_env {
            env::var_os("SNAPIT_HOME").map(PathBuf::from)
        } else {
            None
        }
    });

    let explicit_file = opts.config_file.clone().or_else(|| {
        if !opts.use_process_env {
            return None;
        }
        env::var_os("SNAPIT_CONFIG")
            .or_else(|| env::var_os("SNAPIT_DATABASES"))
            .map(PathBuf::from)
    });

    if let Some(file) = explicit_file {
        let config_dir = file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.clone());
        return finish(
            ConfigOrigin::Explicit,
            config_dir,
            file,
            snapit_home.as_deref(),
            opts.prefer_global,
            opts.use_process_env,
        );
    }

    if let Some(dir) = opts.config_dir.as_ref() {
        let file = first_existing_db_file(dir).unwrap_or_else(|| dir.join("databases.json"));
        let origin = if file.exists() {
            ConfigOrigin::SnapitHome
        } else {
            ConfigOrigin::Uninitialized
        };
        return finish(
            origin,
            dir.clone(),
            file,
            Some(dir.as_path()),
            opts.prefer_global,
            opts.use_process_env,
        );
    }

    if !opts.prefer_global {
        if let Some(file) = find_project_database_file(&cwd) {
            let config_dir = file.parent().map(Path::to_path_buf).unwrap_or(cwd.clone());
            return finish(
                ConfigOrigin::ProjectLocal,
                config_dir,
                file,
                snapit_home.as_deref(),
                false,
                opts.use_process_env,
            );
        }
    }

    if let Some(dir) = snapit_home.as_ref() {
        if let Some(file) = first_existing_db_file(dir) {
            return finish(
                ConfigOrigin::SnapitHome,
                dir.clone(),
                file,
                Some(dir.as_path()),
                opts.prefer_global,
                opts.use_process_env,
            );
        }
    }

    let global = PathBuf::from("/var/backupSystem");
    if let Some(file) = first_existing_db_file(&global) {
        return finish(
            ConfigOrigin::Global,
            global,
            file,
            snapit_home.as_deref(),
            true,
            opts.use_process_env,
        );
    }

    // Nothing exists yet — decide where a new file should be created.
    if opts.prefer_global {
        let dir = snapit_home.clone().unwrap_or(global);
        let file = dir.join("databases.json");
        return finish(
            ConfigOrigin::Uninitialized,
            dir,
            file,
            snapit_home.as_deref(),
            true,
            opts.use_process_env,
        );
    }

    if let Some(dir) = snapit_home {
        let file = first_existing_db_file(&dir).unwrap_or_else(|| dir.join("databases.json"));
        return finish(
            ConfigOrigin::Uninitialized,
            dir.clone(),
            file,
            Some(dir.as_path()),
            false,
            opts.use_process_env,
        );
    }

    if looks_like_project(&cwd) {
        let file = cwd.join("database.json");
        return finish(
            ConfigOrigin::Uninitialized,
            cwd,
            file,
            None,
            false,
            opts.use_process_env,
        );
    }

    finish(
        ConfigOrigin::Uninitialized,
        global.clone(),
        global.join("databases.json"),
        None,
        true,
        opts.use_process_env,
    )
}

fn finish(
    origin: ConfigOrigin,
    config_dir: PathBuf,
    databases_file: PathBuf,
    snapit_home: Option<&Path>,
    global_data: bool,
    use_process_env: bool,
) -> DiscoveredConfig {
    let (data_dir, log_dir) = resolve_runtime_dirs(
        &config_dir,
        snapit_home,
        global_data || origin == ConfigOrigin::Global,
        use_process_env,
    );

    let storage_file = first_existing(&config_dir, &["storage.json"])
        .unwrap_or_else(|| config_dir.join("storage.json"));
    let groups_file = first_existing(&config_dir, &["groups.json"])
        .unwrap_or_else(|| config_dir.join("groups.json"));
    let schedules_file = first_existing(&config_dir, &["schedules.json"])
        .unwrap_or_else(|| config_dir.join("schedules.json"));

    DiscoveredConfig {
        origin,
        config_dir,
        databases_file,
        storage_file,
        groups_file,
        schedules_file,
        data_dir,
        log_dir,
    }
}

fn resolve_runtime_dirs(
    config_dir: &Path,
    snapit_home: Option<&Path>,
    use_system_defaults: bool,
    use_process_env: bool,
) -> (PathBuf, PathBuf) {
    let data_override = if use_process_env {
        env::var_os("SNAPIT_DATA").map(PathBuf::from)
    } else {
        None
    };
    let log_override = if use_process_env {
        env::var_os("SNAPIT_LOG").map(PathBuf::from)
    } else {
        None
    };

    let data_dir = data_override.unwrap_or_else(|| {
        if use_system_defaults && snapit_home.is_none() {
            PathBuf::from("/var/lib/backup-system")
        } else if let Some(home) = snapit_home {
            // SNAPIT_HOME is a dedicated config root; keep data on the system
            // path unless the home itself looks project-local.
            if home == Path::new("/var/backupSystem") {
                PathBuf::from("/var/lib/backup-system")
            } else {
                home.join("data")
            }
        } else {
            config_dir.join(".snapit")
        }
    });

    let log_dir = log_override.unwrap_or_else(|| {
        if data_dir == Path::new("/var/lib/backup-system")
            || (use_system_defaults && snapit_home.is_none())
        {
            PathBuf::from("/var/log/backup-system")
        } else {
            data_dir.join("logs")
        }
    });

    (data_dir, log_dir)
}

/// Walk from `start` toward the filesystem root (stopping at a git root) and
/// return the first database config file found.
pub fn find_project_database_file(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if let Some(found) = first_existing_db_file(&dir) {
            return Some(found);
        }
        let is_git_root = dir.join(".git").exists();
        if is_git_root {
            return None;
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn first_existing_db_file(dir: &Path) -> Option<PathBuf> {
    for sub in CONFIG_SUBDIRS {
        let base = if sub.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(sub)
        };
        for name in DATABASE_FILENAMES {
            let candidate = base.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn looks_like_project(dir: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| dir.join(m).exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn prefers_project_database_json_over_global() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });

        assert_eq!(found.origin, ConfigOrigin::ProjectLocal);
        assert_eq!(found.databases_file, dir.path().join("database.json"));
        assert_eq!(found.config_dir, dir.path());
    }

    #[test]
    fn accepts_databases_json_in_project() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("databases.json"), r#"{"databases":[]}"#);

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });

        assert_eq!(found.origin, ConfigOrigin::ProjectLocal);
        assert_eq!(found.databases_file, dir.path().join("databases.json"));
    }

    #[test]
    fn prefers_singular_database_json_when_both_exist() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);
        write(&dir.path().join("databases.json"), r#"{"databases":[]}"#);

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });

        assert_eq!(found.databases_file, dir.path().join("database.json"));
    }

    #[test]
    fn walks_up_to_git_root() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join(".git"), "");
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);
        let nested = dir.path().join("app").join("http");
        fs::create_dir_all(&nested).unwrap();

        let found = find_project_database_file(&nested).unwrap();
        assert_eq!(found, dir.path().join("database.json"));
    }

    #[test]
    fn does_not_walk_above_git_root() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);
        let repo = dir.path().join("laravel-test");
        fs::create_dir_all(&repo).unwrap();
        write(&repo.join(".git"), "");

        assert!(find_project_database_file(&repo).is_none());
    }

    #[test]
    fn finds_config_in_snapit_subdir() {
        let dir = TempDir::new().unwrap();
        write(
            &dir.path().join(".snapit").join("databases.json"),
            r#"{"databases":[]}"#,
        );

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(found.origin, ConfigOrigin::ProjectLocal);
        assert!(found.databases_file.ends_with(".snapit/databases.json"));
    }

    #[test]
    fn explicit_file_wins() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("custom.json");
        write(&file, r#"{"databases":[]}"#);
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);

        let found = discover(&DiscoveryOptions {
            config_file: Some(file.clone()),
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(found.origin, ConfigOrigin::Explicit);
        assert_eq!(found.databases_file, file);
    }

    #[test]
    fn uninitialized_project_writes_database_json() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("composer.json"), "{}");

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(found.origin, ConfigOrigin::Uninitialized);
        assert_eq!(found.databases_file, dir.path().join("database.json"));
    }

    #[test]
    fn prefer_global_skips_project_file() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            prefer_global: true,
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_ne!(found.databases_file, dir.path().join("database.json"));
        assert!(found.databases_file.ends_with("databases.json"));
    }

    #[test]
    fn project_local_uses_dot_snapit_for_data() {
        let dir = TempDir::new().unwrap();
        write(&dir.path().join("database.json"), r#"{"databases":[]}"#);

        let found = discover(&DiscoveryOptions {
            cwd: Some(dir.path().to_path_buf()),
            use_process_env: false,
            ..DiscoveryOptions::default()
        });
        assert_eq!(found.data_dir, dir.path().join(".snapit"));
        assert_eq!(found.log_dir, dir.path().join(".snapit").join("logs"));
    }
}
