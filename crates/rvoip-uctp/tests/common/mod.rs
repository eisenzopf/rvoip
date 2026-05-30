//! Shared test helpers for coordinator-driven integration tests.

use chrono::Utc;
use rvoip_uctp::{envelope::UctpEnvelope, payloads::auth, types::MessageType};
use tokio::sync::mpsc;

