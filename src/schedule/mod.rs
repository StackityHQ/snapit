//! Schedule management and systemd timer generation.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::{save_json, AppConfig, RetentionPolicy, ScheduleEntry, SchedulesConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleSpec {
    pub name: String,
    pub when: String,
    pub databases: Vec<String>,
    pub group: Option<String>,
    pub storages: Vec<String>,
    pub retention: Option<RetentionPolicy>,
    pub enabled: bool,
}

/// Convert human-friendly when → systemd OnCalendar.
pub fn to_on_calendar(when: &str) -> Result<String> {
    let w = when.trim().to_lowercase();
    let cal = match w.as_str() {
        "hourly" => "*-*-* *:00:00".to_string(),
        "daily" | "every day" | "everyday" => "*-*-* 02:00:00".to_string(),
        "daily-02:00" | "every day at 02:00" => "*-*-* 02:00:00".to_string(),
        "weekly" => "Mon *-*-* 02:00:00".to_string(),
        "monthly" => "*-*-01 02:00:00".to_string(),
        other if other.starts_with("*-*-") || other.contains(" *-*-*") || other.contains(':') => {
            // Assume already OnCalendar or time-like "HH:MM"
            if let Some((h, m)) = parse_hhmm(other) {
                format!("*-*-* {h:02}:{m:02}:00")
            } else {
                when.to_string()
            }
        }
        other => {
            if let Some((h, m)) = parse_hhmm(other) {
                format!("*-*-* {h:02}:{m:02}:00")
            } else {
                bail!("unsupported schedule when='{when}'; use daily, hourly, HH:MM, or OnCalendar")
            }
        }
    };
    Ok(cal)
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let parts: Vec<_> = s.split(':').collect();
    if parts.len() == 2 {
        let h: u32 = parts[0].parse().ok()?;
        let m: u32 = parts[1].parse().ok()?;
        if h < 24 && m < 60 {
            return Some((h, m));
        }
    }
    None
}

pub fn build_snapit_args(sch: &ScheduleEntry) -> Vec<String> {
    let mut args = vec!["backup".into()];
    if let Some(ref g) = sch.group {
        args.push("group".into());
        args.push(g.clone());
    } else if sch.databases.len() == 1 {
        args.push("database".into());
        args.push(sch.databases[0].clone());
    } else {
        args.push("databases".into());
        args.extend(sch.databases.clone());
    }
    if !sch.storages.is_empty() {
        args.push("--storage".into());
        args.push(sch.storages.join(","));
    }
    args
}

pub fn render_service_unit(schedule_name: &str, binary: &str, args: &[String]) -> String {
    let exec = format!("{} {}", binary, args.join(" "));
    format!(
        r#"[Unit]
Description=Snapit scheduled backup ({schedule_name})
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
User=snapit
Group=snapit
ExecStart={exec}
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
# Prevent overlapping runs of the same unit
Environment=SNAPIT_HOME=/var/backupSystem
Environment=SNAPIT_DATA=/var/lib/backup-system
Environment=SNAPIT_LOG=/var/log/backup-system

[Install]
WantedBy=multi-user.target
"#
    )
}

pub fn render_timer_unit(schedule_name: &str, on_calendar: &str, enabled: bool) -> String {
    let _ = enabled;
    format!(
        r#"[Unit]
Description=Snapit timer for schedule '{schedule_name}'

[Timer]
OnCalendar={on_calendar}
Persistent=true
Unit=snapit-schedule@{schedule_name}.service
AccuracySec=1min

[Install]
WantedBy=timers.target
"#
    )
}

pub fn write_systemd_units(sch: &ScheduleEntry, unit_dir: &Path, binary: &str) -> Result<()> {
    fs::create_dir_all(unit_dir)?;
    let args = build_snapit_args(sch);
    let cal = to_on_calendar(&sch.when)?;
    let service = render_service_unit(&sch.name, binary, &args);
    let timer = render_timer_unit(&sch.name, &cal, sch.enabled).replace(
        &format!("Unit=snapit-schedule@{}.service", sch.name),
        &format!("Unit=snapit-schedule-{}.service", sch.name),
    );

    let svc_path = unit_dir.join(format!("snapit-schedule-{}.service", sch.name));
    let tmr_path = unit_dir.join(format!("snapit-schedule-{}.timer", sch.name));
    fs::write(&svc_path, service)?;
    fs::write(&tmr_path, timer)?;
    Ok(())
}

pub fn add_schedule(cfg: &mut AppConfig, spec: ScheduleSpec) -> Result<()> {
    if cfg.schedules.schedules.iter().any(|s| s.name == spec.name) {
        bail!("schedule already exists: {}", spec.name);
    }
    // Validate when
    let _ = to_on_calendar(&spec.when)?;
    cfg.schedules.schedules.push(ScheduleEntry {
        name: spec.name,
        when: spec.when,
        databases: spec.databases,
        group: spec.group,
        storages: spec.storages,
        retention: spec.retention,
        enabled: spec.enabled,
    });
    cfg.validate()?;
    save_json(&cfg.paths.schedules_file(), &cfg.schedules)?;
    Ok(())
}

pub fn remove_schedule(cfg: &mut AppConfig, name: &str) -> Result<()> {
    let before = cfg.schedules.schedules.len();
    cfg.schedules.schedules.retain(|s| s.name != name);
    if cfg.schedules.schedules.len() == before {
        bail!("schedule not found: {name}");
    }
    save_json(&cfg.paths.schedules_file(), &cfg.schedules)?;
    Ok(())
}

pub fn set_schedule_enabled(cfg: &mut AppConfig, name: &str, enabled: bool) -> Result<()> {
    let sch = cfg
        .schedules
        .schedules
        .iter_mut()
        .find(|s| s.name == name)
        .with_context(|| format!("schedule not found: {name}"))?;
    sch.enabled = enabled;
    let schedules = SchedulesConfig {
        schedules: cfg.schedules.schedules.clone(),
    };
    save_json(&cfg.paths.schedules_file(), &schedules)?;
    Ok(())
}

pub fn try_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl").args(args).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => bail!("systemctl {:?} failed with {}", args, s),
        Err(e) => bail!("systemctl not available: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_calendar_daily() {
        assert_eq!(to_on_calendar("daily").unwrap(), "*-*-* 02:00:00");
        assert_eq!(to_on_calendar("03:30").unwrap(), "*-*-* 03:30:00");
        assert_eq!(to_on_calendar("hourly").unwrap(), "*-*-* *:00:00");
    }

    #[test]
    fn build_args_group() {
        let sch = ScheduleEntry {
            name: "nightly".into(),
            when: "daily".into(),
            databases: vec![],
            group: Some("production".into()),
            storages: vec!["arvan".into()],
            retention: Some(RetentionPolicy::keep_last(14)),
            enabled: true,
        };
        let args = build_snapit_args(&sch);
        assert_eq!(args[0], "backup");
        assert_eq!(args[1], "group");
        assert_eq!(args[2], "production");
        assert!(args.contains(&"--storage".into()));
    }

    #[test]
    fn render_units_contain_name() {
        let svc = render_service_unit(
            "nightly",
            "/usr/local/bin/snapit",
            &["backup".into(), "group".into(), "production".into()],
        );
        assert!(svc.contains("snapit backup group production"));
        assert!(svc.contains("User=snapit"));
        let tmr = render_timer_unit("nightly", "*-*-* 02:00:00", true);
        assert!(tmr.contains("OnCalendar=*-*-* 02:00:00"));
    }
}
