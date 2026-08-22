//! Safe process execution helpers (no shell).

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Run a binary with args and optional env. Never uses a shell.
pub fn run_command(
    program: &str,
    args: &[&str],
    env: &HashMap<String, String>,
    cwd: Option<&Path>,
) -> Result<Output> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(env);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd
        .output()
        .with_context(|| format!("failed to execute {program}"))?;
    Ok(output)
}

pub fn require_success(program: &str, output: &Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Scrub common password patterns from error output
    let scrubbed = scrub_secrets(&stderr);
    bail!(
        "{program} failed (exit {:?}): {scrubbed}",
        output.status.code()
    );
}

fn scrub_secrets(s: &str) -> String {
    let mut out = s.to_string();
    for key in ["PGPASSWORD", "AWS_SECRET_ACCESS_KEY", "AWS_ACCESS_KEY_ID"] {
        let mut search_from = 0;
        let needle = format!("{key}=");
        while let Some(rel) = out[search_from..].find(&needle) {
            let idx = search_from + rel;
            let rest_start = idx + needle.len();
            let rest = &out[rest_start..];
            let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
            out.replace_range(rest_start..rest_start + end, "***");
            search_from = rest_start + 3;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_echo() {
        let out = run_command("echo", &["hello"], &HashMap::new(), None).unwrap();
        require_success("echo", &out).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[test]
    fn scrub() {
        let s = scrub_secrets("error PGPASSWORD=hunter2 failed");
        assert!(!s.contains("hunter2"));
        assert!(s.contains("PGPASSWORD=***"));
    }
}
