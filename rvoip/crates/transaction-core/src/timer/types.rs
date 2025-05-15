//! Defines the core types used for managing SIP timers within the transaction layer.
//!
//! This module provides:
//! - [`TimerType`]: An enumeration of specific SIP transaction timers (e.g., Timer A, Timer B)
//!   as defined in RFC 3261, as well as generic timer categories.
//! - [`Timer`]: A struct representing an active timer instance, containing its properties,
//!   scheduling information, and associated transaction.
//! - [`TimerSettings`]: Configuration for timer durations as specified in RFC 3261.
//!
//! # SIP Timer Protocol Requirements
//!
//! RFC 3261 specifies various timers that govern different aspects of SIP transactions:
//!
//! - **Retransmission Timers**: Control when messages are retransmitted over unreliable transports (A, E, G)
//! - **Transaction Timeout Timers**: Limit overall transaction lifetime (B, F, H)
//! - **Wait Timers**: Control how long to remain in certain states to absorb retransmissions (D, I, J, K)
//!
//! The base retransmission interval (T1) is typically 500ms, doubling with each retransmission
//! up to a maximum (T2) of 4 seconds. For reliable transports like TCP, many of these timers
//! can be set to zero as retransmissions are handled by the transport layer.

use std::fmt;
use std::time::Duration;
use tokio::time::Instant;

use crate::transaction::TransactionKey;

/// Specifies the type of a SIP transaction timer, corresponding to timers defined in RFC 3261
/// or generic timer categories used by the transaction layer.
///
/// Each timer plays a specific role in managing the lifecycle of SIP transactions,
/// such as handling retransmissions, request timeouts, and state transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerType {
    /// **Timer A (INVITE Client): Initial INVITE Retransmission Interval.**
    /// Governs the interval for retransmitting INVITE requests when using an unreliable transport (e.g., UDP).
    /// Starts at T1 and doubles for each retransmission. Defined in RFC 3261, Section 17.1.1.2.
    A,
    /// **Timer B (INVITE Client): INVITE Transaction Timeout.**
    /// Limits the lifetime of an INVITE client transaction. If this timer fires, the transaction is terminated.
    /// Typically 64 * T1. Defined in RFC 3261, Section 17.1.1.2.
    B,
    /// **Timer D (INVITE Client): Wait Time for Response Retransmissions.**
    /// After receiving a non-2xx final response to an INVITE, the client transaction waits for this duration
    /// to absorb any retransmissions of the response before terminating.
    /// At least 32 seconds for unreliable transports. Defined in RFC 3261, Section 17.1.1.2.
    D,
    /// **Timer E (Non-INVITE Client): Initial Non-INVITE Request Retransmission Interval.**
    /// Governs the interval for retransmitting non-INVITE requests (e.g., REGISTER, OPTIONS)
    /// when using an unreliable transport. Starts at T1 and doubles up to T2.
    /// Defined in RFC 3261, Section 17.1.2.2.
    E,
    /// **Timer F (Non-INVITE Client): Non-INVITE Transaction Timeout.**
    /// Limits the lifetime of a non-INVITE client transaction. If this timer fires, the transaction is terminated.
    /// Typically 64 * T1. Defined in RFC 3261, Section 17.1.2.2.
    F,
    /// **Timer G (INVITE Server): INVITE Response Retransmission Interval.**
    /// Governs the retransmission of a 2xx response to an INVITE by the server transaction
    /// if an ACK is not received. Starts at T1 and doubles up to T2.
    /// Defined in RFC 3261, Section 17.2.1.
    G,
    /// **Timer H (INVITE Server): ACK Timeout.**
    /// After sending a 2xx response to an INVITE, the server transaction waits for an ACK.
    /// If Timer H fires before an ACK is received, the transaction proceeds as if an ACK was received.
    /// Typically 64 * T1. Defined in RFC 3261, Section 17.2.1.
    H,
    /// **Timer I (INVITE Server): Wait Time for ACK Retransmissions.**
    /// After an ACK is received for a 2xx response, the server transaction waits for this duration
    /// in the Confirmed state to absorb any retransmitted ACKs (primarily for reliable transports,
    /// but for UDP it also relates to T4 from Timer G perspective for how long 2xx is retransmitted).
    /// Value is T4 for unreliable transports. Defined in RFC 3261, Section 17.2.1.
    I,
    /// **Timer J (Non-INVITE Server): Wait Time for Request Retransmissions.**
    /// After sending a final response to a non-INVITE request, the server transaction waits for
    /// this duration to absorb any retransmissions of the request before terminating.
    /// Value is 64 * T1 for unreliable transports (effectively T4 if T1=500ms, or T4 itself directly).
    /// Defined in RFC 3261, Section 17.2.2.
    J,
    /// **Timer K (Non-INVITE Client): Wait Time for Response Retransmissions.**
    /// After receiving a final response to a non-INVITE request, the client transaction waits
    /// for this duration to absorb retransmissions of the response before terminating.
    /// Value is T4 for unreliable transports. Defined in RFC 3261, Section 17.1.2.2.
    K,
    /// A generic timer used for message retransmissions, not specific to a particular RFC 3261 timer letter.
    /// Behavior (e.g., backoff strategy) would be defined by how the `Timer` struct is configured.
    Retransmission,
    /// A generic timer indicating the overall timeout for a transaction.
    /// Firing of this timer usually leads to transaction termination.
    TransactionTimeout,
    /// A generic timer used to wait for potential retransmissions of messages to cease
    /// before fully cleaning up a transaction.
    WaitForRetransmissions,
    /// A generic timer specifically for waiting for an ACK message.
    WaitForAck,
    /// A generic timer for a waiting period after an ACK has been processed.
    WaitAfterAck,
    /// A custom timer type, used when the timer does not fit any of the predefined categories.
    /// The behavior is entirely determined by its configuration in the `Timer` struct.
    Custom,
}

