//! Terminal UI helpers — tables, colors, steps.

use colored::Colorize;
use comfy_table::{presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement, Table};

use crate::config::{DatabaseEntry, ScheduleEntry, StorageEntry};
use crate::metadata::BackupRecord;
use crate::util::{format_bytes, relative_time};

pub fn print_header(title: &str) {
    println!("{}", title.bold());
    println!("{}", "────────────────────────────────".dimmed());
}

pub fn step(msg: &str) {
    println!("  {msg}");
}

pub fn success(msg: &str) {
    println!("{} {}", "✓".green().bold(), msg);
}

pub fn warn(msg: &str) {
    println!("{} {}", "!".yellow().bold(), msg);
}

pub fn table_backups(rows: &[BackupRecord]) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            header("ID"),
            header("NAME"),
            header("DATABASE"),
            header("SIZE"),
            header("CREATED"),
            header("STATUS"),
            header("STORAGE"),
        ]);

    for r in rows {
        let id_short = if r.id.len() > 8 { &r.id[..8] } else { &r.id };
        let status = match r.status {
            crate::metadata::BackupStatus::Success => Cell::new("SUCCESS").fg(Color::Green),
            crate::metadata::BackupStatus::Failed => Cell::new("FAILED").fg(Color::Red),
            crate::metadata::BackupStatus::Running => Cell::new("RUNNING").fg(Color::Yellow),
            crate::metadata::BackupStatus::Partial => Cell::new("PARTIAL").fg(Color::Yellow),
            crate::metadata::BackupStatus::Pending => Cell::new("PENDING"),
        };
        table.add_row(vec![
            Cell::new(id_short),
            Cell::new(&r.name),
            Cell::new(r.database.as_deref().unwrap_or("-")),
            Cell::new(format_bytes(r.size_bytes)),
            Cell::new(relative_time(r.created_at)),
            status,
            Cell::new(if r.storage_destinations.is_empty() {
                "local".into()
            } else {
                r.storage_destinations.join(",")
            }),
        ]);
    }
    println!("{table}");
}

pub fn table_databases(rows: &[DatabaseEntry]) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            header("NAME"),
            header("HOST"),
            header("PORT"),
            header("DATABASE"),
            header("USER"),
            header("ENABLED"),
        ]);
    for d in rows {
        table.add_row(vec![
            Cell::new(&d.name),
            Cell::new(&d.host),
            Cell::new(d.port.to_string()),
            Cell::new(&d.database),
            Cell::new(&d.username),
            Cell::new(if d.enabled { "yes" } else { "no" }),
        ]);
    }
    println!("{table}");
}

pub fn table_storages(rows: &[StorageEntry]) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            header("NAME"),
            header("PROVIDER"),
            header("BUCKET"),
            header("REGION"),
            header("ENDPOINT"),
            header("ENABLED"),
        ]);
    for s in rows {
        table.add_row(vec![
            Cell::new(&s.name),
            Cell::new(&s.provider),
            Cell::new(&s.bucket),
            Cell::new(&s.region),
            Cell::new(&s.endpoint),
            Cell::new(if s.enabled { "yes" } else { "no" }),
        ]);
    }
    println!("{table}");
}

pub fn table_schedules(rows: &[ScheduleEntry]) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            header("NAME"),
            header("WHEN"),
            header("TARGET"),
            header("STORAGE"),
            header("ENABLED"),
        ]);
    for s in rows {
        let target = if let Some(ref g) = s.group {
            format!("group:{g}")
        } else {
            s.databases.join(",")
        };
        table.add_row(vec![
            Cell::new(&s.name),
            Cell::new(&s.when),
            Cell::new(target),
            Cell::new(s.storages.join(",")),
            Cell::new(if s.enabled { "yes" } else { "no" }),
        ]);
    }
    println!("{table}");
}

fn header(s: &str) -> Cell {
    Cell::new(s).add_attribute(Attribute::Bold)
}
