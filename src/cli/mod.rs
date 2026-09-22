//! CLI interface for Snapit.

mod ui;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::io::{stdin, Write};

use crate::backup::{delete_backup, run_backup, verify_record, BackupRequest};
use crate::config::paths::Paths;
use crate::config::{
    save_json, AppConfig, BackupGroup, DatabaseEntry, RetentionPolicy, StorageEntry,
};
use crate::install::{install, sync_schedule_units, InstallOptions};
use crate::logging::{self, follow_logs, read_logs};
use crate::metadata::MetadataStore;
use crate::restore::{preview as restore_preview, restore, RestoreOptions};
use crate::retention::apply_retention;
use crate::schedule::{
    add_schedule, remove_schedule, set_schedule_enabled, to_on_calendar, ScheduleSpec,
};
use crate::storage::test_storage;
use crate::util::{format_bytes, format_duration, relative_time};

use ui::{
    print_header, step, success, table_backups, table_databases, table_schedules, table_storages,
    warn,
};

#[derive(Debug, Parser)]
#[command(
    name = "snapit",
    about = "Professional server backup management CLI",
    version,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Show system status overview
    Status,
    /// Backup operations
    Backup {
        #[command(subcommand)]
        action: BackupCmd,
    },
    /// Restore a backup (requires confirmation)
    Restore {
        /// Backup ID
        id: String,
        /// Target database config name
        #[arg(long)]
        target: Option<String>,
        /// Confirm destructive restore
        #[arg(long)]
        yes: bool,
    },
    /// Database configuration
    Database {
        #[command(subcommand)]
        action: DatabaseCmd,
    },
    /// Storage destination management
    Storage {
        #[command(subcommand)]
        action: StorageCmd,
    },
    /// Backup group management
    Group {
        #[command(subcommand)]
        action: GroupCmd,
    },
    /// Schedule management
    Schedule {
        #[command(subcommand)]
        action: ScheduleCmd,
    },
    /// Configuration
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// View logs
    Logs {
        #[arg(long)]
        follow: bool,
        #[arg(long)]
        job: Option<String>,
        #[arg(long, default_value_t = 100)]
        lines: usize,
    },
    /// Apply retention policies
    Retention {
        #[arg(long)]
        database: Option<String>,
        #[arg(long)]
        group: Option<String>,
    },
    /// Install Snapit on this host
    Install {
        #[arg(long)]
        binary: Option<String>,
        #[arg(long)]
        skip_user: bool,
        #[arg(long)]
        with_examples: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum BackupCmd {
    /// Backup a single database
    Database {
        name: String,
        #[arg(long, value_delimiter = ',')]
        storage: Vec<String>,
    },
    /// Backup multiple databases
    Databases {
        names: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        storage: Vec<String>,
    },
    /// Backup a named group
    Group {
        name: String,
        #[arg(long, value_delimiter = ',')]
        storage: Vec<String>,
    },
    /// List backups
    List {
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show backup details
    Show { id: String },
    /// Delete a backup
    Delete {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    /// Verify a backup
    Verify { id: String },
    /// Restore alias
    Restore {
        id: String,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DatabaseCmd {
    List,
    Add {
        name: String,
        #[arg(long)]
        host: String,
        #[arg(long, default_value_t = 5432)]
        port: u16,
        #[arg(long)]
        database: String,
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        keep_last: Option<u32>,
    },
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum StorageCmd {
    List,
    Add {
        name: String,
        #[arg(long)]
        region: String,
        #[arg(long)]
        bucket: String,
        #[arg(long)]
        access_key_id: String,
        #[arg(long)]
        secret_access_key: String,
        #[arg(long)]
        endpoint: String,
        #[arg(long, default_value_t = true)]
        path_style: bool,
    },
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    Test {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum GroupCmd {
    List,
    Add {
        name: String,
        #[arg(long, value_delimiter = ',')]
        databases: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        storages: Vec<String>,
        #[arg(long)]
        keep_last: Option<u32>,
    },
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ScheduleCmd {
    List,
    Add {
        name: String,
        #[arg(long)]
        when: String,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, value_delimiter = ',')]
        databases: Vec<String>,
        #[arg(long, value_delimiter = ',')]
        storages: Vec<String>,
        #[arg(long)]
        keep_last: Option<u32>,
        #[arg(long, default_value_t = true)]
        enabled: bool,
    },
    Remove {
        name: String,
    },
    Enable {
        name: String,
    },
    Disable {
        name: String,
    },
    /// Write/enable systemd timers from schedules.json
    Sync,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    Show,
    Paths,
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Install {
            binary,
            skip_user,
            with_examples,
        } => {
            install(InstallOptions {
                binary_source: binary,
                skip_user,
                with_examples,
            })?;
            return Ok(());
        }
        Commands::Logs { follow, job, lines } => {
            let paths = Paths::resolve();
            if follow {
                follow_logs(&paths.log_dir)?;
            } else {
                for line in read_logs(&paths.log_dir, job.as_deref(), lines)? {
                    println!("{line}");
                }
            }
            return Ok(());
        }
        _ => {}
    }

    let paths = Paths::resolve();
    let _ = paths.ensure_runtime_dirs();
    let _log_guard = logging::init(&paths.log_dir).unwrap_or_else(|_| logging::init_quiet());

    let cfg = AppConfig::load(&paths).with_context(|| {
        format!(
            "failed to load config from {}\nHint: run snapit install or create databases.json / storage.json",
            paths.config_dir.display()
        )
    })?;
    let store = MetadataStore::open(&paths.metadata_db)?;

    match cli.command {
        Commands::Status => cmd_status(&cfg, &store)?,
        Commands::Backup { action } => cmd_backup(&cfg, &store, action).await?,
        Commands::Restore { id, target, yes } => {
            cmd_restore(&cfg, &store, id, target, yes)?;
        }
        Commands::Database { action } => cmd_database(cfg, action)?,
        Commands::Storage { action } => cmd_storage(cfg, action).await?,
        Commands::Group { action } => cmd_group(cfg, action)?,
        Commands::Schedule { action } => cmd_schedule(cfg, action)?,
        Commands::Config { action } => cmd_config(&cfg, action)?,
        Commands::Retention { database, group } => {
            let deleted = apply_retention(&cfg, &store, database.as_deref(), group.as_deref())?;
            if deleted.is_empty() {
                println!("{}", "Nothing to clean up.".dimmed());
            } else {
                success(&format!("Deleted {} backup(s)", deleted.len()));
                for id in deleted {
                    println!("  {id}");
                }
            }
        }
        Commands::Install { .. } | Commands::Logs { .. } => unreachable!(),
    }
    Ok(())
}

fn cmd_status(cfg: &AppConfig, store: &MetadataStore) -> Result<()> {
    print_header("Backup System");
    let last = store.last_backup()?;
    let (last_ago, last_status) = match &last {
        Some(b) => (
            relative_time(b.created_at),
            match &b.status {
                crate::metadata::BackupStatus::Success => "OK SUCCESS".green().to_string(),
                crate::metadata::BackupStatus::Failed => "FAILED".red().to_string(),
                other => format!("{:?}", other).yellow().to_string(),
            },
        ),
        None => ("never".into(), "-".into()),
    };

    println!(
        "{:<16} {}",
        "Databases",
        cfg.databases.databases.len().to_string().bold()
    );
    println!(
        "{:<16} {}",
        "Storage targets",
        cfg.storage.storages.len().to_string().bold()
    );
    println!(
        "{:<16} {}",
        "Schedules",
        cfg.schedules.schedules.len().to_string().bold()
    );
    println!("{:<16} {}", "Backups", store.count()?.to_string().bold());
    println!("{:<16} {}", "Last backup", last_ago);
    println!("{:<16} {}", "Last status", last_status);
    Ok(())
}

async fn cmd_backup(cfg: &AppConfig, store: &MetadataStore, action: BackupCmd) -> Result<()> {
    match action {
        BackupCmd::Database { name, storage } => {
            run_backup_ui(
                cfg,
                store,
                BackupRequest {
                    databases: vec![name],
                    group: None,
                    storages: storage,
                },
            )
            .await?;
        }
        BackupCmd::Databases { names, storage } => {
            if names.is_empty() {
                bail!("provide at least one database name");
            }
            run_backup_ui(
                cfg,
                store,
                BackupRequest {
                    databases: names,
                    group: None,
                    storages: storage,
                },
            )
            .await?;
        }
        BackupCmd::Group { name, storage } => {
            run_backup_ui(
                cfg,
                store,
                BackupRequest {
                    databases: vec![],
                    group: Some(name),
                    storages: storage,
                },
            )
            .await?;
        }
        BackupCmd::List { limit } => {
            let rows = store.list(limit)?;
            table_backups(&rows);
        }
        BackupCmd::Show { id } => {
            let rec = store.require(&id)?;
            print_header("Backup");
            println!("{:<14} {}", "ID", rec.id);
            println!("{:<14} {}", "Name", rec.name);
            println!(
                "{:<14} {}",
                "Database",
                rec.database.as_deref().unwrap_or("-")
            );
            println!(
                "{:<14} {}",
                "Group",
                rec.group_name.as_deref().unwrap_or("-")
            );
            println!("{:<14} {}", "Created", rec.created_at.to_rfc3339());
            println!("{:<14} {}", "Size", format_bytes(rec.size_bytes));
            println!("{:<14} {}", "Checksum", rec.checksum);
            println!("{:<14} {}", "Local path", rec.local_path);
            println!("{:<14} {}", "Upload", rec.upload_status);
            println!("{:<14} {}", "Storage", rec.storage_destinations.join(", "));
            println!("{:<14} {:?}", "Status", rec.status);
            println!(
                "{:<14} {}",
                "Duration",
                format_duration(std::time::Duration::from_millis(rec.duration_ms))
            );
            if let Some(err) = &rec.error {
                println!("{:<14} {}", "Error", err.red());
            }
        }
        BackupCmd::Delete { id, yes } => {
            if !yes {
                warn("This will delete the backup record and local file.");
                if !confirm("Delete backup?")? {
                    bail!("aborted");
                }
            }
            let rec = delete_backup(store, &id, true)?;
            success(&format!("Deleted {}", rec.name));
        }
        BackupCmd::Verify { id } => {
            print_header("Verify");
            step("Checking file and checksum...");
            verify_record(store, &id)?;
            success("Backup verified");
        }
        BackupCmd::Restore { id, target, yes } => {
            cmd_restore(cfg, store, id, target, yes)?;
        }
    }
    Ok(())
}

async fn run_backup_ui(cfg: &AppConfig, store: &MetadataStore, req: BackupRequest) -> Result<()> {
    print_header("Backup System");
    if let Some(ref g) = req.group {
        println!("{:<12} {}", "Group", g.bold());
    } else {
        println!("{:<12} {}", "Database", req.databases.join(", ").bold());
    }
    println!(
        "{:<12} {}",
        "Started",
        chrono::Local::now().format("%H:%M:%S")
    );
    println!();

    step("Dumping database...");
    let outcome = run_backup(cfg, store, req).await?;
    // Steps are inside engine; summarize
    for rec in &outcome.records {
        println!();
        success("Backup completed");
        println!("{:<12} {}", "Name", rec.name);
        println!("{:<12} {}", "Size", format_bytes(rec.size_bytes));
        println!(
            "{:<12} {}",
            "Duration",
            format_duration(std::time::Duration::from_millis(rec.duration_ms))
        );
        let storage = if rec.storage_destinations.is_empty() {
            "local".into()
        } else {
            format!("{} OK", rec.storage_destinations.join(", "))
        };
        println!("{:<12} {}", "Storage", storage);
        println!("{:<12} {}", "ID", rec.id.dimmed());

        // Retention after success
        let _ = apply_retention(
            cfg,
            store,
            rec.database.as_deref(),
            rec.group_name.as_deref(),
        );
    }
    Ok(())
}

fn cmd_restore(
    cfg: &AppConfig,
    store: &MetadataStore,
    id: String,
    target: Option<String>,
    yes: bool,
) -> Result<()> {
    let opts = RestoreOptions {
        backup_id: id,
        target_database: target,
        confirm: true,
        yes,
    };
    let preview = restore_preview(cfg, store, &opts)?;
    println!("{preview}");
    if !yes {
        println!();
        warn("Re-run with --yes to confirm this destructive restore.");
        return Ok(());
    }
    if !confirm("Type yes to proceed with restore")? {
        bail!("aborted");
    }
    restore(cfg, store, &opts)?;
    success("Restore completed");
    Ok(())
}

fn cmd_database(mut cfg: AppConfig, action: DatabaseCmd) -> Result<()> {
    match action {
        DatabaseCmd::List => table_databases(&cfg.databases.databases),
        DatabaseCmd::Add {
            name,
            host,
            port,
            database,
            username,
            password,
            keep_last,
        } => {
            if cfg.find_database(&name).is_some() {
                bail!("database already exists: {name}");
            }
            cfg.databases.databases.push(DatabaseEntry {
                name: name.clone(),
                driver: "postgres".into(),
                host,
                port,
                database,
                username,
                password,
                enabled: true,
                retention: keep_last.map(RetentionPolicy::keep_last),
            });
            cfg.validate()?;
            save_json(&cfg.paths.databases_file(), &cfg.databases)?;
            success(&format!("Added database [{name}]"));
        }
        DatabaseCmd::Remove { name, yes } => {
            if !yes && !confirm(&format!("Remove database [{name}] from config?"))? {
                bail!("aborted");
            }
            let before = cfg.databases.databases.len();
            cfg.databases.databases.retain(|d| d.name != name);
            if cfg.databases.databases.len() == before {
                bail!("database not found: {name}");
            }
            save_json(&cfg.paths.databases_file(), &cfg.databases)?;
            success(&format!("Removed database [{name}]"));
        }
    }
    Ok(())
}

async fn cmd_storage(mut cfg: AppConfig, action: StorageCmd) -> Result<()> {
    match action {
        StorageCmd::List => table_storages(&cfg.storage.storages),
        StorageCmd::Add {
            name,
            region,
            bucket,
            access_key_id,
            secret_access_key,
            endpoint,
            path_style,
        } => {
            if cfg.find_storage(&name).is_some() {
                bail!("storage already exists: {name}");
            }
            cfg.storage.storages.push(StorageEntry {
                name: name.clone(),
                provider: "s3".into(),
                region,
                bucket,
                access_key_id,
                secret_access_key,
                endpoint,
                path_style,
                enabled: true,
                retention: None,
            });
            cfg.validate()?;
            save_json(&cfg.paths.storage_file(), &cfg.storage)?;
            success(&format!("Added storage [{name}]"));
        }
        StorageCmd::Remove { name, yes } => {
            if !yes && !confirm(&format!("Remove storage [{name}]?"))? {
                bail!("aborted");
            }
            let before = cfg.storage.storages.len();
            cfg.storage.storages.retain(|s| s.name != name);
            if cfg.storage.storages.len() == before {
                bail!("storage not found: {name}");
            }
            save_json(&cfg.paths.storage_file(), &cfg.storage)?;
            success(&format!("Removed storage [{name}]"));
        }
        StorageCmd::Test { name } => {
            let entry = cfg
                .find_storage(&name)
                .with_context(|| format!("storage not found: {name}"))?;
            step(&format!("Testing storage [{name}]..."));
            test_storage(entry).await?;
            success(&format!("Storage [{name}] OK"));
        }
    }
    Ok(())
}

fn cmd_group(mut cfg: AppConfig, action: GroupCmd) -> Result<()> {
    match action {
        GroupCmd::List => {
            print_header("Groups");
            for g in &cfg.groups.groups {
                println!(
                    "  {}  {}  -> {}",
                    g.name.bold(),
                    if g.enabled { "on".green() } else { "off".red() },
                    g.databases.join(", ")
                );
            }
            if cfg.groups.groups.is_empty() {
                println!("{}", "(none)".dimmed());
            }
        }
        GroupCmd::Add {
            name,
            databases,
            storages,
            keep_last,
        } => {
            if databases.is_empty() {
                bail!("--databases is required");
            }
            cfg.groups.groups.push(BackupGroup {
                name: name.clone(),
                databases,
                storages,
                retention: keep_last.map(RetentionPolicy::keep_last),
                enabled: true,
            });
            cfg.validate()?;
            save_json(&cfg.paths.groups_file(), &cfg.groups)?;
            success(&format!("Added group [{name}]"));
        }
        GroupCmd::Remove { name, yes } => {
            if !yes && !confirm(&format!("Remove group [{name}]?"))? {
                bail!("aborted");
            }
            let before = cfg.groups.groups.len();
            cfg.groups.groups.retain(|g| g.name != name);
            if cfg.groups.groups.len() == before {
                bail!("group not found: {name}");
            }
            save_json(&cfg.paths.groups_file(), &cfg.groups)?;
            success(&format!("Removed group [{name}]"));
        }
    }
    Ok(())
}

fn cmd_schedule(mut cfg: AppConfig, action: ScheduleCmd) -> Result<()> {
    match action {
        ScheduleCmd::List => table_schedules(&cfg.schedules.schedules),
        ScheduleCmd::Add {
            name,
            when,
            group,
            databases,
            storages,
            keep_last,
            enabled,
        } => {
            let cal = to_on_calendar(&when)?;
            add_schedule(
                &mut cfg,
                ScheduleSpec {
                    name: name.clone(),
                    when,
                    databases,
                    group,
                    storages,
                    retention: keep_last.map(RetentionPolicy::keep_last),
                    enabled,
                },
            )?;
            success(&format!("Added schedule [{name}] (OnCalendar={cal})"));
            println!(
                "{}",
                "Run: snapit schedule sync  (installs systemd timers)".dimmed()
            );
        }
        ScheduleCmd::Remove { name } => {
            remove_schedule(&mut cfg, &name)?;
            success(&format!("Removed schedule [{name}]"));
        }
        ScheduleCmd::Enable { name } => {
            set_schedule_enabled(&mut cfg, &name, true)?;
            success(&format!("Enabled schedule [{name}]"));
        }
        ScheduleCmd::Disable { name } => {
            set_schedule_enabled(&mut cfg, &name, false)?;
            success(&format!("Disabled schedule [{name}]"));
        }
        ScheduleCmd::Sync => {
            sync_schedule_units(&cfg)?;
            success("Systemd schedule timers are synchronized");
        }
    }
    Ok(())
}

fn cmd_config(cfg: &AppConfig, action: ConfigCmd) -> Result<()> {
    match action {
        ConfigCmd::Show => {
            println!("{}", serde_json::to_string_pretty(&cfg.show_safe())?);
        }
        ConfigCmd::Paths => {
            print_header("Paths");
            println!("{:<14} {}", "Config", cfg.paths.config_dir.display());
            println!("{:<14} {}", "Data", cfg.paths.data_dir.display());
            println!("{:<14} {}", "Backups", cfg.paths.backups_dir.display());
            println!("{:<14} {}", "Metadata", cfg.paths.metadata_db.display());
            println!("{:<14} {}", "Logs", cfg.paths.log_dir.display());
        }
    }
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    stdin().read_line(&mut line)?;
    let t = line.trim().to_lowercase();
    Ok(t == "y" || t == "yes")
}
