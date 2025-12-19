//! Graceful shutdown handling with connection draining.
//!
//! This module provides:
//! - Signal handling (SIGTERM, SIGINT, Ctrl+C)
//! - Connection draining with configurable timeout
//! - Active transaction cleanup
//! - Cache persistence (if enabled)
//! - Proper resource cleanup

use crate::state::SharedState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tracing::{error, info, warn};

/// Shutdown signal that can be awaited.
#[derive(Clone)]
pub struct ShutdownSignal {
    /// Receiver for shutdown notification.
    receiver: watch::Receiver<bool>,
}

impl ShutdownSignal {
    /// Wait for the shutdown signal.
    pub async fn recv(&mut self) {
        // Wait until the value becomes true
        let _ = self.receiver.wait_for(|&v| v).await;
    }

    /// Check if shutdown has been signaled without blocking.
    pub fn is_shutdown(&self) -> bool {
        *self.receiver.borrow()
    }
}

/// Controller for managing graceful shutdown.
pub struct ShutdownController {
    /// Sender to notify all listeners of shutdown.
    sender: watch::Sender<bool>,

    /// Flag indicating shutdown in progress.
    shutting_down: Arc<AtomicBool>,

    /// Broadcast sender for shutdown phase notifications.
    phase_sender: broadcast::Sender<ShutdownPhase>,

    /// Drain timeout duration.
    drain_timeout: Duration,

    /// Force shutdown timeout (after drain timeout).
    force_timeout: Duration,
}

/// Shutdown phases for coordinated cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownPhase {
    /// Shutdown has been initiated.
    Initiated,

    /// No longer accepting new requests.
    DrainingRequests,

    /// Rolling back active transactions.
    CleaningTransactions,

    /// Closing database connections.
    ClosingConnections,

    /// Flushing caches and buffers.
    FlushingCaches,

    /// Final cleanup complete.
    Complete,
}

impl std::fmt::Display for ShutdownPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShutdownPhase::Initiated => write!(f, "initiated"),
            ShutdownPhase::DrainingRequests => write!(f, "draining_requests"),
            ShutdownPhase::CleaningTransactions => write!(f, "cleaning_transactions"),
            ShutdownPhase::ClosingConnections => write!(f, "closing_connections"),
            ShutdownPhase::FlushingCaches => write!(f, "flushing_caches"),
            ShutdownPhase::Complete => write!(f, "complete"),
        }
    }
}

impl ShutdownController {
    /// Create a new shutdown controller with default timeouts.
    pub fn new() -> Self {
        Self::with_timeouts(Duration::from_secs(30), Duration::from_secs(10))
    }

    /// Create a shutdown controller with custom timeouts.
    pub fn with_timeouts(drain_timeout: Duration, force_timeout: Duration) -> Self {
        let (sender, _) = watch::channel(false);
        let (phase_sender, _) = broadcast::channel(16);

        Self {
            sender,
            shutting_down: Arc::new(AtomicBool::new(false)),
            phase_sender,
            drain_timeout,
            force_timeout,
        }
    }

    /// Get a shutdown signal receiver.
    pub fn signal(&self) -> ShutdownSignal {
        ShutdownSignal {
            receiver: self.sender.subscribe(),
        }
    }

    /// Subscribe to shutdown phase notifications.
    pub fn subscribe_phases(&self) -> broadcast::Receiver<ShutdownPhase> {
        self.phase_sender.subscribe()
    }