impl fmt::Display for TimerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimerType::A => write!(f, "A"),
            TimerType::B => write!(f, "B"),
            TimerType::D => write!(f, "D"),
            TimerType::E => write!(f, "E"),
            TimerType::F => write!(f, "F"),
            TimerType::G => write!(f, "G"),
            TimerType::H => write!(f, "H"),
            TimerType::I => write!(f, "I"),
            TimerType::J => write!(f, "J"),
            TimerType::K => write!(f, "K"),
            TimerType::Retransmission => write!(f, "Retransmission"),
            TimerType::TransactionTimeout => write!(f, "TransactionTimeout"),
            TimerType::WaitForRetransmissions => write!(f, "WaitForRetransmissions"),
            TimerType::WaitForAck => write!(f, "WaitForAck"),
            TimerType::WaitAfterAck => write!(f, "WaitAfterAck"),
            TimerType::Custom => write!(f, "Custom"),
        }
    }
}

/// Represents an active timer instance within the SIP transaction layer.
///
/// A `Timer` is associated with a specific transaction (`transaction_id`) and has a defined
/// behavior (one-shot, repeating, backoff) and scheduling.
#[derive(Debug, Clone)]
pub struct Timer {
    /// A descriptive name for the timer, often corresponding to its `TimerType`
    /// (e.g., "A", "B", "G", "H", or a custom name).
    pub name: String,
    /// The unique identifier of the transaction this timer is associated with.
    pub transaction_id: TransactionKey,
    /// Indicates whether the timer should automatically reschedule itself after firing.
    /// `true` for repeating or backoff timers, `false` for one-shot timers.
    pub repeating: bool,
    /// The base interval for the timer. For one-shot timers, this is the duration until it fires.
    /// For repeating timers, this might be the initial or fixed interval.
    /// For backoff timers, this is the initial interval before exponential backoff begins.
    pub interval: Option<Duration>,
    /// The current interval at which the timer will fire next, particularly relevant for
    /// backoff timers where this value changes after each firing.
    /// For non-backoff repeating timers, this would be the same as `interval`.
    pub current_interval: Option<Duration>,
    /// The maximum interval for a backoff timer. The `current_interval` will not exceed this value.
    pub max_interval: Option<Duration>,
    /// The `Instant` at which this timer instance was created.
    pub created_at: Instant,
    /// The `Instant` at which this timer is scheduled to fire next.
    pub scheduled_at: Instant,
    /// The specific [`TimerType`] of this timer, categorizing its purpose (e.g., TimerA, Retransmission).
    pub timer_type: TimerType,
}

