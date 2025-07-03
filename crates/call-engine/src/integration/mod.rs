//! Session-core integration module
//!
//! This module provides documentation and utilities for interfacing
//! with session-core functionality. The actual integration is performed
//! directly through session-core APIs in the CallCenterEngine.
//!
//! ## Integration Architecture
//!
//! The call-engine integrates with session-core through:
//!
//! ### Direct API Usage
//! - `ServerSessionManager` for session lifecycle management
//! - `IncomingCallNotification` trait for call routing decisions  
//! - Bridge APIs for connecting agents and customers
//! - Event system for real-time monitoring
//!
//! ### Key Integration Points
//! - **CallCenterEngine**: Main integration point using `create_full_server_manager()`
//! - **Agent Registration**: Using `create_outgoing_session()` for agent sessions
//! - **Call Bridging**: Using `bridge_sessions()` for agent-customer connections
//! - **Event Monitoring**: Using `subscribe_to_bridge_events()` for real-time updates
//!
//! ## Real Session-Core APIs Used
//!
//! ```rust,no_run
//! # use rvoip_call_engine::prelude::*;
//! # use rvoip_session_core::{SessionCoordinator, SessionId};
//! # use std::sync::Arc;
//! # async fn example() -> anyhow::Result<()> {
//! // Server creation - using high-level API
//! let session_coordinator = rvoip_session_core::SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_media_ports(10000, 11000)
//!     .build()
//!     .await?;
//!
//! // Agent registration  
//! let agent_session = session_coordinator.create_outgoing_session().await?;
//!
//! // Call bridging (assuming we have session IDs)
//! let customer_session_id = SessionId::new();
//! let agent_session_id = SessionId::new();
//! let bridge_id = session_coordinator.bridge_sessions(
//!     &customer_session_id,
//!     &agent_session_id
//! ).await?;
//!
//! // Event monitoring
//! let mut events = session_coordinator.subscribe_to_bridge_events().await;
//! # Ok(())
//! # }
//! ```
//!
//! Note: Previously this module contained adapter stubs, but we now use
//! session-core APIs directly for better performance and maintainability. 