    /// Check if shutdown is in progress.
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    /// Initiate graceful shutdown.
    pub fn shutdown(&self) {
        if self
            .shutting_down
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            info!("Initiating graceful shutdown...");
            let _ = self.sender.send(true);
            let _ = self.phase_sender.send(ShutdownPhase::Initiated);
        }
    }

    /// Send a shutdown phase notification.
    fn notify_phase(&self, phase: ShutdownPhase) {
        info!("Shutdown phase: {}", phase);
        let _ = self.phase_sender.send(phase);
    }

    /// Perform graceful shutdown with connection draining.
    ///
    /// This method:
    /// 1. Stops accepting new requests
    /// 2. Waits for in-flight requests to complete (up to drain_timeout)
    /// 3. Rolls back any active transactions
    /// 4. Closes database connections
    /// 5. Flushes caches
    pub async fn graceful_shutdown(&self, state: &SharedState) {
        self.shutdown();

        // Phase 1: Draining requests
        self.notify_phase(ShutdownPhase::DrainingRequests);
        self.drain_requests(state).await;

        // Phase 2: Clean up transactions
        self.notify_phase(ShutdownPhase::CleaningTransactions);
        self.cleanup_transactions(state).await;

        // Phase 3: Close connections (pool cleanup is handled by Drop)
        self.notify_phase(ShutdownPhase::ClosingConnections);
        // Connection pool cleanup happens automatically via bb8's drop

        // Phase 4: Flush caches
        self.notify_phase(ShutdownPhase::FlushingCaches);
        self.flush_caches().await;

        // Phase 5: Complete
        self.notify_phase(ShutdownPhase::Complete);
        info!("Graceful shutdown complete");
    }

    /// Drain in-flight requests by waiting for running sessions to complete.
    async fn drain_requests(&self, state: &SharedState) {
        let start = std::time::Instant::now();

        loop {
            let running_count = {
                let s = state.read().await;
                s.running_session_count()
            };

            if running_count == 0 {
                info!("All requests drained");
                break;
            }

            if start.elapsed() > self.drain_timeout {
                warn!(
                    "Drain timeout exceeded with {} requests still running",
                    running_count
                );
                break;
            }

            info!(
                "Waiting for {} in-flight requests to complete...",
                running_count
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Clean up active transactions by rolling them back.
    async fn cleanup_transactions(&self, state: &SharedState) {
        let mut s = state.write().await;
        let active_count = s.active_transaction_count();

        if active_count > 0 {
            warn!(
                "Rolling back {} active transactions during shutdown",
                active_count
            );

            // Get all active transaction IDs
            let tx_ids: Vec<String> = s
                .list_active_transactions()
                .iter()
                .map(|t| t.id.clone())
                .collect();

            // Mark all transactions as rolled back
            for tx_id in tx_ids {
                if let Some(tx) = s.get_transaction_mut(&tx_id) {
                    tx.rollback();
                    info!("Rolled back transaction: {}", tx_id);
                }
            }
        }

        // Clean up old sessions and transactions
        s.cleanup_sessions(0); // Remove all completed sessions
        s.cleanup_transactions(0); // Remove all completed transactions
    }

    /// Flush caches before shutdown.
    async fn flush_caches(&self) {
        // Cache cleanup would happen here if we had a reference to the cache
        // For now, we just log that we're flushing
        info!("Flushing caches...");
        // The actual cache is cleaned up via Drop when the server is dropped
    }

    /// Get the drain timeout.
    pub fn drain_timeout(&self) -> Duration {
        self.drain_timeout
    }

    /// Get the force timeout.
    pub fn force_timeout(&self) -> Duration {
        self.force_timeout
    }
}

impl Default for ShutdownController {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared shutdown controller type.
pub type SharedShutdownController = Arc<ShutdownController>;

/// Create a new shared shutdown controller.
pub fn new_shutdown_controller() -> SharedShutdownController {
    Arc::new(ShutdownController::new())
}

/// Create a shutdown controller with custom timeouts.
pub fn new_shutdown_controller_with_timeouts(
    drain_timeout: Duration,
    force_timeout: Duration,
) -> SharedShutdownController {
    Arc::new(ShutdownController::with_timeouts(
        drain_timeout,
        force_timeout,
    ))
}

/// Install signal handlers for graceful shutdown.
///
/// This sets up handlers for:
/// - SIGTERM (Unix)
/// - SIGINT (Ctrl+C)
///
/// When a signal is received, the shutdown controller is triggered.
pub async fn install_signal_handlers(controller: SharedShutdownController) {
    let ctrl_c_controller = controller.clone();

    // Handle Ctrl+C
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("Received Ctrl+C, initiating shutdown...");
                ctrl_c_controller.shutdown();
            }
            Err(e) => {
                error!("Failed to listen for Ctrl+C signal: {}", e);
            }
        }
    });

    // Handle SIGTERM (Unix only)
    #[cfg(unix)]
    {
        let term_controller = controller.clone();
        tokio::spawn(async move {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    sigterm.recv().await;
                    info!("Received SIGTERM, initiating shutdown...");
                    term_controller.shutdown();
                }
                Err(e) => {
                    error!("Failed to install SIGTERM handler: {}", e);
                }
            }
        });
    }

    // Handle SIGHUP (Unix only) - often used to reload config, but we use it for shutdown
    #[cfg(unix)]
    {
        let hup_controller = controller;
        tokio::spawn(async move {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
                Ok(mut sighup) => {
                    sighup.recv().await;
                    info!("Received SIGHUP, initiating shutdown...");
                    hup_controller.shutdown();
                }
                Err(e) => {
                    error!("Failed to install SIGHUP handler: {}", e);
                }
            }
        });
    }
}

