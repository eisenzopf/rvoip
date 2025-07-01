// Media module for SDP parsing
//
// This module handles all media-related SDP parsing, including:
// - Media types (audio, video, etc.)
// - Transport protocols
// - Media formats
// - Media descriptions

pub mod types;
mod transport;
mod format;
mod description;
mod attributes;
mod utils;

// Re-export public API
pub use self::description::{parse_media_description_line, parse_media_description_nom};
pub use self::attributes::{update_media_with_attribute, parse_media_attributes, is_media_level_attribute};
pub use self::utils::{is_valid_token, is_valid_id, is_valid_ice_char, is_valid_ice_string, tag_no_case}; 