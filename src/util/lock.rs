//! Exclusive job locking to prevent duplicate scheduled runs.

use anyhow::{bail, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Advisory lock held for the lifetime of a backup job.
pub struct JobLock {
    path: PathBuf,
    _file: File,
}

impl JobLock {
    pub fn acquire(lock_dir: &Path, key: &str) -> Result<Self> {
        std::fs::create_dir_all(lock_dir)?;
        let safe: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = lock_dir.join(format!("{safe}.lock"));

        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);

        let mut file = opts.open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                bail!(
                    "another snapit job is already running for '{key}' (lock: {})",
                    path.display()
                );
            }
        }

        writeln!(file, "pid={} key={key}", std::process::id())?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for JobLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn lock_exclusive() {
        let dir = TempDir::new().unwrap();
        let a = JobLock::acquire(dir.path(), "job1").unwrap();
        let b = JobLock::acquire(dir.path(), "job1");
        assert!(b.is_err());
        drop(a);
        let c = JobLock::acquire(dir.path(), "job1");
        assert!(c.is_ok());
    }
}
