// Media module for SDP parsing
//
// This module handles all media-related SDP parsing, including:
// - Media types (audio, video, etc.)
// - Transport protocols
// - Media formats
// - Media descriptions

mod attributes;
mod description;
mod format;
mod transport;
pub mod types;
mod utils;

// Re-export public API
pub use self::attributes::{
    is_media_level_attribute, parse_media_attributes, update_media_with_attribute,
};
pub use self::description::{parse_media_description_line, parse_media_description_nom};
pub use self::utils::{
    is_valid_ice_char, is_valid_ice_string, is_valid_id, is_valid_token, tag_no_case,
};
