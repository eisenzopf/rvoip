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
//! ```rust
//! // Server creation
//! let server_manager = create_full_server_manager(transaction_manager, config).await?;
//!
//! // Agent registration  
//! let agent_session = server_manager.session_manager().create_outgoing_session().await?;
//!
//! // Call bridging
//! let bridge_id = server_manager.bridge_sessions(&customer_session, &agent_session).await?;
//!
//! // Event monitoring
//! let events = server_manager.subscribe_to_bridge_events().await;
//! ```
//!
//! Note: Previously this module contained adapter stubs, but we now use
//! session-core APIs directly for better performance and maintainability. 