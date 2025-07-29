pub mod common;
pub mod decoder;
pub mod encoder;
pub mod rust_test;

// Re-export main codec components
pub use encoder::g729a_encoder::G729AEncoder;
pub use decoder::G729ADecoder;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}