use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};
use std::path::Path;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::net::SocketAddr;

use tokio::sync::RwLock;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use tracing::{debug, error, warn};
use async_trait::async_trait;

use crate::call::{Call, CallState, CallDirection, WeakCall, CallRegistryInterface};
use crate::error::{Result, Error};

/// Record of a call state change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallStateRecord {
    /// Timestamp when the state change occurred
    pub timestamp: SystemTime,
    /// Previous state
    pub previous_state: CallState,
    /// New state 
    pub new_state: CallState,
}

/// Transaction information record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    /// Transaction ID
    pub transaction_id: String,
    /// Transaction type (e.g., "INVITE", "BYE", "INFO")
    pub transaction_type: String,
    /// Timestamp when the transaction was created
    pub timestamp: SystemTime,
    /// Direction (incoming/outgoing)
    pub direction: CallDirection,
    /// Current status (e.g., "created", "completed", "terminated")
    pub status: String,
    /// Additional information about the transaction
    pub info: Option<String>,
    /// Destination address (for outgoing transactions)
    pub destination: Option<String>,
}

/// Call record with detailed information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    /// Call ID
    pub id: String,
    /// Call direction
    pub direction: CallDirection,
    /// Remote URI
    pub remote_uri: String,
    /// Start time
    pub start_time: SystemTime,
    /// End time (if call is terminated)
    pub end_time: Option<SystemTime>,
    /// Current state
    pub state: CallState,
    /// State history
    pub state_history: Vec<CallStateRecord>,
    /// Duration (if call is established)
    pub duration: Option<Duration>,
    /// SIP call ID
    pub sip_call_id: String,
    /// Transaction history
    pub transactions: Vec<TransactionRecord>,
    /// Initial INVITE transaction ID (if known)
    pub invite_transaction_id: Option<String>,
}

/// Call filter criteria
#[derive(Debug, Clone, Default)]
pub struct CallFilter {
    /// Filter by call state
    pub state: Option<CallState>,
    /// Filter by call direction
    pub direction: Option<CallDirection>,
    /// Filter by remote URI (substring match)
    pub remote_uri: Option<String>,
    /// Filter by start time (calls after this time)
    pub start_time_after: Option<SystemTime>,
    /// Filter by start time (calls before this time)
    pub start_time_before: Option<SystemTime>,
    /// Filter by minimum duration
    pub min_duration: Option<Duration>,
    /// Filter by maximum duration
    pub max_duration: Option<Duration>,
}

/// Call statistics data
#[derive(Debug, Clone, Default)]
pub struct CallStatistics {
    /// Total number of calls
    pub total_calls: usize,
    /// Number of incoming calls
    pub incoming_calls: usize,
    /// Number of outgoing calls
    pub outgoing_calls: usize,
    /// Number of established calls
    pub established_calls: usize,
    /// Number of failed calls
    pub failed_calls: usize,
    /// Number of missed calls (incoming calls that never reached established state)
    pub missed_calls: usize,
    /// Total call duration (for all established calls)
    pub total_duration: Duration,
    /// Average call duration
    pub average_duration: Option<Duration>,
    /// Maximum call duration
    pub max_duration: Option<Duration>,
    /// Minimum call duration
    pub min_duration: Option<Duration>,
    /// Calls by state counts
    pub calls_by_state: HashMap<CallState, usize>,
}

/// Result of a call lookup, containing the call record and optional references to the call itself
#[derive(Debug, Clone)]
pub struct CallLookupResult {
    /// The call record with historical information
    pub record: CallRecord,
    /// Strong reference to the call if it's active
    pub active_call: Option<Arc<Call>>,
    /// Weak reference to the call for memory-safe access
    pub weak_call: Option<WeakCall>,
}

