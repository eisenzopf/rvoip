// Common test utilities for sip-core
use ordered_float::NotNan;
use std::net::IpAddr;
use std::str::FromStr;

// SIP Core imports using the rvoip_sip_core crate name
use rvoip_sip_core::prelude::{Message, Request, Response};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::types::param::GenericValue;
use rvoip_sip_core::types::uri::{Host, Uri};
use rvoip_sip_core::types::via::Via;
use rvoip_sip_core::types::{Address, Method, Param, StatusCode};
use rvoip_sip_core::{
    parse_message,
    types::header::{HeaderName, TypedHeader},
    Error as SipError, Result as SipResult,
};

// Use crate:: syntax as this will be part of the test crate
use std::fmt::{Debug, Display};

// --- Type Construction Helpers ---



// Param construction helpers

// --- Parser/FromStr Test Helpers ---



// --- Display Test Helper ---


// --- Message Test Helpers ---








// Helper to create a Via header
