/// Audio capture and validation for benchmark verification
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use rustfft::{FftPlanner, num_complex::Complex32};
use rand::Rng;

#[derive(Debug, Clone)]
pub struct AudioCapture {
    pub call_id: String,
    pub client_audio: Vec<i16>,  // Audio sent by client (440Hz)
    pub server_audio: Vec<i16>,  // Audio sent by server (880Hz)
    pub start_time: Instant,
    pub end_time: Option<Instant>,
    pub packet_count: usize,
}

impl AudioCapture {
    pub fn new(call_id: String) -> Self {
        Self {
            call_id,
            client_audio: Vec::new(),
            server_audio: Vec::new(),
            start_time: Instant::now(),
            end_time: None,
            packet_count: 0,
        }
    }
    
    pub fn duration(&self) -> Duration {
        self.end_time.unwrap_or_else(Instant::now) - self.start_time
    }
}

#[derive(Debug)]
pub struct AudioValidator {
    captures: Arc<Mutex<HashMap<String, AudioCapture>>>,
    selected_calls: Arc<Mutex<HashSet<String>>>,
}

impl AudioValidator {
    pub fn new() -> Self {
        Self {
            captures: Arc::new(Mutex::new(HashMap::new())),
            selected_calls: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    
    /// Select random calls to capture audio from
    pub async fn select_random_calls(&self, call_ids: &[String], count: usize) {
        let mut rng = rand::thread_rng();
        let mut selected = self.selected_calls.lock().await;
        let mut captures = self.captures.lock().await;
        
        // Select random indices
        let mut indices = HashSet::new();
        while indices.len() < count.min(call_ids.len()) {
            indices.insert(rng.gen_range(0..call_ids.len()));
        }
        
        // Add selected calls
        for idx in indices {
            let call_id = &call_ids[idx];
            selected.insert(call_id.clone());
            captures.insert(call_id.clone(), AudioCapture::new(call_id.clone()));
            println!("Selected call #{} (ID: {}) for audio capture", idx, call_id);
        }
    }
    
    /// Check if a call is selected for capture
    pub async fn is_selected(&self, call_id: &str) -> bool {
        self.selected_calls.lock().await.contains(call_id)
    }
    
    /// Capture audio from client (called when receiving RTP at server)
    pub async fn capture_client_audio(&self, call_id: &str, samples: Vec<i16>) {
        let mut captures = self.captures.lock().await;
        if let Some(capture) = captures.get_mut(call_id) {
            capture.client_audio.extend(samples);
            capture.packet_count += 1;
        }
    }
    
    /// Capture audio from server (called when receiving RTP at client)
    pub async fn capture_server_audio(&self, call_id: &str, samples: Vec<i16>) {
        let mut captures = self.captures.lock().await;
        if let Some(capture) = captures.get_mut(call_id) {
            capture.server_audio.extend(samples);
        }
    }
    
    /// Mark call as ended
    pub async fn end_call(&self, call_id: &str) {
        let mut captures = self.captures.lock().await;
        if let Some(capture) = captures.get_mut(call_id) {
            capture.end_time = Some(Instant::now());
        }
    }
    
    /// Detect the dominant frequency in audio samples using FFT
    pub fn detect_frequency(samples: &[i16], sample_rate: u32) -> Option<(f32, f32)> {
        if samples.len() < 1024 {
            return None;
        }
        
        // Use a window of samples for FFT (power of 2)
        let fft_size = 8192.min(samples.len());
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        
        // Convert samples to complex numbers
        let mut buffer: Vec<Complex32> = samples.iter()
            .take(fft_size)
            .map(|&s| Complex32::new(s as f32 / 32768.0, 0.0))
            .collect();
        
        // Apply Hanning window to reduce spectral leakage
        for (i, sample) in buffer.iter_mut().enumerate() {
            let window = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / fft_size as f32).cos();
            *sample = Complex32::new(sample.re * window, sample.im);
        }
        
        // Perform FFT
        fft.process(&mut buffer);
        
        // Calculate magnitude spectrum
        let magnitudes: Vec<f32> = buffer.iter()
            .take(fft_size / 2) // Only need first half (Nyquist)
            .map(|c| (c.re * c.re + c.im * c.im).sqrt())
            .collect();
        
        // Find peak frequency
        let (peak_bin, &peak_magnitude) = magnitudes.iter()
            .enumerate()
            .skip(10) // Skip DC and very low frequencies
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())?;
        
        let peak_frequency = peak_bin as f32 * sample_rate as f32 / fft_size as f32;
        
        Some((peak_frequency, peak_magnitude))
    }
    
