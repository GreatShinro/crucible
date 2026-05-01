//! Dashboard data API handler.
//!
//! Provides a single `GET /api/dashboard` endpoint that aggregates system
//! metrics, active recovery tasks, and active alerts into one response.
//! Results are cached in Redis for [`CACHE_TTL_SECS`] seconds to reduce
//! load on downstream services.
//!
//! # Example
//! ```rust,no_run
//! use std::sync::Arc;
//! use axum::{Router, routing::get};
//! use backend::api::handlers::dashboard::{DashboardState, get_dashboard};
//!
//! # async fn example() {
//! // state is constructed with your real service instances
//! # }
//! ```

use axum::{extract::State, response::IntoResponse, Json};
use redis::{AsyncCommands, Client as RedisClient};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tracing::{debug, error, warn};

use crate::services::{
    error_recovery::{ErrorManager, RecoveryTask},
    log_alerts::{Alert, AlertManager},
    sys_metrics::{MetricsExporter, SystemMetrics},
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CACHE_KEY: &str = "dashboard:summary";
const CACHE_TTL_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur while building the dashboard response.
#[derive(Debug, Error)]
pub enum DashboardError {
    /// A Redis error occurred.
    #[error("Cache error: {0}")]
    Cache(#[from] redis::RedisError),

    /// JSON serialization/deserialization failed.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl IntoResponse for DashboardError {
    fn into_response(self) -> axum::response::Response {
        error!(error = %self, "Dashboard handler error");
        let body = serde_json::json!({ "error": self.to_string() });
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Aggregated dashboard data returned by `GET /api/dashboard`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    /// Current system metrics snapshot.
    pub metrics: SystemMetrics,
    /// Recovery tasks that are currently active.
    pub active_recovery_tasks: Vec<RecoveryTask>,
    /// Alerts that have fired and not yet been resolved.
    pub active_alerts: Vec<Alert>,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Shared application state for the dashboard handler.
pub struct DashboardState {
    pub metrics_exporter: Arc<MetricsExporter>,
    pub error_manager: Arc<ErrorManager>,
    pub alert_manager: Arc<AlertManager>,
    pub redis: RedisClient,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /api/dashboard` — return aggregated dashboard data.
///
/// Attempts to serve a cached response from Redis. On cache miss (or cache
/// error) the data is assembled from the live services and the cache is
/// populated before responding.
#[tracing::instrument(skip(state))]
pub async fn get_dashboard(
    State(state): State<Arc<DashboardState>>,
) -> Result<impl IntoResponse, DashboardError> {
    // --- try cache ---
    match try_cache_get(&state.redis).await {
        Ok(Some(cached)) => {
            debug!("Dashboard cache hit");
            return Ok(Json(cached));
        }
        Ok(None) => debug!("Dashboard cache miss"),
        Err(e) => warn!(error = %e, "Dashboard cache read failed; falling back to live data"),
    }

    // --- assemble live data ---
    let (metrics, active_recovery_tasks, active_alerts) = tokio::join!(
        state.metrics_exporter.get_metrics(),
        state.error_manager.get_active_tasks(),
        state.alert_manager.get_active_alerts(),
    );

    let data = DashboardData {
        metrics,
        active_recovery_tasks,
        active_alerts,
    };

    // --- populate cache (best-effort) ---
    if let Err(e) = try_cache_set(&state.redis, &data).await {
        warn!(error = %e, "Failed to populate dashboard cache");
    }

    Ok(Json(data))
}

// ---------------------------------------------------------------------------
// Cache helpers
// ---------------------------------------------------------------------------

async fn try_cache_get(redis: &RedisClient) -> Result<Option<DashboardData>, DashboardError> {
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let raw: Option<String> = conn.get(CACHE_KEY).await?;
    match raw {
        Some(s) => Ok(Some(serde_json::from_str(&s)?)),
        None => Ok(None),
    }
}

async fn try_cache_set(redis: &RedisClient, data: &DashboardData) -> Result<(), DashboardError> {
    let serialized = serde_json::to_string(data)?;
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let _: () = conn.set_ex(CACHE_KEY, serialized, CACHE_TTL_SECS).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use tower::ServiceExt;

    fn make_state() -> Arc<DashboardState> {
        Arc::new(DashboardState {
            metrics_exporter: Arc::new(MetricsExporter::new()),
            error_manager: Arc::new(ErrorManager::new()),
            alert_manager: Arc::new(AlertManager::new()),
            // Use a URL that will fail to connect — the handler degrades gracefully.
            redis: RedisClient::open("redis://127.0.0.1:1/").unwrap(),
        })
    }

    fn make_app(state: Arc<DashboardState>) -> Router {
        Router::new()
            .route("/api/dashboard", get(get_dashboard))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_dashboard_returns_200_without_redis() {
        let app = make_app(make_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dashboard_response_shape() {
        let app = make_app(make_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(json.get("metrics").is_some());
        assert!(json.get("active_recovery_tasks").is_some());
        assert!(json.get("active_alerts").is_some());
    }

    #[tokio::test]
    async fn test_dashboard_metrics_fields() {
        let state = make_state();
        state.metrics_exporter.update_metrics(42.0, 2048, 120).await;

        let app = make_app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["metrics"]["cpu_usage"], 42.0);
        assert_eq!(json["metrics"]["memory_usage"], 2048);
        assert_eq!(json["metrics"]["uptime"], 120);
    }

    #[tokio::test]
    async fn test_dashboard_includes_recovery_tasks() {
        use crate::services::error_recovery::RecoveryError;

        let state = make_state();
        state
            .error_manager
            .handle_error(RecoveryError::Internal("boom".into()), "worker_a")
            .await
            .unwrap();

        let app = make_app(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let tasks = json["active_recovery_tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["name"], "worker_a");
    }

    #[test]
    fn test_dashboard_error_display() {
        let err = DashboardError::Serialization(
            serde_json::from_str::<serde_json::Value>("bad json").unwrap_err(),
        );
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn test_dashboard_data_serialization_roundtrip() {
        let data = DashboardData {
            metrics: SystemMetrics::default(),
            active_recovery_tasks: vec![],
            active_alerts: vec![],
        };
        let json = serde_json::to_string(&data).unwrap();
        let back: DashboardData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.active_recovery_tasks.len(), 0);
        assert_eq!(back.active_alerts.len(), 0);
    }
}
