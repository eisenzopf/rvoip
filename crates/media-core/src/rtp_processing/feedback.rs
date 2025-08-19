//! Media feedback algorithms (moved from rtp-core)

/// Media feedback message
#[derive(Debug, Clone)]
pub struct MediaFeedback {
    /// Feedback type
    pub feedback_type: FeedbackType,
    
    /// Target session/stream
    pub session_id: String,
    
    /// Feedback payload
    pub payload: Vec<u8>,
}

/// Types of media feedback
#[derive(Debug, Clone)]
pub enum FeedbackType {
    /// Generic NACK (Negative Acknowledgment)
    Nack,
    
    /// Picture Loss Indication
    Pli,
    
    /// Full Intra Request
    Fir,
    
    /// Slice Loss Indication
    Sli,
    
    /// Reference Picture Selection Indication
    Rpsi,
}

/// Feedback generator for media quality control
pub struct FeedbackGenerator {
    session_id: String,
}

impl FeedbackGenerator {
    /// Create a new feedback generator
    pub fn new(session_id: String) -> Self {
        Self { session_id }
    }
    
    /// Generate NACK feedback for lost packets
    pub fn generate_nack(&self, lost_sequences: &[u16]) -> MediaFeedback {
        MediaFeedback {
            feedback_type: FeedbackType::Nack,
            session_id: self.session_id.clone(),
            payload: lost_sequences.iter().flat_map(|s| s.to_be_bytes()).collect(),
        }
    }
    
    /// Generate PLI feedback for picture loss
    pub fn generate_pli(&self) -> MediaFeedback {
        MediaFeedback {
            feedback_type: FeedbackType::Pli,
            session_id: self.session_id.clone(),
            payload: Vec::new(),
        }
    }
}