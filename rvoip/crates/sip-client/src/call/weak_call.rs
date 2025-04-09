use std::sync::{Arc, Weak};
use std::net::SocketAddr;
use std::time::Instant;

use tokio::sync::{RwLock, watch};
use rvoip_sip_core::Uri;
use rvoip_transaction_core::TransactionManager;

use crate::error::{Error, Result};
use super::CallRegistryInterface;
use super::types::CallState;
use super::types::CallDirection;
use super::call_struct::Call;

/// A weak reference to a Call, safe to pass around without keeping the call alive
#[derive(Debug, Clone)]
pub struct WeakCall {
    /// Unique call ID (strong - small string)
    pub id: String,
    /// Call direction (copy type)
    pub direction: CallDirection,
    /// SIP call ID (strong - small string)
    pub sip_call_id: String,
    /// Local URI (strong - small)
    pub local_uri: Uri,
    /// Remote URI (strong - small)
    pub remote_uri: Uri,
    /// Remote address (copy type)
    pub remote_addr: SocketAddr,
    /// State watcher receiver (needs to be strong to receive updates)
    pub state_watcher: watch::Receiver<CallState>,
    
    // Weak references to internal state
    pub(crate) remote_tag: Weak<RwLock<Option<String>>>,
    pub(crate) state: Weak<RwLock<CallState>>,
    pub(crate) connect_time: Weak<RwLock<Option<Instant>>>,
    pub(crate) end_time: Weak<RwLock<Option<Instant>>>,
    
    // Registry reference (weak)
    pub(crate) registry: Weak<RwLock<Option<Arc<dyn CallRegistryInterface + Send + Sync>>>>,
    
    // Transaction manager reference (strong to ensure DTMF works)
    pub(crate) transaction_manager: Arc<TransactionManager>,
}

impl WeakCall {
    /// Get the call registry
    pub async fn registry(&self) -> Result<Option<Arc<dyn CallRegistryInterface + Send + Sync>>> {
        // Try to upgrade the weak reference
        match self.registry.upgrade() {
            Some(registry_lock) => {
                // Try to acquire the read lock
                match registry_lock.read().await.clone() {
                    Some(registry) => Ok(Some(registry)),
                    None => Ok(None),
                }
            },
            None => Ok(None),
        }
    }
    
    /// Get the current call state
    pub async fn state(&self) -> Result<CallState> {
        Ok(*self.state_watcher.borrow())
    }
    
    /// Get the SIP call ID
    pub fn sip_call_id(&self) -> String {
        self.sip_call_id.clone()
    }
    
    /// Hang up the call
    pub async fn hangup(&self) -> Result<()> {
        // Try to upgrade to a full Call
        if let Some(call) = self.upgrade() {
            call.hangup().await
        } else {
            Err(Error::Call("Cannot hang up: call no longer exists".into()))
        }
    }
    
    /// Wait until the call is established or fails
    pub async fn wait_until_established(&self) -> Result<()> {
        // First check if the call is already established using the state_watcher
        let current_state = *self.state_watcher.borrow();
        
        if current_state == CallState::Established {
            return Ok(());
        }
        
        if current_state == CallState::Terminated || current_state == CallState::Failed {
            return Err(Error::Call("Call terminated before being established".into()));
        }
        
        // Wait for state changes via the state_watcher
        let start = std::time::Instant::now();
        let timeout_duration = std::time::Duration::from_secs(30);
        
        // Create a clone of the state_watcher to avoid borrowing issues
        let mut watcher = self.state_watcher.clone();
        
        loop {
            // Check if we've exceeded the timeout
            if start.elapsed() > timeout_duration {
                return Err(Error::Timeout("Timed out waiting for call to establish".into()));
            }
            
            // Wait for the next state change
            if watcher.changed().await.is_err() {
                // The sender was dropped, which could mean the Call was dropped
                return Err(Error::Call("Call state watcher closed".into()));
            }
            
            // Check the new state
            let state = *watcher.borrow();
            match state {
                CallState::Established => {
                    return Ok(());
                },
                CallState::Terminated | CallState::Failed => {
                    return Err(Error::Call("Call terminated before being established".into()));
                },
                _ => {
                    // Continue waiting
                    continue;
                }
            }
        }
    }
    
    /// Upgrade a weak call reference to a strong Arc<Call> reference if possible
    pub fn upgrade(&self) -> Option<Arc<Call>> {
        // First check if we can upgrade the necessary components
        let state = match self.state.upgrade() {
            Some(state) => state,
            None => return None,
        };
        
        let remote_tag = match self.remote_tag.upgrade() {
            Some(remote_tag) => remote_tag,
            None => return None,
        };
        
        let connect_time = match self.connect_time.upgrade() {
            Some(connect_time) => connect_time,
            None => return None,
        };
        
        let end_time = match self.end_time.upgrade() {
            Some(end_time) => end_time,
            None => return None,
        };
        
        // Upgrade registry weak reference
        let registry_opt = self.registry.upgrade().and_then(|lock| {
            if let Ok(guard) = lock.try_read() {
                guard.clone()
            } else {
                None
            }
        });
        
        // Here we would need to reconstruct the original Call object
        // This is typically done in the Call implementation with a method to create
        // from a WeakCall, which we'll move to the Call struct
        None
    }
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        // Try to upgrade to a full Call
        if let Some(call) = self.upgrade() {
            call.send_dtmf(digit).await
        } else {
            Err(Error::Call("Cannot send DTMF: call no longer exists".into()))
        }
    }
} 