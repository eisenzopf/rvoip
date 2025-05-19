use std::time::{Duration, Instant};
use std::collections::VecDeque;

/// Network quality metrics
#[derive(Debug, Clone)]
pub struct NetworkMetrics {
    /// Packet loss rate (0.0-1.0)
    pub packet_loss: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: f32,
    /// Bandwidth estimate in kbps
    pub bandwidth_kbps: f32,
    /// Consecutive packet losses
    pub consecutive_losses: u32,
    /// Maximum packet loss burst observed
    pub max_loss_burst: u32,
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        Self {
            packet_loss: 0.0,
            jitter_ms: 0.0,
            rtt_ms: 0.0,
            bandwidth_kbps: 0.0,
            consecutive_losses: 0,
            max_loss_burst: 0,
        }
    }
}

/// Audio quality metrics
#[derive(Debug, Clone)]
pub struct AudioMetrics {
    /// Average energy level (0.0-1.0)
    pub avg_level: f32,
    /// Peak level (0.0-1.0)
    pub peak_level: f32,
    /// Speech activity ratio (0.0-1.0)
    pub speech_activity: f32,
    /// Signal-to-noise ratio in dB
    pub snr_db: f32,
    /// Estimated audio clarity (0.0-1.0)
    pub clarity: f32,
    /// Audio codec bitrate in kbps
    pub codec_bitrate_kbps: f32,
    /// Audio sample rate in Hz
    pub sample_rate_hz: u32,
}

impl Default for AudioMetrics {
    fn default() -> Self {
        Self {
            avg_level: 0.0,
            peak_level: 0.0,
            speech_activity: 0.0,
            snr_db: 30.0, // Default to a reasonable value
            clarity: 1.0,
            codec_bitrate_kbps: 64.0, // Default to common bitrate
            sample_rate_hz: 8000, // Default to narrowband
        }
    }
}

/// Complete quality metrics collection
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    /// Network metrics
    pub network: NetworkMetrics,
    /// Audio metrics
    pub audio: AudioMetrics,
    /// Timestamp when metrics were collected
    pub timestamp: Instant,
    /// Call duration in seconds
    pub duration_sec: u64,
    /// Metrics collection interval in milliseconds
    pub collection_interval_ms: u32,
}

impl Default for QualityMetrics {
    fn default() -> Self {
        Self {
            network: NetworkMetrics::default(),
            audio: AudioMetrics::default(),
            timestamp: Instant::now(),
            duration_sec: 0,
            collection_interval_ms: 1000, // Default to 1 second
        }
    }
}

/// Metric sample with timestamp
#[derive(Debug, Clone, Copy)]
struct MetricSample {
    /// The value
    value: f32,
    /// When it was recorded
    timestamp: Instant,
}

/// Tracks metrics over time with moving averages
#[derive(Debug)]
pub struct MetricsTracker {
    /// Network metrics history
    packet_loss_samples: VecDeque<MetricSample>,
    jitter_samples: VecDeque<MetricSample>,
    rtt_samples: VecDeque<MetricSample>,
    bandwidth_samples: VecDeque<MetricSample>,
    
    /// Audio metrics history
    level_samples: VecDeque<MetricSample>,
    speech_activity_samples: VecDeque<MetricSample>,
    snr_samples: VecDeque<MetricSample>,
    
    /// Aggregate metrics
    metrics: QualityMetrics,
    
    /// Maximum samples to keep for each metric
    max_samples: usize,
    
    /// Start time of the call
    start_time: Instant,
    
    /// Last update time
    last_update: Instant,
    
    /// Packet counters
    total_packets: u64,
    lost_packets: u64,
    
    /// Current consecutive loss count
    current_loss_burst: u32,
}

impl MetricsTracker {
    /// Create a new metrics tracker
    pub fn new(max_samples: usize) -> Self {
        let now = Instant::now();
        Self {
            packet_loss_samples: VecDeque::with_capacity(max_samples),
            jitter_samples: VecDeque::with_capacity(max_samples),
            rtt_samples: VecDeque::with_capacity(max_samples),
            bandwidth_samples: VecDeque::with_capacity(max_samples),
            
            level_samples: VecDeque::with_capacity(max_samples),
            speech_activity_samples: VecDeque::with_capacity(max_samples),
            snr_samples: VecDeque::with_capacity(max_samples),
            
            metrics: QualityMetrics::default(),
            max_samples,
            start_time: now,
            last_update: now,
            
            total_packets: 0,
            lost_packets: 0,
            current_loss_burst: 0,
        }
    }
    
