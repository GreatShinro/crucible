//! Application configuration.
//!
//! - [`Config`] — environment-variable-based startup configuration.
//! - [`AppConfig`] — hot-reloadable runtime configuration.
//! - [`reload`] — [`ConfigManager`] and Axum handlers for live config updates.

pub mod reload;

use serde::{Deserialize, Serialize};
use std::env;

/// Startup configuration loaded from environment variables.
///
/// Read once at process start. For values that change at runtime without a
/// restart, see [`AppConfig`] and [`reload::ConfigManager`].
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub server_port: u16,
    pub environment: String,
    pub log_level: String,
}

impl Config {
    /// Load configuration from environment variables (`.env` file optional).
    pub fn from_env() -> Result<Self, anyhow::Error> {
        dotenvy::dotenv().ok();

        Ok(Config {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:password@localhost:5432/backend".into()),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".into()),
            server_port: env::var("PORT")
                .unwrap_or_else(|_| "3000".into())
                .parse()?,
            environment: env::var("APP_ENV")
                .unwrap_or_else(|_| "development".into()),
            log_level: env::var("LOG_LEVEL")
                .unwrap_or_else(|_| "info".into()),
        })
    }
}

// ---------------------------------------------------------------------------
// AppConfig — hot-reloadable runtime configuration
// ---------------------------------------------------------------------------

/// Server bind configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

/// Database pool configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

/// Redis connection configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedisConfig {
    pub url: String,
}

/// Live application configuration that can be hot-reloaded at runtime.
///
/// All fields have sensible defaults so the application starts without any
/// external configuration source. Use [`reload::ConfigManager`] to update
/// these values without restarting the process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    /// Tracing / log filter directive (e.g. `"backend=debug"`).
    pub log_level: String,
    /// Maximum number of database connections in the pool.
    pub max_connections: u32,
    /// Request timeout in seconds.
    pub request_timeout_secs: u64,
    /// Whether the maintenance mode banner is shown.
    pub maintenance_mode: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
            },
            database: DatabaseConfig {
                url: "postgres://postgres:postgres@localhost:5432/crucible".to_string(),
                max_connections: 10,
            },
            redis: RedisConfig {
                url: "redis://127.0.0.1:6379".to_string(),
            },
            log_level: "backend=debug,tower_http=debug".to_string(),
            max_connections: 10,
            request_timeout_secs: 30,
            maintenance_mode: false,
        }
    }
}
