//! Structured logging to /var/log/backup-system.

use anyhow::Result;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub struct LogGuard {
    _guard: Option<WorkerGuard>,
}

pub fn init(log_dir: &Path) -> Result<LogGuard> {
    std::fs::create_dir_all(log_dir)?;
    let file_appender = tracing_appender::rolling::daily(log_dir, "snapit.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(false),
        )
        .try_init();

    Ok(LogGuard {
        _guard: Some(guard),
    })
}

pub fn init_quiet() -> LogGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false).with_writer(std::io::stderr))
        .try_init();
    LogGuard { _guard: None }
}

pub fn latest_log_file(log_dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(log_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("snapit.log"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files.pop()
}

pub fn read_logs(log_dir: &Path, job_id: Option<&str>, lines: usize) -> Result<Vec<String>> {
    let path = latest_log_file(log_dir);
    let Some(path) = path else {
        return Ok(vec!["(no log files yet)".into()]);
    };
    let file = OpenOptions::new().read(true).open(&path)?;
    let reader = BufReader::new(file);
    let mut all: Vec<String> = reader.lines().map_while(Result::ok).collect();
    if let Some(jid) = job_id {
        all.retain(|l| l.contains(jid));
    }
    let start = all.len().saturating_sub(lines);
    Ok(all[start..].to_vec())
}

pub fn follow_logs(log_dir: &Path) -> Result<()> {
    use std::thread;
    use std::time::Duration;

    let path = latest_log_file(log_dir)
        .ok_or_else(|| anyhow::anyhow!("no log files in {}", log_dir.display()))?;
    let file = OpenOptions::new().read(true).open(&path)?;
    let mut reader = BufReader::new(file);
    // Seek to end
    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            thread::sleep(Duration::from_millis(400));
            continue;
        }
        print!("{buf}");
        let _ = std::io::stdout().flush();
    }
}

/// Ensure a message never contains known secret patterns (defense in depth).
pub fn scrub_log_line(line: &str, secrets: &[&str]) -> String {
    let mut out = line.to_string();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s, "***");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_secrets() {
        let line = scrub_log_line(
            "upload with key=ABCSECRET and pass=xyz",
            &["ABCSECRET", "xyz"],
        );
        assert!(!line.contains("ABCSECRET"));
        assert!(!line.contains("xyz"));
        assert!(line.contains("***"));
    }
}
