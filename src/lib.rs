//! Snapit — professional server backup management CLI.

pub mod backup;
pub mod cli;
pub mod config;
pub mod install;
pub mod logging;
pub mod metadata;
pub mod restore;
pub mod retention;
pub mod schedule;
pub mod storage;
pub mod util;

pub use config::discover::{ConfigOrigin, DiscoveryOptions};
pub use config::paths::Paths;
