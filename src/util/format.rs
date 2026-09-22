//! Human-readable formatting helpers.

use chrono::{DateTime, Utc};
use std::time::Duration;

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else if secs < 3600.0 {
        format!("{:.1}m", secs / 60.0)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}

pub fn relative_time(at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let diff = now.signed_duration_since(at);
    let secs = diff.num_seconds().abs();
    let suffix = if diff.num_seconds() >= 0 {
        "ago"
    } else {
        "from now"
    };
    if secs < 60 {
        format!("{secs} seconds {suffix}")
    } else if secs < 3600 {
        format!("{} minutes {suffix}", secs / 60)
    } else if secs < 86400 {
        format!("{} hours {suffix}", secs / 3600)
    } else {
        format!("{} days {suffix}", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_format() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert!(format_bytes(1024 * 1024).contains("MB"));
    }

    #[test]
    fn duration_format() {
        assert_eq!(format_duration(Duration::from_millis(42800)), "42.8s");
    }
}
