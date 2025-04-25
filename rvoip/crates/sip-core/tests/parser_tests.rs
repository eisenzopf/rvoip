// Entry point for all parser integration tests
mod common;
mod parser;

// Re-export the tests to make them available for direct testing
pub use parser::headers_test;
pub use parser::uri_test;
pub use parser::sdp_test; 