impl Timer {
    /// Creates a new one-shot (non-repeating) timer.
    ///
    /// The timer will fire once after the specified `interval` has elapsed from the moment of creation.
    ///
    /// # Arguments
    /// * `name` - A descriptive name for the timer.
    /// * `transaction_id` - The [`TransactionKey`] of the associated transaction.
    /// * `interval` - The [`Duration`] after which the timer will fire.
    ///
    /// The `timer_type` is set to [`TimerType::Custom`].
    pub fn new_one_shot(name: &str, transaction_id: TransactionKey, interval: Duration) -> Self {
        let now = Instant::now();
        Self {
            name: name.to_string(),
            transaction_id,
            repeating: false,
            interval: Some(interval),
            current_interval: None,
            max_interval: None,
            created_at: now,
            scheduled_at: now + interval,
            timer_type: TimerType::Custom,
        }
    }

    /// Creates a new repeating timer with a fixed interval.
    ///
    /// The timer will fire repeatedly, with each firing occurring after the specified `interval`
    /// has elapsed from the previous firing (or creation time for the first firing).
    ///
    /// # Arguments
    /// * `name` - A descriptive name for the timer.
    /// * `transaction_id` - The [`TransactionKey`] of the associated transaction.
    /// * `interval` - The fixed [`Duration`] between firings.
    ///
    /// The `timer_type` is set to [`TimerType::Custom`]. `current_interval` is initialized to `interval`.
    pub fn new_repeating(name: &str, transaction_id: TransactionKey, interval: Duration) -> Self {
        let now = Instant::now();
        Self {
            name: name.to_string(),
            transaction_id,
            repeating: true,
            interval: Some(interval),
            current_interval: Some(interval),
            max_interval: None,
            created_at: now,
            scheduled_at: now + interval,
            timer_type: TimerType::Custom,
        }
    }

    /// Creates a new repeating timer that uses an exponential backoff strategy for its interval.
    ///
    /// The timer starts with `initial_interval`. After each firing, the interval for the next
    /// firing is doubled, up to a maximum of `max_interval`.
    ///
    /// # Arguments
    /// * `name` - A descriptive name for the timer.
    /// * `transaction_id` - The [`TransactionKey`] of the associated transaction.
    /// * `initial_interval` - The starting [`Duration`] for the timer.
    /// * `max_interval` - The maximum [`Duration`] the interval can reach.
    ///
    /// The `timer_type` is set to [`TimerType::Custom`].
    pub fn new_backoff(name: &str, transaction_id: TransactionKey, initial_interval: Duration, max_interval: Duration) -> Self {
        let now = Instant::now();
        Self {
            name: name.to_string(),
            transaction_id,
            repeating: true,
            interval: Some(initial_interval),
            current_interval: Some(initial_interval),
            max_interval: Some(max_interval),
            created_at: now,
            scheduled_at: now + initial_interval,
            timer_type: TimerType::Custom,
        }
    }

    /// Creates a new timer with a specific [`TimerType`], inferring some properties.
    ///
    /// This constructor is useful for creating standard RFC 3261 timers where behavior
    /// (like repeating for retransmissions) is implied by the type.
    /// For example, a `TimerType::Retransmission` timer will be set as `repeating`.
    ///
    /// # Arguments
    /// * `name` - A descriptive name for the timer.
    /// * `transaction_id` - The [`TransactionKey`] of the associated transaction.
    /// * `interval` - The base [`Duration`] for the timer.
    /// * `timer_type` - The specific [`TimerType`] for this timer.
    ///
    /// Note: The `repeating` and `current_interval` fields are set based on a simple match
    /// for `TimerType::Retransmission`. Other types default to non-repeating and no `current_interval`
    /// unless set otherwise after creation.
    pub fn new_with_type(name: &str, transaction_id: TransactionKey, interval: Duration, timer_type: TimerType) -> Self {
        let now = Instant::now();
        Self {
            name: name.to_string(),
            transaction_id,
            repeating: match timer_type {
                TimerType::Retransmission => true,
                _ => false,
            },
            interval: Some(interval),
            current_interval: match timer_type {
                TimerType::Retransmission => Some(interval),
                _ => None,
            },
            max_interval: None,
            created_at: now,
            scheduled_at: now + interval,
            timer_type,
        }
    }