    /// Validate a specific tone is present
    pub fn validate_tone(samples: &[i16], expected_freq: f32, sample_rate: u32) -> ToneValidation {
        if let Some((detected_freq, magnitude)) = Self::detect_frequency(samples, sample_rate) {
            let frequency_error = (detected_freq - expected_freq).abs() / expected_freq;
            let is_valid = frequency_error < 0.02; // Within 2% of expected frequency
            
            ToneValidation {
                expected_frequency: expected_freq,
                detected_frequency: Some(detected_freq),
                frequency_error_percent: frequency_error * 100.0,
                magnitude,
                is_valid,
            }
        } else {
            ToneValidation {
                expected_frequency: expected_freq,
                detected_frequency: None,
                frequency_error_percent: 100.0,
                magnitude: 0.0,
                is_valid: false,
            }
        }
    }
    
    /// Calculate Signal-to-Noise Ratio
    pub fn calculate_snr(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        // Simple RMS calculation
        let sum_squares: f64 = samples.iter()
            .map(|&s| (s as f64).powi(2))
            .sum();
        
        let rms = (sum_squares / samples.len() as f64).sqrt();
        
        // Estimate noise floor (simplified - use quietest 10% of samples)
        let mut sorted_samples: Vec<i16> = samples.iter().map(|&s| s.abs()).collect();
        sorted_samples.sort();
        let noise_floor_idx = sorted_samples.len() / 10;
        let noise_floor = sorted_samples[noise_floor_idx] as f64;
        
        if noise_floor > 0.0 {
            20.0 * (rms / noise_floor).log10() as f32
        } else {
            60.0 // Assume good SNR if no noise detected
        }
    }
    
    /// Validate all captured audio
    pub async fn validate_all(&self) -> Vec<ValidationResult> {
        let captures = self.captures.lock().await;
        let mut results = Vec::new();
        
        for (call_id, capture) in captures.iter() {
            let client_validation = Self::validate_tone(&capture.client_audio, 440.0, 8000);
            let server_validation = Self::validate_tone(&capture.server_audio, 880.0, 8000);
            
            let result = ValidationResult {
                call_id: call_id.clone(),
                duration: capture.duration().as_secs_f32(),
                client_tone: client_validation,
                server_tone: server_validation,
                snr: Self::calculate_snr(&capture.client_audio),
                packet_count: capture.packet_count,
            };
            
            results.push(result);
        }
        
        results.sort_by_key(|r| r.call_id.clone());
        results
    }
    
    /// Print validation results
    pub fn print_validation_results(results: &[ValidationResult]) {
        println!("\n╠════════════════════════════════════════════════════════════════╣");
        println!("║                    AUDIO VALIDATION                            ║");
        println!("╠════════════════════════════════════════════════════════════════╣");
        
        for result in results {
            let duration_check = if (result.duration - 10.0).abs() < 0.5 { "✅" } else { "❌" };
            let client_check = if result.client_tone.is_valid { "✅" } else { "❌" };
            let server_check = if result.server_tone.is_valid { "✅" } else { "❌" };
            
            let call_id_display = if result.call_id.len() >= 8 {
                result.call_id[..8].to_string()
            } else {
                format!("{:8}", result.call_id)
            };
            println!("║ Call {}: {} Duration: {:.2}s │ 440Hz: {} │ 880Hz: {} │ SNR: {:.1}dB ║",
                call_id_display,
                duration_check,
                result.duration,
                client_check,
                server_check,
                result.snr,
            );
        }
        
        // Calculate aggregate stats
        let avg_snr = results.iter().map(|r| r.snr).sum::<f32>() / results.len() as f32;
        let all_valid = results.iter().all(|r| r.client_tone.is_valid && r.server_tone.is_valid);
        let avg_packets = results.iter().map(|r| r.packet_count).sum::<usize>() / results.len();
        
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ Audio Quality Metrics:                                         ║");
        println!("║   Average SNR: {:.1} dB                                        ║", avg_snr);
        println!("║   All Tones Detected: {}                                     ║", 
            if all_valid { "✅ Yes" } else { "❌ No " });
        println!("║   Avg Packets/Call: {}                                       ║", avg_packets);
        println!("╚════════════════════════════════════════════════════════════════╝");
    }
}

#[derive(Debug)]
pub struct ToneValidation {
    pub expected_frequency: f32,
    pub detected_frequency: Option<f32>,
    pub frequency_error_percent: f32,
    pub magnitude: f32,
    pub is_valid: bool,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub call_id: String,
    pub duration: f32,
    pub client_tone: ToneValidation,
    pub server_tone: ToneValidation,
    pub snr: f32,
    pub packet_count: usize,
}