    /// Update network metrics with new values
    pub fn update_network(
        &mut self,
        jitter_ms: Option<f32>,
        rtt_ms: Option<f32>,
        bandwidth_kbps: Option<f32>,
        packet_received: bool,
    ) {
        let now = Instant::now();
        
        // Track packet loss
        self.total_packets += 1;
        if !packet_received {
            self.lost_packets += 1;
            self.current_loss_burst += 1;
            
            // Update max loss burst if needed
            if self.current_loss_burst > self.metrics.network.max_loss_burst {
                self.metrics.network.max_loss_burst = self.current_loss_burst;
            }
        } else {
            // Reset consecutive loss counter
            self.current_loss_burst = 0;
        }
        
        // Update packet loss rate
        if self.total_packets > 0 {
            let packet_loss = self.lost_packets as f32 / self.total_packets as f32;
            self.add_sample(&mut self.packet_loss_samples, packet_loss, now);
            self.metrics.network.packet_loss = self.average(&self.packet_loss_samples);
        }
        
        // Update other network metrics if provided
        if let Some(jitter) = jitter_ms {
            self.add_sample(&mut self.jitter_samples, jitter, now);
            self.metrics.network.jitter_ms = self.average(&self.jitter_samples);
        }
        
        if let Some(rtt) = rtt_ms {
            self.add_sample(&mut self.rtt_samples, rtt, now);
            self.metrics.network.rtt_ms = self.average(&self.rtt_samples);
        }
        
        if let Some(bandwidth) = bandwidth_kbps {
            self.add_sample(&mut self.bandwidth_samples, bandwidth, now);
            self.metrics.network.bandwidth_kbps = self.average(&self.bandwidth_samples);
        }
        
        // Update consecutive losses
        self.metrics.network.consecutive_losses = self.current_loss_burst;
        
        // Update timestamp and duration
        self.update_timestamp(now);
    }
    
    /// Update audio metrics with new values
    pub fn update_audio(
        &mut self,
        level: Option<f32>,
        peak_level: Option<f32>,
        speech_active: Option<bool>,
        snr_db: Option<f32>,
        codec_bitrate_kbps: Option<f32>,
        sample_rate_hz: Option<u32>,
    ) {
        let now = Instant::now();
        
        // Update level if provided
        if let Some(l) = level {
            self.add_sample(&mut self.level_samples, l, now);
            self.metrics.audio.avg_level = self.average(&self.level_samples);
        }
        
        // Update peak level if provided and higher than current
        if let Some(peak) = peak_level {
            if peak > self.metrics.audio.peak_level {
                self.metrics.audio.peak_level = peak;
            }
        }
        
        // Update speech activity if provided
        if let Some(active) = speech_active {
            let activity_value = if active { 1.0 } else { 0.0 };
            self.add_sample(&mut self.speech_activity_samples, activity_value, now);
            self.metrics.audio.speech_activity = self.average(&self.speech_activity_samples);
        }
        
        // Update SNR if provided
        if let Some(snr) = snr_db {
            self.add_sample(&mut self.snr_samples, snr, now);
            self.metrics.audio.snr_db = self.average(&self.snr_samples);
        }
        
        // Update codec info if provided
        if let Some(bitrate) = codec_bitrate_kbps {
            self.metrics.audio.codec_bitrate_kbps = bitrate;
        }
        
        if let Some(sample_rate) = sample_rate_hz {
            self.metrics.audio.sample_rate_hz = sample_rate;
        }
        
        // Update timestamp and duration
        self.update_timestamp(now);
    }
    
    /// Set the audio clarity estimate (after processing)
    pub fn set_clarity(&mut self, clarity: f32) {
        self.metrics.audio.clarity = clarity.clamp(0.0, 1.0);
    }
    
    /// Get the current quality metrics
    pub fn metrics(&self) -> &QualityMetrics {
        &self.metrics
    }
    
    /// Reset the metrics tracker
    pub fn reset(&mut self) {
        let now = Instant::now();
        
        self.packet_loss_samples.clear();
        self.jitter_samples.clear();
        self.rtt_samples.clear();
        self.bandwidth_samples.clear();
        
        self.level_samples.clear();
        self.speech_activity_samples.clear();
        self.snr_samples.clear();
        
        self.metrics = QualityMetrics::default();
        self.start_time = now;
        self.last_update = now;
        
        self.total_packets = 0;
        self.lost_packets = 0;
        self.current_loss_burst = 0;
    }
    
    /// Add a metric sample, maintaining the maximum sample count
    fn add_sample(&mut self, samples: &mut VecDeque<MetricSample>, value: f32, timestamp: Instant) {
        samples.push_back(MetricSample { value, timestamp });
        
        // Keep sample count in check
        while samples.len() > self.max_samples {
            samples.pop_front();
        }
    }
    
    /// Calculate the average of samples
    fn average(&self, samples: &VecDeque<MetricSample>) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        let sum: f32 = samples.iter().map(|s| s.value).sum();
        sum / samples.len() as f32
    }
    
    /// Update timestamp and duration
    fn update_timestamp(&mut self, now: Instant) {
        self.metrics.timestamp = now;
        self.metrics.duration_sec = now.duration_since(self.start_time).as_secs();
        self.last_update = now;
    }
    
    /// Check if metrics should be updated based on collection interval
    pub fn should_update(&self) -> bool {
        let now = Instant::now();
        now.duration_since(self.last_update).as_millis() >= self.metrics.collection_interval_ms as u128
    }
    
    /// Set the collection interval in milliseconds
    pub fn set_collection_interval(&mut self, interval_ms: u32) {
        self.metrics.collection_interval_ms = interval_ms;
    }
} 