pub mod config;
pub mod providers;
pub mod pipeline;
pub mod db;

// Re-exports
pub use config::AiCopilotConfig;
pub use providers::{VoiceAiProvider, VoiceAiEvent, CallContext, Message};
pub use pipeline::CopilotPipeline;
