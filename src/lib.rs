//! Snapit — professional server backup management CLI.

pub mod backup;
pub mod config;
pub mod metadata;
pub mod storage;
pub mod util;

pub use config::paths::Paths;