    /// Calculates the next interval for a backoff timer and updates its `current_interval`.
    ///
    /// The current interval is doubled. If a `max_interval` is set, the new interval is capped at this maximum.
    /// If `current_interval` was not set, it attempts to use `interval` as the base.
    ///
    /// # Returns
    /// The new `current_interval` [`Duration`] after applying the backoff logic.
    ///
    /// Panics if `interval` is `None` and `current_interval` was also `None` (this case is handled by unwrap_or_else).
    pub fn next_backoff_interval(&mut self) -> Duration {
        if let (Some(current), Some(max)) = (self.current_interval, self.max_interval) {
            let next = std::cmp::min(current * 2, max);
            self.current_interval = Some(next);
            next
        } else if let Some(current) = self.current_interval {
            let next = current * 2;
            self.current_interval = Some(next);
            next
        } else {
            // Fallback to interval if current_interval is not set
            let interval = self.interval.unwrap_or_else(|| Duration::from_millis(500));
            self.current_interval = Some(interval);
            interval
        }
    }

    /// Reschedules the timer to fire again after its `current_interval` (if set) or `interval`.
    ///
    /// Updates `scheduled_at` to `Instant::now() + relevant_interval`.
    /// This is typically called for repeating timers after they have fired.
    pub fn reschedule(&mut self) {
        if let Some(interval) = self.current_interval.or(self.interval) {
            self.scheduled_at = Instant::now() + interval;
        }
    }

    /// Reschedules the timer to fire after a specific `interval`.
    ///
    /// Updates `scheduled_at` to `Instant::now() + interval`.
    /// If the timer is `repeating`, `current_interval` is also updated to this new `interval`.
    ///
    /// # Arguments
    /// * `interval` - The new [`Duration`] to wait before the timer fires.
    pub fn reschedule_with_interval(&mut self, interval: Duration) {
        self.scheduled_at = Instant::now() + interval;
        if self.repeating {
            self.current_interval = Some(interval);
        }
    }

    /// Checks if the timer has expired.
    ///
    /// A timer is considered expired if the current `Instant::now()` is greater than or equal to
    /// its `scheduled_at` time.
    ///
    /// # Returns
    /// `true` if the timer has expired, `false` otherwise.
    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.scheduled_at
    }

    /// Gets the time remaining until the timer is scheduled to fire.
    ///
    /// If the timer has already expired (i.e., `scheduled_at` is in the past or now),
    /// it returns a zero duration.
    ///
    /// # Returns
    /// A [`Duration`] representing the time remaining, or zero if expired.
    pub fn time_remaining(&self) -> Duration {
        let now = Instant::now();
        if now >= self.scheduled_at {
            Duration::from_secs(0)
        } else {
            self.scheduled_at - now
        }
    }
}

impl fmt::Display for Timer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Timer({}, tx={}, repeating={}, scheduled={}ms)",
            self.name,
            self.transaction_id,
            self.repeating,
            self.time_remaining().as_millis()
        )
    }
}

/// Configuration for standard SIP transaction timer durations, based on RFC 3261.
///
/// These settings control the behavior of timers managed by the [`TimerFactory`]
/// and [`TimerManager`].
///
/// # Example: Custom Timer Settings
///
/// ```rust
/// use std::time::Duration;
/// use rvoip_transaction_core::timer::TimerSettings;
///
/// // Default settings (RFC 3261 recommended values)
/// let default_settings = TimerSettings::default();
/// assert_eq!(default_settings.t1, Duration::from_millis(500));
/// assert_eq!(default_settings.t2, Duration::from_secs(4));
///
/// // Custom settings for high-latency networks
/// let slow_network_settings = TimerSettings {
///     t1: Duration::from_millis(1000),    // Double the retransmission interval
///     t2: Duration::from_secs(8),         // Double the maximum retransmission interval
///     transaction_timeout: Duration::from_secs(64),  // 64*T1 with new T1 value
///     ..Default::default()
/// };
///
/// // Custom settings for local testing (faster timeouts)
/// let fast_test_settings = TimerSettings {
///     t1: Duration::from_millis(100),     // Fast initial retransmissions
///     t2: Duration::from_millis(400),     // Fast maximum retransmission interval
///     transaction_timeout: Duration::from_secs(8),  // Quicker timeouts for tests
///     wait_time_d: Duration::from_secs(4),  // Short wait times
///     wait_time_i: Duration::from_millis(500),
///     wait_time_k: Duration::from_millis(500),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)] // Added PartialEq, Eq for testing
pub struct TimerSettings {
    /// **T1: Round-Trip Time (RTT) Estimate (Default: 500 ms)**
    /// An estimate of the RTT between client and server. It's the initial retransmission
    /// interval for INVITEs (Timer A) and non-INVITEs (Timer E) over UDP.
    /// Retransmissions typically double this interval. (RFC 3261, Section 17.1.1.2)
    pub t1: Duration,

