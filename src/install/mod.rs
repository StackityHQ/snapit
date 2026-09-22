//! Installation helpers for directories, user, and systemd.

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use crate::config::paths::Paths;
use crate::config::{
    save_json, AppConfig, DatabasesConfig, GroupsConfig, SchedulesConfig, StorageConfig,
};
use crate::schedule::write_systemd_units;

pub struct InstallOptions {
    pub binary_source: Option<String>,
    pub skip_user: bool,
    pub with_examples: bool,
}

pub fn install(opts: InstallOptions) -> Result<()> {
    let paths = Paths::resolve();
    println!("Installing Snapit…");
    println!("  config : {}", paths.config_dir.display());
    println!("  data   : {}", paths.data_dir.display());
    println!("  logs   : {}", paths.log_dir.display());

    create_dirs(&paths)?;
    if !opts.skip_user {
        ensure_user("snapit")?;
    }

    let binary_dest = Path::new("/usr/local/bin/snapit");
    install_binary(opts.binary_source.as_deref(), binary_dest)?;

    // Seed empty/example configs if missing
    if !paths.databases_file().exists() {
        if opts.with_examples {
            write_example_configs(&paths)?;
        } else {
            save_json(&paths.databases_file(), &DatabasesConfig::default())?;
            save_json(&paths.storage_file(), &StorageConfig::default())?;
            save_json(&paths.groups_file(), &GroupsConfig::default())?;
            save_json(&paths.schedules_file(), &SchedulesConfig::default())?;
        }
    }

    secure_config_perms(&paths)?;
    chown_tree(&paths, "snapit")?;

    // Install systemd template units directory for schedules
    let unit_dir = Path::new("/etc/systemd/system");
    if unit_dir.exists() {
        // Placeholder oneshot runner
        let runner = r#"[Unit]
Description=Snapit backup runner
After=network-online.target

[Service]
Type=oneshot
User=snapit
Group=snapit
Environment=SNAPIT_HOME=/var/backupSystem
Environment=SNAPIT_DATA=/var/lib/backup-system
Environment=SNAPIT_LOG=/var/log/backup-system
ExecStart=/usr/local/bin/snapit status
"#;
        fs::write(unit_dir.join("snapit-runner.service"), runner)?;
        let _ = Command::new("systemctl").args(["daemon-reload"]).status();
    }

    println!();
    println!("✓ Snapit installed");
    println!();
    println!("Next steps:");
    println!("  1. snapit config init                  (or place database.json in a project)");
    println!("  2. snapit database add <name> --host … --database … --username … --password …");
    println!("  3. snapit storage add <name> --region … --bucket … --endpoint …");
    println!("  4. snapit database list");
    println!("  5. snapit backup database <name>");
    println!("  6. snapit schedule add …");
    Ok(())
}

fn create_dirs(paths: &Paths) -> Result<()> {
    paths.ensure_runtime_dirs()?;
    Ok(())
}

fn ensure_user(name: &str) -> Result<()> {
    let check = Command::new("id").arg(name).status();
    if let Ok(s) = check {
        if s.success() {
            return Ok(());
        }
    }
    let status = Command::new("useradd")
        .args([
            "--system",
            "--home-dir",
            "/var/lib/backup-system",
            "--shell",
            "/usr/sbin/nologin",
            "--user-group",
            name,
        ])
        .status()
        .context("useradd")?;
    if !status.success() {
        bail!("failed to create user {name}");
    }
    Ok(())
}

fn install_binary(source: Option<&str>, dest: &Path) -> Result<()> {
    let src = if let Some(s) = source {
        Path::new(s).to_path_buf()
    } else {
        std::env::current_exe().context("current_exe")?
    };
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&src, dest).with_context(|| format!("copy {} → {}", src.display(), dest.display()))?;
    fs::set_permissions(dest, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn secure_config_perms(paths: &Paths) -> Result<()> {
    for f in [
        paths.databases_file(),
        paths.storage_file(),
        paths.groups_file(),
        paths.schedules_file(),
    ] {
        if f.exists() {
            fs::set_permissions(&f, fs::Permissions::from_mode(0o600))?;
        }
    }
    fs::set_permissions(&paths.config_dir, fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(&paths.data_dir, fs::Permissions::from_mode(0o750))?;
    Ok(())
}

fn chown_tree(paths: &Paths, user: &str) -> Result<()> {
    for p in [
        paths.config_dir.as_path(),
        paths.data_dir.as_path(),
        paths.log_dir.as_path(),
    ] {
        let _ = Command::new("chown")
            .args(["-R", &format!("{user}:{user}"), &p.display().to_string()])
            .status();
    }
    Ok(())
}

fn write_example_configs(paths: &Paths) -> Result<()> {
    // Write structure only — credentials must be filled by operator
    let db = serde_json::json!({
        "databases": [
            {
                "name": "samane",
                "driver": "postgres",
                "host": "127.0.0.1",
                "port": 5432,
                "database": "postgres",
                "username": "postgres",
                "password": "CHANGE_ME",
                "enabled": true,
                "retention": { "keep_last": 14 }
            }
        ]
    });
    let st = serde_json::json!({
        "storages": [
            {
                "name": "arvan",
                "provider": "s3",
                "region": "ir-thr-at1",
                "bucket": "CHANGE_ME",
                "access_key_id": "CHANGE_ME",
                "secret_access_key": "CHANGE_ME",
                "endpoint": "https://s3.example.com",
                "path_style": true,
                "enabled": true,
                "retention": { "keep_last": 14 }
            }
        ]
    });
    let groups = serde_json::json!({
        "groups": [
            {
                "name": "production",
                "databases": ["samane"],
                "storages": ["arvan"],
                "retention": { "keep_last": 14 },
                "enabled": true
            }
        ]
    });
    let schedules = serde_json::json!({
        "schedules": [
            {
                "name": "nightly",
                "when": "02:00",
                "group": "production",
                "storages": ["arvan"],
                "retention": { "keep_last": 14 },
                "enabled": true
            }
        ]
    });
    fs::write(
        paths.databases_file(),
        serde_json::to_string_pretty(&db)? + "\n",
    )?;
    fs::write(
        paths.storage_file(),
        serde_json::to_string_pretty(&st)? + "\n",
    )?;
    fs::write(
        paths.groups_file(),
        serde_json::to_string_pretty(&groups)? + "\n",
    )?;
    fs::write(
        paths.schedules_file(),
        serde_json::to_string_pretty(&schedules)? + "\n",
    )?;
    Ok(())
}

pub fn sync_schedule_units(cfg: &AppConfig) -> Result<()> {
    let unit_dir = Path::new("/etc/systemd/system");
    if !unit_dir.exists() {
        bail!("systemd unit directory not found");
    }
    let binary = "/usr/local/bin/snapit";
    for sch in &cfg.schedules.schedules {
        write_systemd_units(sch, unit_dir, binary)?;
        if sch.enabled {
            let timer = format!("snapit-schedule-{}.timer", sch.name);
            let _ = Command::new("systemctl")
                .args(["enable", "--now", &timer])
                .status();
        }
    }
    let _ = Command::new("systemctl").args(["daemon-reload"]).status();
    Ok(())
}

/// Detect CPU architecture for release installs.
pub fn detect_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_is_known() {
        let a = detect_arch();
        assert!(!a.is_empty());
    }
}
