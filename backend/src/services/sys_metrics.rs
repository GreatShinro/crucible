use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use tracing::{info, instrument};
use crate::services::tracing::TracingService;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemMetrics {
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub uptime: u64,
    pub timestamp: DateTime<Utc>,
}

pub struct MetricsExporter {
    current_metrics: Arc<RwLock<SystemMetrics>>,
}

impl Default for MetricsExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsExporter {
    pub fn new() -> Self {
        Self {
            current_metrics: Arc::new(RwLock::new(SystemMetrics {
                timestamp: Utc::now(),
                ..Default::default()
            })),
        }
    }

    #[instrument(skip(self), fields(service.name = "MetricsExporter", service.method = "update_metrics"))]
    pub async fn update_metrics(&self, cpu: f64, mem: u64, uptime: u64) {
        let span = TracingService::service_method_span("MetricsExporter", "update_metrics");
        let _enter = span.enter();
        
        let mut metrics = self.current_metrics.write().await;
        metrics.cpu_usage = cpu;
        metrics.memory_usage = mem;
        metrics.uptime = uptime;
        metrics.timestamp = Utc::now();
        info!(metrics = ?*metrics, "Updated system metrics");
    }

    #[instrument(skip(self), fields(service.name = "MetricsExporter", service.method = "get_metrics"))]
    pub async fn get_metrics(&self) -> SystemMetrics {
        let span = TracingService::service_method_span("MetricsExporter", "get_metrics");
        let _enter = span.enter();
        
        self.current_metrics.read().await.clone()
    }

    #[instrument(skip(exporter), fields(service.name = "MetricsExporter", service.method = "run_collector"))]
    pub async fn run_collector(exporter: Arc<Self>) {
        let span = TracingService::service_method_span("MetricsExporter", "run_collector");
        let _enter = span.enter();
        
        info!("Starting system metrics collector worker");
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        let start_time = Utc::now();

        loop {
            interval.tick().await;
            let uptime = (Utc::now() - start_time).num_seconds() as u64;
            // Simulated metrics collection
            exporter
                .update_metrics(12.5, 1024 * 1024 * 512, uptime)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_collection() {
        let exporter = MetricsExporter::new();
        exporter.update_metrics(25.0, 1024, 60).await;

        let metrics = exporter.get_metrics().await;
        assert_eq!(metrics.cpu_usage, 25.0);
        assert_eq!(metrics.memory_usage, 1024);
        assert_eq!(metrics.uptime, 60);
    }
}
