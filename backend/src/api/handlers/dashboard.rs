//! Dashboard metrics handlers.
//!
//! Provides endpoints for aggregated contract and transaction metrics,
//! with Redis caching to reduce database load.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{error, info, instrument};
use utoipa::ToSchema;

use crate::error::AppError;

/// Shared application state for dashboard handlers.
pub struct DashboardState {
    pub db: PgPool,
    pub redis: redis::aio::ConnectionManager,
}

/// Aggregated dashboard metrics.
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct DashboardMetrics {
    pub total_contracts: i64,
    pub total_transactions: i64,
    pub avg_processing_time_ms: f64,
    pub failed_transactions_24h: i64,
    pub timestamp: DateTime<Utc>,
}

/// Per-contract statistics.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ContractStats {
    pub contract_id: String,
    pub invocation_count: i64,
    pub last_invoked: Option<DateTime<Utc>>,
    pub avg_gas_cost: f64,
}

/// `GET /api/v1/dashboard/metrics` — Aggregated dashboard metrics with Redis caching.
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/metrics",
    responses(
        (status = 200, description = "Dashboard metrics", body = DashboardMetrics),
        (status = 500, description = "Internal server error")
    ),
    tag = "dashboard"
)]
#[instrument(skip(state))]
pub async fn get_dashboard_metrics(
    State(state): State<Arc<DashboardState>>,
) -> Result<impl IntoResponse, AppError> {
    info!("Fetching dashboard metrics");

    const CACHE_KEY: &str = "dashboard:metrics";
    let mut redis_conn = state.redis.clone();

    // Try cache first.
    if let Ok(cached) = redis_conn.get::<_, String>(CACHE_KEY).await {
        if let Ok(metrics) = serde_json::from_str::<DashboardMetrics>(&cached) {
            info!("Returning cached dashboard metrics");
            return Ok(Json(metrics));
        }
    }

    let total_contracts = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM contracts")
        .fetch_optional(&state.db)
        .await?
        .unwrap_or(0);

    let total_transactions = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transactions")
        .fetch_optional(&state.db)
        .await?
        .unwrap_or(0);

    let avg_processing_time = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(processing_time_ms) FROM transactions WHERE processing_time_ms IS NOT NULL",
    )
    .fetch_one(&state.db)
    .await?
    .unwrap_or(0.0);

    let failed_24h = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM transactions \
         WHERE status = 'failed' AND created_at > NOW() - INTERVAL '24 hours'",
    )
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(0);

    let metrics = DashboardMetrics {
        total_contracts,
        total_transactions,
        avg_processing_time_ms: avg_processing_time,
        failed_transactions_24h: failed_24h,
        timestamp: Utc::now(),
    };

    // Cache for 60 seconds (best-effort).
    if let Ok(json) = serde_json::to_string(&metrics) {
        let _: Result<(), _> = redis_conn.set_ex(CACHE_KEY, json, 60).await;
    }

    info!(
        contracts = metrics.total_contracts,
        transactions = metrics.total_transactions,
        "Dashboard metrics retrieved"
    );

    Ok(Json(metrics))
}

/// `GET /api/v1/dashboard/contracts/:contract_id/stats` — Per-contract statistics.
#[utoipa::path(
    get,
    path = "/api/v1/dashboard/contracts/{contract_id}/stats",
    params(("contract_id" = String, Path, description = "Contract identifier")),
    responses(
        (status = 200, description = "Contract statistics", body = ContractStats),
        (status = 404, description = "Contract not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "dashboard"
)]
#[instrument(skip(state))]
pub async fn get_contract_stats(
    State(state): State<Arc<DashboardState>>,
    Path(contract_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    info!(contract_id = %contract_id, "Fetching contract statistics");

    let cache_key = format!("dashboard:contract:{}:stats", contract_id);
    let mut redis_conn = state.redis.clone();

    if let Ok(cached) = redis_conn.get::<_, String>(&cache_key).await {
        if let Ok(stats) = serde_json::from_str::<ContractStats>(&cached) {
            return Ok(Json(stats));
        }
    }

    let result = sqlx::query!(
        r#"
        SELECT
            COUNT(*) as "invocation_count!",
            MAX(created_at) as last_invoked,
            AVG(gas_cost) as avg_gas_cost
        FROM transactions
        WHERE contract_id = $1
        "#,
        contract_id
    )
    .fetch_optional(&state.db)
    .await?;

    let stats = match result {
        Some(row) if row.invocation_count > 0 => ContractStats {
            contract_id: contract_id.clone(),
            invocation_count: row.invocation_count,
            last_invoked: row.last_invoked,
            avg_gas_cost: row.avg_gas_cost.unwrap_or(0.0),
        },
        _ => {
            error!(contract_id = %contract_id, "Contract not found");
            return Err(AppError::NotFound(format!(
                "Contract {} not found",
                contract_id
            )));
        }
    };

    if let Ok(json) = serde_json::to_string(&stats) {
        let _: Result<(), _> = redis_conn.set_ex(&cache_key, json, 30).await;
    }

    Ok(Json(stats))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dashboard_metrics_serialization() {
        let metrics = DashboardMetrics {
            total_contracts: 100,
            total_transactions: 5000,
            avg_processing_time_ms: 125.5,
            failed_transactions_24h: 3,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&metrics).unwrap();
        let back: DashboardMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_contracts, 100);
        assert_eq!(back.total_transactions, 5000);
    }

    #[test]
    fn test_contract_stats_serialization() {
        let stats = ContractStats {
            contract_id: "test_contract_123".to_string(),
            invocation_count: 42,
            last_invoked: Some(Utc::now()),
            avg_gas_cost: 1500.75,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let back: ContractStats = serde_json::from_str(&json).unwrap();
        assert_eq!(back.contract_id, "test_contract_123");
        assert_eq!(back.invocation_count, 42);
    }
}
