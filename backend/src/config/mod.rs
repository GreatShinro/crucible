//! Application configuration.

pub mod reload;

use serde::Deserialize;
use std::env;

/// Environment-based application configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub server_port: u16,
    pub environment: String,
    pub log_level: String,
}

impl Config {
    /// Loads configuration from environment variables.
    pub fn from_env() -> Result<Self, anyhow::Error> {
        dotenvy::dotenv().ok();

        Ok(Config {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:password@localhost:5432/backend".into()),
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into()),
            server_port: env::var("PORT").unwrap_or_else(|_| "3000".into()).parse()?,
            environment: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into()),
        })
    }
}