/// Result of a call lookup, containing the call record and optional references to the call itself
/// 
/// This serializable version is used for API responses, omitting the actual call references
/// which cannot be serialized
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCallLookupResult {
    /// The call record with historical information
    pub record: CallRecord,
    /// Indicates if an active call reference is available (but not included)
    pub has_active_call: bool,
    /// Indicates if a weak call reference is available (but not included)
    pub has_weak_call: bool,
}

impl From<CallLookupResult> for SerializableCallLookupResult {
    fn from(result: CallLookupResult) -> Self {
        Self {
            record: result.record,
            has_active_call: result.active_call.is_some(),
            has_weak_call: result.weak_call.is_some(),
        }
    }
}

/// Call registry for storing call history
#[derive(Debug)]
pub struct CallRegistry {
    /// Active calls - strong references to ensure calls stay alive while active
    active_calls: RwLock<HashMap<String, Arc<Call>>>,
    
    /// Call references - weak call references that provide basic access
    weak_calls: RwLock<HashMap<String, WeakCall>>,
    
    /// Call history - data only, no references to actual call objects
    call_history: RwLock<HashMap<String, CallRecord>>,
    
    /// Max history size
    max_history_size: usize,
    
    /// Storage path for persistence
    storage_path: Option<String>,
    
    /// Last cleanup time
    last_cleanup: RwLock<SystemTime>,
}

impl CallRegistry {
    /// Create a new call registry
    pub fn new(max_history_size: usize) -> Self {
        Self {
            active_calls: RwLock::new(HashMap::new()),
            weak_calls: RwLock::new(HashMap::new()),
            call_history: RwLock::new(HashMap::new()),
            max_history_size,
            storage_path: None,
            last_cleanup: RwLock::new(SystemTime::now()),
        }
    }
    
    /// Create a new call registry with storage path for persistence
    pub fn with_storage(max_history_size: usize, storage_path: &str) -> Self {
        Self {
            active_calls: RwLock::new(HashMap::new()),
            weak_calls: RwLock::new(HashMap::new()),
            call_history: RwLock::new(HashMap::new()),
            max_history_size,
            storage_path: Some(storage_path.to_string()),
            last_cleanup: RwLock::new(SystemTime::now()),
        }
    }
    
    /// Load call history from storage
    pub async fn load_from_storage(&self) -> Result<()> {
        if let Some(path) = &self.storage_path {
            let path = Path::new(path);
            if !path.exists() {
                return Ok(());
            }
            
            let file = match File::open(path) {
                Ok(file) => file,
                Err(e) => {
                    return Err(Error::Storage(format!("Failed to open storage file: {}", e)));
                }
            };
            
            let reader = BufReader::new(file);
            let history: HashMap<String, CallRecord> = match serde_json::from_reader(reader) {
                Ok(history) => history,
                Err(e) => {
                    return Err(Error::Storage(format!("Failed to deserialize call history: {}", e)));
                }
            };
            
            // Update call history
            *self.call_history.write().await = history;
            
            Ok(())
        } else {
            Err(Error::Storage("No storage path configured".into()))
        }
    }
    
