pub mod api;
pub mod config;
pub mod decrypt;
pub mod offline;
pub mod player;
pub mod sort;

pub use config::{Config, SortOrder};

/// Type alias for an HTTP client suitable for CDN downloads.
pub type CdnClient = reqwest::Client;
