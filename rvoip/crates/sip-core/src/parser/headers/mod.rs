// Declare header parser modules
pub mod via;
/// Contact Header Parsing
pub mod contact;
pub mod from;
pub mod to;
pub mod route;
pub mod record_route;
pub mod cseq;
pub mod max_forwards;
pub mod expires;
pub mod content_length;
pub mod call_id;
pub mod min_expires;
pub mod mime_version;
pub mod priority;
pub mod subject;
pub mod timestamp;
pub mod user_agent;
pub mod server;
pub mod reply_to;
pub mod organization;
pub mod date;
pub mod allow;
pub mod require;
pub mod supported;
pub mod unsupported;
pub mod proxy_require;
pub mod in_reply_to;
pub mod content_type;
pub mod content_disposition;
pub mod accept;
pub mod accept_encoding;
pub mod accept_language;
pub mod content_encoding;
pub mod content_language;
pub mod alert_info;
pub mod call_info;
pub mod error_info;
pub mod warning;
pub mod retry_after;
pub mod auth; // Group for auth parsers
pub mod www_authenticate;
pub mod proxy_authenticate;
pub mod authorization;
pub mod proxy_authorization;
pub mod authentication_info;
pub mod uri_with_params; // Added

// Keep internal modules private
mod server_val;
mod token_list;
mod media_type;

// Re-export public parser functions
pub use via::parse_via;
pub use contact::contact::parse_contact;
pub use from::parse_from;
pub use to::parse_to;
pub use route::parse_route;
pub use record_route::parse_record_route;
pub use cseq::parse_cseq;
pub use max_forwards::parse_max_forwards;
pub use expires::parse_expires;
pub use content_length::parse_content_length;
pub use call_id::parse_call_id;
pub use min_expires::parse_min_expires;
pub use mime_version::parse_mime_version;
pub use priority::parse_priority;
pub use subject::parse_subject;
pub use timestamp::parse_timestamp;
pub use user_agent::parse_user_agent;
pub use server::parse_server;
pub use reply_to::parse_reply_to;
pub use organization::parse_organization;
pub use date::parse_date;
pub use allow::parse_allow;
pub use require::parse_require;
pub use supported::parse_supported;
pub use unsupported::parse_unsupported;
pub use proxy_require::parse_proxy_require;
pub use in_reply_to::parse_in_reply_to;
pub use content_type::parse_content_type;
pub use content_disposition::parse_content_disposition;
pub use accept::parse_accept;
pub use accept_encoding::parse_accept_encoding;
pub use accept_language::parse_accept_language;
pub use content_encoding::parse_content_encoding;
pub use content_language::parse_content_language;
pub use alert_info::parse_alert_info;
pub use call_info::parse_call_info;
pub use error_info::parse_error_info;
pub use warning::parse_warning;
pub use retry_after::parse_retry_after;
pub use www_authenticate::parse_www_authenticate;
pub use proxy_authenticate::parse_proxy_authenticate;
pub use authorization::parse_authorization;
pub use proxy_authorization::parse_proxy_authorization;
pub use authentication_info::parse_authentication_info;

// Re-export shared auth components if needed directly
// pub use auth::common::{auth_param, realm, nonce, ...};

// Re-export shared URI component parser if needed directly?
// pub use uri_with_params::uri_with_generic_params;