    /// Save call history to storage
    pub async fn save_to_storage(&self) -> Result<()> {
        if let Some(path) = &self.storage_path {
            let path = Path::new(path);
            
            // Create parent directory if it doesn't exist
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| {
                        Error::Storage(format!("Failed to create directory: {}", e))
                    })?;
                }
            }
            
            let file = match File::create(path) {
                Ok(file) => file,
                Err(e) => {
                    return Err(Error::Storage(format!("Failed to create storage file: {}", e)));
                }
            };
            
            let writer = BufWriter::new(file);
            let history = self.call_history.read().await;
            
            match serde_json::to_writer_pretty(writer, &*history) {
                Ok(_) => Ok(()),
                Err(e) => {
                    Err(Error::Storage(format!("Failed to serialize call history: {}", e)))
                }
            }
        } else {
            Err(Error::Storage("No storage path configured".into()))
        }
    }

    /// Register an active call
    pub async fn register_call(&self, call: Arc<Call>) -> Result<()> {
        let call_id = call.id().to_string();
        
        // Create an initial call record
        let call_record = CallRecord {
            id: call_id.clone(),
            direction: call.direction(),
            remote_uri: call.remote_uri().to_string(),
            start_time: SystemTime::now(),
            end_time: None,
            state: call.state().await,
            state_history: Vec::new(),
            duration: None,
            sip_call_id: call.sip_call_id().to_string(),
            transactions: Vec::new(),
            invite_transaction_id: None,
        };
        
        // Add to active calls (strong reference)
        self.active_calls.write().await.insert(call_id.clone(), call.clone());
        
        // Add weak reference for later access
        let weak_call = call.weak_clone();
        self.weak_calls.write().await.insert(call_id.clone(), weak_call);
        
        // Add to call history
        self.call_history.write().await.insert(call_id, call_record);
        
        // Try to save to storage if configured
        if self.storage_path.is_some() {
            let _ = self.save_to_storage().await;
        }
        
        // Perform cleanup periodically (not on every registration)
        let last = *self.last_cleanup.read().await;
        if SystemTime::now().duration_since(last).unwrap_or(Duration::from_secs(0)) > Duration::from_secs(300) {
            self.perform_cleanup().await;
        }
        
        Ok(())
    }

    /// Update call state
    pub async fn update_call_state(&self, call_id: &str, previous_state: CallState, new_state: CallState) -> Result<()> {
        // Add state change to history
        let mut history = self.call_history.write().await;
        
        if let Some(record) = history.get_mut(call_id) {
            // Add state change to history
            record.state_history.push(CallStateRecord {
                timestamp: SystemTime::now(),
                previous_state,
                new_state,
            });
            
            // Update current state
            record.state = new_state;
            
            // Update end time if call is terminated
            if new_state == CallState::Terminated || new_state == CallState::Failed {
                record.end_time = Some(SystemTime::now());
                
                // Try to compute duration
                if let Some(start) = record.state_history.iter()
                    .find(|r| r.new_state == CallState::Established)
                    .map(|r| r.timestamp) {
                    if let Ok(duration) = SystemTime::now().duration_since(start) {
                        record.duration = Some(duration);
                    }
                }
                
                // Remove from active calls to release strong reference
                self.active_calls.write().await.remove(call_id);
            }
            
            // Try to save to storage if configured
            if self.storage_path.is_some() {
                let record_clone = record.clone();
                let self_clone = self.clone();
                tokio::spawn(async move {
                    // Only save terminated calls to avoid excessive writes
                    if record_clone.state == CallState::Terminated || 
                       record_clone.state == CallState::Failed {
                        let _ = self_clone.save_to_storage().await;
                    }
                });
            }
            
            Ok(())
        } else {
            Err(Error::Call(format!("Call record not found: {}", call_id)))
        }
    }

    /// Get active calls
    pub async fn active_calls(&self) -> HashMap<String, Arc<Call>> {
        // Return strong references - caller should be careful with these
        // Most use cases should prefer get_calls_with_weak_refs() instead
        debug!("Returning strong references to all active calls - use sparingly");
        self.active_calls.read().await.clone()
    }

    /// Get call history
    pub async fn call_history(&self) -> HashMap<String, CallRecord> {
        self.call_history.read().await.clone()
    }

    /// Get calls by state
    pub async fn get_calls_by_state(&self, state: CallState) -> HashMap<String, CallRecord> {
        let all_calls = self.call_history.read().await;
        all_calls
            .iter()
            .filter(|(_, record)| record.state == state)
            .map(|(id, record)| (id.clone(), record.clone()))
            .collect()
    }

    /// Get recent calls within the specified duration from now
    pub async fn get_recent_calls(&self, duration: Duration) -> HashMap<String, CallRecord> {
        // Calculate the time threshold by subtracting the duration from current time
        let now = SystemTime::now();
        let threshold = now.checked_sub(duration)
            .unwrap_or_else(|| SystemTime::UNIX_EPOCH);
        
        let all_calls = self.call_history.read().await;
        
        all_calls
            .iter()
            .filter(|(_, record)| record.start_time >= threshold)
            .map(|(id, record)| (id.clone(), record.clone()))
            .collect()
    }

    /// Get recent calls with a specific state
    pub async fn get_recent_calls_by_state(&self, duration: Duration, state: CallState) -> HashMap<String, CallRecord> {
        // Calculate the time threshold
        let now = SystemTime::now();
        let threshold = now.checked_sub(duration)
            .unwrap_or_else(|| SystemTime::UNIX_EPOCH);
        
        let all_calls = self.call_history.read().await;
        
        all_calls
            .iter()
            .filter(|(_, record)| record.start_time >= threshold && record.state == state)
            .map(|(id, record)| (id.clone(), record.clone()))
            .collect()
    }

    /// Get active calls by state 
    pub async fn get_active_calls_by_state(&self, state: CallState) -> HashMap<String, Arc<Call>> {
        let mut result = HashMap::new();
        let active_calls = self.active_calls.read().await;
        
        for (id, call) in active_calls.iter() {
            if call.state().await == state {
                result.insert(id.clone(), call.clone());
            }
        }
        
        result
    }

    /// Get active calls with reference safety
    pub async fn get_calls_with_references(&self) -> HashMap<String, Arc<Call>> {
        let mut result = HashMap::new();
        
        // First add all active calls
        for (id, call) in self.active_calls.read().await.iter() {
            result.insert(id.clone(), call.clone());
        }
        
        // Try to upgrade any weak references that aren't already included
        let refs = self.weak_calls.read().await;
        for (id, weak_call) in refs.iter() {
            if !result.contains_key::<String>(id) {
                if let Some(call) = weak_call.upgrade() {
                    result.insert(id.clone(), call);
                }
            }
        }
        
        result
    }

    /// Get call by ID (active or history)
    pub async fn get_call(&self, call_id: &str) -> Option<CallRecord> {
        self.call_history.read().await.get(call_id).cloned()
    }

    /// Get active call by ID
    pub async fn get_active_call(&self, call_id: &str) -> Option<Arc<Call>> {
        self.active_calls.read().await.get(call_id).cloned()
    }

    /// Get weak call reference by ID
    pub async fn get_weak_call(&self, call_id: &str) -> Option<WeakCall> {
        self.weak_calls.read().await.get(call_id).cloned()
    }

    /// Get call reference, attempting to upgrade from weak reference if not active
    pub async fn get_call_reference(&self, call_id: &str) -> Option<Arc<Call>> {
        // First check active calls (strong refs)
        if let Some(call) = self.active_calls.read().await.get(call_id) {
            return Some(call.clone());
        }
        
        // Try to get and upgrade the weak reference
        if let Some(weak_call) = self.weak_calls.read().await.get(call_id).cloned() {
            return weak_call.upgrade();
        }
        
        None
    }

    /// Clean up old call history to maintain max size
    pub async fn cleanup_history(&self) {
        let mut history = self.call_history.write().await;
        
        if history.len() > self.max_history_size {
            // Sort calls by end time or start time
            let mut calls: Vec<_> = history.values().cloned().collect();
            calls.sort_by(|a, b| {
                let a_time = a.end_time.unwrap_or(a.start_time);
                let b_time = b.end_time.unwrap_or(b.start_time);
                a_time.cmp(&b_time)
            });
            
            // Remove oldest calls until we're under the limit
            let to_remove = calls.len() - self.max_history_size;
            for i in 0..to_remove {
                let id = calls[i].id.clone();
                history.remove(&id);
                
                // Also remove from weak_calls
                self.weak_calls.write().await.remove(&id);
            }
        }
        
        // Try to save to storage if configured
        if self.storage_path.is_some() {
            let self_clone = self.clone();
            tokio::spawn(async move {
                let _ = self_clone.save_to_storage().await;
            });
        }
    }

    /// Filter calls by criteria
    pub async fn filter_calls(&self, filter: &CallFilter) -> HashMap<String, CallRecord> {
        let all_calls = self.call_history.read().await;
        
        all_calls
            .iter()
            .filter(|(_, record)| {
                // Filter by state if specified
                if let Some(state) = filter.state {
                    if record.state != state {
                        return false;
                    }
                }
                
                // Filter by direction if specified
                if let Some(direction) = filter.direction {
                    if record.direction != direction {
                        return false;
                    }
                }
                
                // Filter by remote URI if specified
                if let Some(ref uri) = filter.remote_uri {
                    if !record.remote_uri.contains(uri) {
                        return false;
                    }
                }
                
                // Filter by start time range
                if let Some(after) = filter.start_time_after {
                    if record.start_time < after {
                        return false;
                    }
                }
                
                if let Some(before) = filter.start_time_before {
                    if record.start_time > before {
                        return false;
                    }
                }
                
                // Filter by duration if the call has a duration
                if let Some(duration) = record.duration {
                    if let Some(min_duration) = filter.min_duration {
                        if duration < min_duration {
                            return false;
                        }
                    }
                    
                    if let Some(max_duration) = filter.max_duration {
                        if duration > max_duration {
                            return false;
                        }
                    }
                } else if filter.min_duration.is_some() {
                    // If min_duration is specified but call has no duration, exclude it
                    return false;
                }
                
                true
            })
            .map(|(id, record)| (id.clone(), record.clone()))
            .collect()
    }

    /// Get calls within a specific time range
    pub async fn get_calls_in_time_range(&self, start_time: SystemTime, end_time: SystemTime) -> HashMap<String, CallRecord> {
        let all_calls = self.call_history.read().await;
        
        all_calls
            .iter()
            .filter(|(_, record)| {
                // Call must start after start_time
                if record.start_time < start_time {
                    return false;
                }
                
                // Call must start before end_time
                if record.start_time > end_time {
                    return false;
                }
                
                true
            })
            .map(|(id, record)| (id.clone(), record.clone()))
            .collect()
    }
    
    /// Calculate statistics for all calls in the registry
    pub async fn calculate_statistics(&self) -> CallStatistics {
        let all_calls = self.call_history.read().await;
        self.calculate_statistics_for_records(&all_calls)
    }
    
    /// Calculate statistics for calls in a time range
    pub async fn calculate_statistics_in_time_range(&self, start_time: SystemTime, end_time: SystemTime) -> CallStatistics {
        let calls_in_range = self.get_calls_in_time_range(start_time, end_time).await;
        self.calculate_statistics_for_records(&calls_in_range)
    }
    
    /// Calculate statistics for recent calls
    pub async fn calculate_recent_statistics(&self, duration: Duration) -> CallStatistics {
        let recent_calls = self.get_recent_calls(duration).await;
        self.calculate_statistics_for_records(&recent_calls)
    }
    
    /// Helper method to calculate statistics for a collection of call records
    fn calculate_statistics_for_records(&self, records: &HashMap<String, CallRecord>) -> CallStatistics {
        let mut stats = CallStatistics::default();
        let mut durations = Vec::new();
        
        stats.total_calls = records.len();
        
        // Initialize calls_by_state with all states at 0
        for state in [
            CallState::Initial, CallState::Ringing, CallState::Connecting,
            CallState::Established, CallState::Terminating, 
            CallState::Terminated, CallState::Failed,
        ] {
            stats.calls_by_state.insert(state, 0);
        }
        
        for record in records.values() {
            // Count by direction
            match record.direction {
                CallDirection::Incoming => stats.incoming_calls += 1,
                CallDirection::Outgoing => stats.outgoing_calls += 1,
            }
            
            // Count by state
            if let Some(count) = stats.calls_by_state.get_mut(&record.state) {
                *count += 1;
            }
            
            // Count established/failed calls
            match record.state {
                CallState::Established => stats.established_calls += 1,
                CallState::Failed => stats.failed_calls += 1,
                _ => {}
            }
            
            // Calculate missed calls (incoming that never reached established)
            if record.direction == CallDirection::Incoming && 
               !record.state_history.iter().any(|s| s.new_state == CallState::Established) {
                stats.missed_calls += 1;
            }
            
            // Collect durations for established calls with a duration
            if let Some(duration) = record.duration {
                stats.total_duration += duration;
                durations.push(duration);
            }
        }
        
        // Calculate duration statistics if we have any durations
        if !durations.is_empty() {
            // Calculate average
            stats.average_duration = Some(
                Duration::from_secs_f64(stats.total_duration.as_secs_f64() / durations.len() as f64)
            );
            
            // Find min and max
            stats.min_duration = durations.iter().min().cloned();
            stats.max_duration = durations.iter().max().cloned();
        }
        
        stats
    }

    /// Perform comprehensive cleanup
    async fn perform_cleanup(&self) {
        debug!("Performing call registry cleanup");
        
        // Update the last cleanup time
        *self.last_cleanup.write().await = SystemTime::now();
        
        // Check active calls to find terminated ones
        let mut active_to_remove = Vec::new();
        {
            let active_calls = self.active_calls.read().await;
            for (id, call) in active_calls.iter() {
                if self.check_and_update_call_status(id, call).await {
                    active_to_remove.push(id.clone());
                }
            }
        }
        
        // Remove terminated calls from active_calls
        if !active_to_remove.is_empty() {
            debug!("Removing {} terminated calls from active_calls", active_to_remove.len());
            let mut active_calls = self.active_calls.write().await;
            for id in active_to_remove {
                active_calls.remove(&id);
            }
        }
        
        // Clean up weak references that can't be upgraded
        let mut refs_to_remove = Vec::new();
        {
            let weak_refs = self.weak_calls.read().await;
            for (id, weak_call) in weak_refs.iter() {
                if weak_call.upgrade().is_none() {
                    refs_to_remove.push(id.clone());
                }
            }
        }
        
        if !refs_to_remove.is_empty() {
            debug!("Removing {} dangling weak references", refs_to_remove.len());
            let mut weak_refs = self.weak_calls.write().await;
            for id in refs_to_remove {
                weak_refs.remove(&id);
            }
        }
        
        // Run the standard history cleanup
        self.cleanup_history().await;
    }

    /// Check if a call has terminated and should be removed from active calls
    async fn check_and_update_call_status(&self, call_id: &str, call: &Arc<Call>) -> bool {
        // Check if the call is in a terminal state but still in active_calls
        match call.state().await {
            CallState::Terminated | CallState::Failed => {
                // Update the call record if needed
                let mut history = self.call_history.write().await;
                if let Some(record) = history.get_mut(call_id) {
                    if record.state != CallState::Terminated && record.state != CallState::Failed {
                        record.state = call.state().await;
                        record.end_time = Some(SystemTime::now());
                        
                        // Try to get duration if established
                        if record.state_history.iter().any(|s| s.new_state == CallState::Established) {
                            if let Some(connect_time) = record.state_history.iter()
                                .find(|s| s.new_state == CallState::Established)
                                .map(|s| s.timestamp) {
                                if let Ok(duration) = SystemTime::now().duration_since(connect_time) {
                                    record.duration = Some(duration);
                                }
                            }
                        }
                    }
                }
                
                // It's terminated, should be removed from active_calls
                true
            },
            _ => false
        }
    }

    /// Get calls with weak references
    pub async fn get_calls_with_weak_refs(&self) -> HashMap<String, WeakCall> {
        // Return weak references - preferred approach for callers that don't need ownership
        debug!("Returning weak references to calls - preferred approach");
        self.weak_calls.read().await.clone()
    }

    /// Find a call by its ID, searching in both active calls and call history
    /// 
    /// This method first checks for an active call with the given ID, then falls back
    /// to the call history if not found as active. It returns both the historical record
    /// and any available references to the actual call object.
    /// 
    /// # Parameters
    /// * `call_id` - The ID of the call to find
    /// 
    /// # Returns
    /// * `Some(CallLookupResult)` - If the call was found, containing the record and any references
    /// * `None` - If no call with the given ID exists in either active calls or history
    pub async fn find_call_by_id(&self, call_id: &str) -> Option<CallLookupResult> {
        // First check if this is an active call
        let active_call = self.active_calls.read().await.get(call_id).cloned();
        
        // Also check if we have a weak reference
        let weak_call = self.weak_calls.read().await.get(call_id).cloned();
        
        // Check if we have the call in history
        let record = match self.call_history.read().await.get(call_id) {
            Some(record) => record.clone(),
            None => {
                // If we found an active call but no record, this is an error condition
                // We should always have a record for any call in the system
                if active_call.is_some() || weak_call.is_some() {
                    warn!("Found call references for ID {} but no historical record", call_id);
                    
                    // If we have an active call, we can create a basic record from it
                    if let Some(call) = &active_call {
                        let state = call.state().await;
                        let record = CallRecord {
                            id: call_id.to_string(),
                            direction: call.direction(),
                            remote_uri: call.remote_uri().to_string(),
                            start_time: SystemTime::now(),  // Approximation
                            end_time: None,
                            state,
                            state_history: Vec::new(),
                            duration: None,
                            sip_call_id: call.sip_call_id().to_string(),
                            transactions: Vec::new(),
                            invite_transaction_id: None,
                        };
                        return Some(CallLookupResult {
                            record,
                            active_call,
                            weak_call,
                        });
                    }
                }
                return None;
            }
        };

        Some(CallLookupResult {
            record,
            active_call,
            weak_call,
        })
    }

    /// Log a transaction for a call
    pub async fn log_transaction(&self, call_id: &str, transaction: TransactionRecord) -> Result<()> {
        let mut history = self.call_history.write().await;
        
        if let Some(record) = history.get_mut(call_id) {
            // Add transaction to history
            record.transactions.push(transaction.clone());
            
            // If this is an INVITE transaction, store it specifically
            if transaction.transaction_type == "INVITE" {
                record.invite_transaction_id = Some(transaction.transaction_id.clone());
            }
            
            // Try to save to storage if configured
            if self.storage_path.is_some() {
                let self_clone = self.clone();
                tokio::spawn(async move {
                    let _ = self_clone.save_to_storage().await;
                });
            }
            
            Ok(())
        } else {
            Err(Error::Call(format!("Call record not found: {}", call_id)))
        }
    }
    
    /// Get transactions for a call
    pub async fn get_transactions(&self, call_id: &str) -> Result<Vec<TransactionRecord>> {
        let history = self.call_history.read().await;
        
        if let Some(record) = history.get(call_id) {
            Ok(record.transactions.clone())
        } else {
            Err(Error::Call(format!("Call record not found: {}", call_id)))
        }
    }
    
    /// Get a specific transaction by ID
    pub async fn get_transaction(&self, call_id: &str, transaction_id: &str) -> Result<Option<TransactionRecord>> {
        let history = self.call_history.read().await;
        
        if let Some(record) = history.get(call_id) {
            let transaction = record.transactions.iter()
                .find(|t| t.transaction_id == transaction_id)
                .cloned();
            
            Ok(transaction)
        } else {
            Err(Error::Call(format!("Call record not found: {}", call_id)))
        }
    }
    
    /// Update transaction status
    pub async fn update_transaction_status(&self, call_id: &str, transaction_id: &str, status: &str, info: Option<String>) -> Result<()> {
        let mut history = self.call_history.write().await;
        
        if let Some(record) = history.get_mut(call_id) {
            let found = record.transactions.iter_mut()
                .find(|t| t.transaction_id == transaction_id);
            
            if let Some(transaction) = found {
                transaction.status = status.to_string();
                if let Some(info_str) = info {
                    transaction.info = Some(info_str);
                }
                
                // Try to save to storage if configured
                if self.storage_path.is_some() {
                    let self_clone = self.clone();
                    tokio::spawn(async move {
                        let _ = self_clone.save_to_storage().await;
                    });
                }
                
                Ok(())
            } else {
                Err(Error::Call(format!("Transaction not found: {}", transaction_id)))
            }
        } else {
            Err(Error::Call(format!("Call record not found: {}", call_id)))
        }
    }

    /// Get transaction destination (SocketAddr) from the registry, used for ACK fallback
    async fn get_transaction_destination(&self, call_id: &str) -> Result<Option<SocketAddr>> {
        let calls = self.call_history.read().await;
        
        // Look for the call
        if let Some(call_record) = calls.get(call_id) {
            // Look for any INVITE transaction with a destination
            for tx in &call_record.transactions {
                // Prioritize INVITE transactions as they're most likely to have the correct destination
                if tx.transaction_type == "INVITE" && tx.destination.is_some() {
                    if let Some(dest_str) = &tx.destination {
                        // Try to parse the destination string as a SocketAddr
                        if let Ok(addr) = dest_str.parse::<SocketAddr>() {
                            debug!("Found destination for call {}: {}", call_id, addr);
                            return Ok(Some(addr));
                        }
                    }
                }
            }
            
            // If no INVITE transaction found, try any transaction with a destination
            for tx in &call_record.transactions {
                if let Some(dest_str) = &tx.destination {
                    // Try to parse the destination string as a SocketAddr
                    if let Ok(addr) = dest_str.parse::<SocketAddr>() {
                        debug!("Found fallback destination for call {}: {}", call_id, addr);
                        return Ok(Some(addr));
                    }
                }
            }
            
            // No suitable transaction found, but the call exists
            debug!("No transaction with destination found for call {}", call_id);
            return Ok(None);
        }
        
        // Call not found
        debug!("Call {} not found in registry", call_id);
        Ok(None)
    }
}