    /// **T2: Maximum Retransmission Interval (Default: 4 seconds)**
    /// The maximum interval for retransmitting non-INVITE requests (Timer E) and
    /// INVITE responses (Timer G). (RFC 3261, Section 17.1.2.2)
    pub t2: Duration,

    /// **Transaction Timeout (Default: 32 seconds, i.e., 64 * T1)**
    /// General timeout for INVITE (Timer B) and non-INVITE (Timer F) client transactions.
    /// (RFC 3261, Sections 17.1.1.2 & 17.1.2.2)
    pub transaction_timeout: Duration,

    /// **Timer D Wait Time (Default: 32 seconds)**
    /// Duration an INVITE client transaction waits in the Completed state for response
    /// retransmissions after receiving a non-2xx final response.
    /// Minimum 32s for unreliable transports. (RFC 3261, Section 17.1.1.2)
    pub wait_time_d: Duration,

    /// **Timer H Wait Time (Default: 32 seconds, i.e., 64 * T1)**
    /// Duration an INVITE server transaction waits for an ACK after sending a 2xx response.
    /// (RFC 3261, Section 17.2.1)
    pub wait_time_h: Duration,

    /// **Timer I Wait Time (Default: 5 seconds, i.e., T4)**
    /// Duration an INVITE server transaction waits in the Confirmed state (after ACK for 2xx)
    /// to absorb retransmitted ACKs. This is T4 for unreliable transports.
    /// (RFC 3261, Section 17.2.1)
    pub wait_time_i: Duration,

    /// **Timer J Wait Time (Default: 32 seconds, i.e., 64 * T1)**
    /// Duration a non-INVITE server transaction waits in the Completed state
    /// to absorb request retransmissions. (RFC 3261, Section 17.2.2)
    pub wait_time_j: Duration,

    /// **Timer K Wait Time (Default: 5 seconds, i.e., T4)**
    /// Duration a non-INVITE client transaction waits in the Completed state
    /// to absorb response retransmissions. (RFC 3261, Section 17.1.2.2)
    pub wait_time_k: Duration,
}