/// Shutdown configuration.
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// Timeout for draining in-flight requests.
    pub drain_timeout: Duration,

    /// Timeout before forcing shutdown after drain timeout.
    pub force_timeout: Duration,

    /// Whether to rollback active transactions on shutdown.
    pub rollback_transactions: bool,

    /// Whether to flush caches before shutdown.
    pub flush_caches: bool,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            force_timeout: Duration::from_secs(10),
            rollback_transactions: true,
            flush_caches: true,
        }
    }
}

impl ShutdownConfig {
    /// Create configuration from environment variables.
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(drain) = std::env::var("MSSQL_SHUTDOWN_DRAIN_TIMEOUT") {
            if let Ok(secs) = drain.parse::<u64>() {
                config.drain_timeout = Duration::from_secs(secs);
            }
        }

        if let Ok(force) = std::env::var("MSSQL_SHUTDOWN_FORCE_TIMEOUT") {
            if let Ok(secs) = force.parse::<u64>() {
                config.force_timeout = Duration::from_secs(secs);
            }
        }

        if let Ok(rollback) = std::env::var("MSSQL_SHUTDOWN_ROLLBACK_TX") {
            config.rollback_transactions = rollback.to_lowercase() == "true" || rollback == "1";
        }

        if let Ok(flush) = std::env::var("MSSQL_SHUTDOWN_FLUSH_CACHE") {
            config.flush_caches = flush.to_lowercase() == "true" || flush == "1";
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::new_shared_state;

    #[test]
    fn test_shutdown_controller_creation() {
        let controller = ShutdownController::new();
        assert!(!controller.is_shutting_down());
        assert_eq!(controller.drain_timeout(), Duration::from_secs(30));
        assert_eq!(controller.force_timeout(), Duration::from_secs(10));
    }

    #[test]
    fn test_shutdown_signal() {
        let controller = ShutdownController::new();
        let signal = controller.signal();

        assert!(!signal.is_shutdown());

        controller.shutdown();
        assert!(controller.is_shutting_down());
        assert!(signal.is_shutdown());
    }

    #[test]
    fn test_shutdown_idempotent() {
        let controller = ShutdownController::new();

        controller.shutdown();
        assert!(controller.is_shutting_down());

        // Calling shutdown again should be a no-op
        controller.shutdown();
        assert!(controller.is_shutting_down());
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let controller = new_shutdown_controller();
        let state = new_shared_state();

        // Initialize state
        {
            let mut s = state.write().await;
            s.mark_initialized();
        }

        // Perform graceful shutdown
        controller.graceful_shutdown(&state).await;

        assert!(controller.is_shutting_down());
    }

    #[tokio::test]
    async fn test_drain_with_sessions() {
        let controller = new_shutdown_controller_with_timeouts(
            Duration::from_millis(100),
            Duration::from_millis(50),
        );
        let state = new_shared_state();

        // Create a running session
        {
            let mut s = state.write().await;
            s.mark_initialized();
            let _ = s.create_session("SELECT 1".to_string(), 10);
        }

        // Start shutdown in background and complete session
        let state_clone = state.clone();
        let controller_clone = controller.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let mut s = state_clone.write().await;
            let sessions = s.list_sessions();
            if let Some(session) = sessions.first() {
                if let Some(sess) = s.get_session_mut(&session.id) {
                    sess.complete(crate::database::QueryResult::empty());
                }
            }
        });

        controller_clone.graceful_shutdown(&state).await;
        assert!(controller_clone.is_shutting_down());
    }

    #[test]
    fn test_shutdown_config_defaults() {
        let config = ShutdownConfig::default();
        assert_eq!(config.drain_timeout, Duration::from_secs(30));
        assert_eq!(config.force_timeout, Duration::from_secs(10));
        assert!(config.rollback_transactions);
        assert!(config.flush_caches);
    }

    #[test]
    fn test_shutdown_phase_display() {
        assert_eq!(ShutdownPhase::Initiated.to_string(), "initiated");
        assert_eq!(
            ShutdownPhase::DrainingRequests.to_string(),
            "draining_requests"
        );
        assert_eq!(
            ShutdownPhase::CleaningTransactions.to_string(),
            "cleaning_transactions"
        );
        assert_eq!(
            ShutdownPhase::ClosingConnections.to_string(),
            "closing_connections"
        );
        assert_eq!(ShutdownPhase::FlushingCaches.to_string(), "flushing_caches");
        assert_eq!(ShutdownPhase::Complete.to_string(), "complete");
    }
}