impl Clone for CallRegistry {
    fn clone(&self) -> Self {
        Self {
            active_calls: RwLock::new(HashMap::new()),
            weak_calls: RwLock::new(HashMap::new()),
            call_history: RwLock::new(self.get_call_history_sync()),
            max_history_size: self.max_history_size,
            storage_path: self.storage_path.clone(),
            last_cleanup: RwLock::new(SystemTime::now()),
        }
    }
}

impl CallRegistry {
    // Synchronous helper for getting call history
    fn get_call_history_sync(&self) -> HashMap<String, CallRecord> {
        // Create a one-off runtime for the blocking read
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
            
        runtime.block_on(async {
            self.call_history.read().await.clone()
        })
    }
}

// Implement the CallRegistryInterface trait for CallRegistry
#[async_trait]
impl CallRegistryInterface for CallRegistry {
    async fn log_transaction(&self, call_id: &str, transaction: TransactionRecord) -> Result<()> {
        self.log_transaction(call_id, transaction).await
    }
    
    async fn get_transactions(&self, call_id: &str) -> Result<Vec<TransactionRecord>> {
        self.get_transactions(call_id).await
    }
    
    async fn update_transaction_status(&self, call_id: &str, transaction_id: &str, status: &str, info: Option<String>) -> Result<()> {
        self.update_transaction_status(call_id, transaction_id, status, info).await
    }
    
    async fn get_transaction_destination(&self, call_id: &str) -> Result<Option<SocketAddr>> {
        self.get_transaction_destination(call_id).await
    }
} 