impl Default for TimerSettings {
    /// Provides default values for `TimerSettings` as recommended by RFC 3261
    /// (assuming T1=500ms, T4=5s).
    fn default() -> Self {
        Self {
            t1: Duration::from_millis(500),
            t2: Duration::from_secs(4),
            transaction_timeout: Duration::from_secs(32), // 64 * T1
            wait_time_d: Duration::from_secs(32),
            wait_time_h: Duration::from_secs(32),         // 64 * T1
            wait_time_i: Duration::from_secs(5),          // T4
            wait_time_j: Duration::from_secs(32),         // 64 * T1
            wait_time_k: Duration::from_secs(5),          // T4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::TransactionKey; // Assuming TransactionKey can be easily created for tests
    use rvoip_sip_core::Method; // For TransactionKey
    use tokio::time::sleep; // For testing expiration

    // Helper to create a dummy TransactionKey for tests
    fn dummy_tx_key(name: &str) -> TransactionKey {
        TransactionKey::new(format!("branch-{}", name), Method::Invite, false)
    }

    #[test]
    fn timer_type_display() {
        assert_eq!(TimerType::A.to_string(), "A");
        assert_eq!(TimerType::Retransmission.to_string(), "Retransmission");
        assert_eq!(TimerType::Custom.to_string(), "Custom");
        // Add more variants as needed
    }

    #[test]
    fn timer_type_equality_and_hash() {
        // Basic equality
        assert_eq!(TimerType::B, TimerType::B);
        assert_ne!(TimerType::B, TimerType::A); // Changed C to A
        // Can be tested with a HashSet if more thoroughness is needed
        let mut set = std::collections::HashSet::new();
        set.insert(TimerType::A);
        assert!(set.contains(&TimerType::A));
    }
    
    #[tokio::test]
    async fn timer_new_one_shot() {
        let tx_key = dummy_tx_key("one_shot");
        let interval = Duration::from_millis(100);
        let timer = Timer::new_one_shot("test_one_shot", tx_key.clone(), interval);

        assert_eq!(timer.name, "test_one_shot");
        assert_eq!(timer.transaction_id, tx_key);
        assert!(!timer.repeating);
        assert_eq!(timer.interval, Some(interval));
        assert!(timer.current_interval.is_none());
        assert!(timer.max_interval.is_none());
        assert_eq!(timer.timer_type, TimerType::Custom);
        assert!(timer.scheduled_at > timer.created_at);
        assert_eq!(timer.scheduled_at, timer.created_at + interval);

        assert!(!timer.is_expired());
        sleep(interval + Duration::from_millis(10)).await; // Sleep a bit longer
        assert!(timer.is_expired());
    }

    #[tokio::test]
    async fn timer_new_repeating() {
        let tx_key = dummy_tx_key("repeating");
        let interval = Duration::from_millis(50);
        let timer = Timer::new_repeating("test_repeating", tx_key.clone(), interval);

        assert!(timer.repeating);
        assert_eq!(timer.interval, Some(interval));
        assert_eq!(timer.current_interval, Some(interval));
        assert_eq!(timer.timer_type, TimerType::Custom);
        assert_eq!(timer.scheduled_at, timer.created_at + interval);
    }

    #[tokio::test]
    async fn timer_new_backoff() {
        let tx_key = dummy_tx_key("backoff");
        let initial_interval = Duration::from_millis(50);
        let max_interval = Duration::from_millis(200);
        let timer = Timer::new_backoff("test_backoff", tx_key.clone(), initial_interval, max_interval);

        assert!(timer.repeating);
        assert_eq!(timer.interval, Some(initial_interval));
        assert_eq!(timer.current_interval, Some(initial_interval));
        assert_eq!(timer.max_interval, Some(max_interval));
        assert_eq!(timer.timer_type, TimerType::Custom);
        assert_eq!(timer.scheduled_at, timer.created_at + initial_interval);
    }

    #[test]
    fn timer_new_with_type() {
        let tx_key = dummy_tx_key("with_type");
        let interval = Duration::from_millis(100);

        // Test Retransmission type
        let timer_retransmit = Timer::new_with_type("retransmit_timer", tx_key.clone(), interval, TimerType::Retransmission);
        assert!(timer_retransmit.repeating);
        assert_eq!(timer_retransmit.current_interval, Some(interval));
        assert_eq!(timer_retransmit.timer_type, TimerType::Retransmission);

        // Test a non-retransmission type (e.g., TimerB - transaction timeout)
        let timer_b = Timer::new_with_type("timer_b", tx_key.clone(), interval, TimerType::B);
        assert!(!timer_b.repeating); // Based on current logic in new_with_type
        assert!(timer_b.current_interval.is_none());
        assert_eq!(timer_b.timer_type, TimerType::B);
    }

    #[test]
    fn timer_next_backoff_interval() {
        let tx_key = dummy_tx_key("next_backoff");
        let initial = Duration::from_millis(50);
        let max = Duration::from_millis(300);
        let mut timer = Timer::new_backoff("backoff_test", tx_key, initial, max);

        assert_eq!(timer.current_interval, Some(initial));

        // 1st backoff: 50 * 2 = 100
        let next1 = timer.next_backoff_interval();
        assert_eq!(next1, Duration::from_millis(100));
        assert_eq!(timer.current_interval, Some(Duration::from_millis(100)));

        // 2nd backoff: 100 * 2 = 200
        let next2 = timer.next_backoff_interval();
        assert_eq!(next2, Duration::from_millis(200));
        assert_eq!(timer.current_interval, Some(Duration::from_millis(200)));

        // 3rd backoff: 200 * 2 = 400, capped at 300
        let next3 = timer.next_backoff_interval();
        assert_eq!(next3, Duration::from_millis(300));
        assert_eq!(timer.current_interval, Some(Duration::from_millis(300)));
        
        // 4th backoff: 300 * 2 = 600, capped at 300
        let next4 = timer.next_backoff_interval();
        assert_eq!(next4, Duration::from_millis(300));
        assert_eq!(timer.current_interval, Some(Duration::from_millis(300)));
        
        // Test case where current_interval is None (should use interval then, if current logic holds)
        let mut timer_no_current = Timer::new_one_shot("no_current", dummy_tx_key("nc"), Duration::from_millis(70));
        timer_no_current.current_interval = None;
        timer_no_current.interval = Some(Duration::from_millis(70)); // Ensure interval is set
        timer_no_current.max_interval = Some(Duration::from_millis(500)); // Add max_interval for backoff test
        
        // Current logic: interval.unwrap_or_else(|| Duration::from_millis(500)); (if current_interval is None, uses interval if present)
        // Then it doubles that. So, 70 * 2 = 140.
        let next_fallback = timer_no_current.next_backoff_interval();
        assert_eq!(next_fallback, Duration::from_millis(70)); // Oh, wait, the current logic IS: self.current_interval = Some(interval); interval. So it returns the base.
                                                                // And sets current_interval to 70. The NEXT call would double it.
        assert_eq!(timer_no_current.current_interval, Some(Duration::from_millis(70)));
        let next_fallback_2 = timer_no_current.next_backoff_interval();
        assert_eq!(next_fallback_2, Duration::from_millis(140));
        assert_eq!(timer_no_current.current_interval, Some(Duration::from_millis(140)));
    }

    #[tokio::test]
    async fn timer_reschedule() {
        let tx_key = dummy_tx_key("reschedule");
        let interval = Duration::from_millis(50);
        let mut timer = Timer::new_repeating("test_reschedule", tx_key.clone(), interval);
        
        let initial_scheduled_at = timer.scheduled_at;
        sleep(Duration::from_millis(10)).await; // Let some time pass
        
        timer.reschedule();
        assert!(timer.scheduled_at > initial_scheduled_at);
        assert!(timer.scheduled_at > Instant::now() - Duration::from_millis(5)); // Check it's roughly now + interval
        assert!(timer.scheduled_at <= Instant::now() + interval);

        // Test with current_interval different from interval (e.g. after a backoff)
        let mut backoff_timer = Timer::new_backoff("test_resched_backoff", tx_key, Duration::from_millis(20), Duration::from_millis(100));
        backoff_timer.next_backoff_interval(); // current_interval is now 40ms
        assert_eq!(backoff_timer.current_interval, Some(Duration::from_millis(40)));
        
        sleep(Duration::from_millis(5)).await;
        let now_before_reschedule = Instant::now();
        backoff_timer.reschedule();
        assert!(backoff_timer.scheduled_at >= now_before_reschedule + Duration::from_millis(40));
    }
    
    #[tokio::test]
    async fn timer_reschedule_with_interval() {
        let tx_key = dummy_tx_key("reschedule_with");
        let initial_interval = Duration::from_millis(50);
        let mut timer = Timer::new_one_shot("test_res_int", tx_key.clone(), initial_interval); // Start as one-shot

        let new_interval = Duration::from_millis(100);
        sleep(Duration::from_millis(10)).await; // Simulate some time passing
        let now_before_reschedule = Instant::now(); // Capture time just before
        timer.reschedule_with_interval(new_interval); // This calls Instant::now() internally
        
        // Check that scheduled_at is roughly now_before_reschedule + new_interval
        // It should be slightly after due to Instant::now() call inside reschedule_with_interval
        assert!(timer.scheduled_at >= now_before_reschedule + new_interval, "Scheduled_at should be at or after expected time");
        // Add a small tolerance for the upper bound, e.g., 10 milliseconds
        let tolerance = Duration::from_millis(10);
        assert!(
            timer.scheduled_at < now_before_reschedule + new_interval + tolerance,
            "Scheduled_at ({:?}) should be within tolerance of expected time + tolerance ({:?})",
            timer.scheduled_at,
            now_before_reschedule + new_interval + tolerance
        );

        assert!(!timer.repeating); // Should still be false
        assert_eq!(timer.current_interval, None); // Should be None for non-repeating

        // Test with a repeating timer
        let mut repeating_timer = Timer::new_repeating("test_res_int_rep", tx_key, initial_interval);
        sleep(Duration::from_millis(10)).await;
        let now_before_reschedule_rep = Instant::now();
        repeating_timer.reschedule_with_interval(new_interval);

        // Apply similar tolerance check for repeating timer
        assert!(repeating_timer.scheduled_at >= now_before_reschedule_rep + new_interval, "Repeating: Scheduled_at should be at or after expected time");
        assert!(
            repeating_timer.scheduled_at < now_before_reschedule_rep + new_interval + tolerance,
            "Repeating: Scheduled_at ({:?}) should be within tolerance of expected time + tolerance ({:?})",
            repeating_timer.scheduled_at,
            now_before_reschedule_rep + new_interval + tolerance
        );
        assert!(repeating_timer.repeating);
        assert_eq!(repeating_timer.current_interval, Some(new_interval));
    }

    #[tokio::test]
    async fn timer_is_expired_and_time_remaining() {
        let tx_key = dummy_tx_key("expired_test");
        let interval = Duration::from_millis(50);
        let timer = Timer::new_one_shot("test_expiry", tx_key, interval);

        assert!(!timer.is_expired());
        assert!(timer.time_remaining() <= interval);
        assert!(timer.time_remaining() > Duration::from_millis(0));

        sleep(interval / 2).await;
        assert!(!timer.is_expired());
        assert!(timer.time_remaining() < interval);
        assert!(timer.time_remaining() > Duration::from_millis(0));
        
        sleep(interval / 2 + Duration::from_millis(10)).await; // Sleep past expiry
        assert!(timer.is_expired());
        assert_eq!(timer.time_remaining(), Duration::from_secs(0));
    }

    #[test]
    fn timer_display() {
        let tx_key = dummy_tx_key("display");
        let interval = Duration::from_millis(100);
        let timer = Timer::new_one_shot("display_timer", tx_key, interval);
        // Exact time_remaining can be tricky, so check for expected parts
        let display_str = timer.to_string();
        assert!(display_str.starts_with("Timer(display_timer, tx=Key(branch-display:INVITE:client)"));
        assert!(display_str.contains("repeating=false"));
        assert!(display_str.contains("scheduled=")); // e.g., scheduled=99ms) or scheduled=100ms)
        assert!(display_str.ends_with("ms)"));
    }

    #[test]
    fn timer_settings_default() {
        let settings = TimerSettings::default();
        assert_eq!(settings.t1, Duration::from_millis(500));
        assert_eq!(settings.t2, Duration::from_secs(4));
        assert_eq!(settings.transaction_timeout, Duration::from_secs(32));
        assert_eq!(settings.wait_time_d, Duration::from_secs(32));
        assert_eq!(settings.wait_time_h, Duration::from_secs(32));
        assert_eq!(settings.wait_time_i, Duration::from_secs(5));
        assert_eq!(settings.wait_time_j, Duration::from_secs(32));
        assert_eq!(settings.wait_time_k, Duration::from_secs(5));
    }

    #[test]
    fn timer_settings_custom() {
        let settings = TimerSettings {
            t1: Duration::from_millis(100),
            t2: Duration::from_secs(1),
            transaction_timeout: Duration::from_secs(10),
            wait_time_d: Duration::from_secs(10),
            wait_time_h: Duration::from_secs(10),
            wait_time_i: Duration::from_secs(1),
            wait_time_j: Duration::from_secs(10),
            wait_time_k: Duration::from_secs(1),
        };
        assert_eq!(settings.t1, Duration::from_millis(100));
        assert_eq!(settings.wait_time_k, Duration::from_secs(1));
    }
} 