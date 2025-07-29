pub mod acelp_codebook;
pub mod adaptive_codebook;
pub mod gain;
pub mod lsp;
pub mod post_processing;
pub mod postfilter;
pub mod g729a_decoder;

// Re-export main decoder
pub use g729a_decoder::G729ADecoder;
