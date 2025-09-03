use tracing::info;

/// Side effects that may be triggered by state transitions
pub struct Effects;

impl Effects {
    /// Log state transition for debugging
    pub fn log_transition(
        session_id: &str,
        from_state: &str,
        to_state: &str,
        event: &str,
    ) {
        info!(
            "Session {} transitioned from {} to {} via {}",
            session_id, from_state, to_state, event
        );
    }
    
    /// Record metrics for monitoring
    pub fn record_metrics(
        session_id: &str,
        state: &str,
        duration_ms: u64,
    ) {
        // Log metrics for now - in production this would integrate with Prometheus/StatsD/etc.
        tracing::debug!(
            "Metrics recorded - Session: {}, State: {}, Duration: {}ms",
            session_id, state, duration_ms
        );
    }
    
    /// Send notifications
    pub async fn notify_handlers(
        session_id: &str,
        event: &str,
    ) {
        // Log notification for now - in production this would call registered callbacks
        tracing::debug!(
            "Event notification sent - Session: {}, Event: {}",
            session_id, event
        );
    }
}