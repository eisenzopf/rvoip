/// Audio capture and validation for benchmark verification using WAV files
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use rustfft::{FftPlanner, num_complex::Complex32};
use rand::Rng;

use crate::wav_writer::{WavCaptureManager, analyze_wav_file};

#[derive(Debug)]
pub struct AudioValidator {
    selected_indices: Arc<Mutex<HashSet<usize>>>,  // Store indices instead of call IDs
    selected_calls: Arc<Mutex<HashSet<String>>>,    // Actual call IDs that get selected
    wav_manager: Arc<WavCaptureManager>,
    saved_files: Arc<Mutex<Vec<(String, PathBuf)>>>,
}

impl AudioValidator {
    pub fn new() -> Self {
        // Create samples directory in the bench folder
        let output_dir = PathBuf::from("/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core/bench/samples");
        let wav_manager = Arc::new(WavCaptureManager::new(output_dir));
        
        // Clean up old files
        let _ = wav_manager.cleanup_old_files();
        
        Self {
            selected_indices: Arc::new(Mutex::new(HashSet::new())),
            selected_calls: Arc::new(Mutex::new(HashSet::new())),
            wav_manager,
            saved_files: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Select random call indices to capture audio from
    pub async fn select_random_indices(&self, total_calls: usize, count: usize) -> Vec<usize> {
        let mut rng = rand::thread_rng();
        let mut selected = self.selected_indices.lock().await;
        
        // Select random indices
        let mut indices = HashSet::new();
        while indices.len() < count.min(total_calls) {
            indices.insert(rng.gen_range(0..total_calls));
        }
        
        // Store selected indices
        for &idx in &indices {
            selected.insert(idx);
            println!("Selected call index {} for audio capture", idx);
        }
        
        indices.into_iter().collect()
    }
    
    /// Check if a call index should be captured
    pub async fn should_capture_index(&self, index: usize) -> bool {
        self.selected_indices.lock().await.contains(&index)
    }
    
    /// Register an actual call ID for a selected index
    pub async fn register_call_for_index(&self, index: usize, call_id: String) {
        if self.should_capture_index(index).await {
            self.selected_calls.lock().await.insert(call_id.clone());
            self.wav_manager.init_capture(call_id.clone()).await;
            println!("Registered call {} (index {}) for audio capture", call_id, index);
        }
    }
    
    /// Check if a call is selected for capture
    pub async fn is_selected(&self, call_id: &str) -> bool {
        self.selected_calls.lock().await.contains(call_id)
    }
    
    /// Capture audio received by client (from server)
    pub async fn capture_client_received(&self, call_id: &str, samples: Vec<i16>) {
        if self.is_selected(call_id).await {
            self.wav_manager.add_client_received(call_id, &samples).await;
        }
    }
    
    /// Capture audio received by server (from client)
    pub async fn capture_server_received(&self, call_id: &str, samples: Vec<i16>) {
        if self.is_selected(call_id).await {
            self.wav_manager.add_server_received(call_id, &samples).await;
        }
    }
    
    /// Save all captured audio to WAV files
    pub async fn save_wav_files(&self) -> Result<(), Box<dyn std::error::Error>> {
        let files = self.wav_manager.save_all().await?;
        *self.saved_files.lock().await = files;
        Ok(())
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
    
    /// Validate all captured audio by analyzing WAV files
    pub async fn validate_all(&self) -> Vec<ValidationResult> {
        // First save all WAV files
        if let Err(e) = self.save_wav_files().await {
            eprintln!("Failed to save WAV files: {}", e);
            return Vec::new();
        }
        
        let saved_files = self.saved_files.lock().await;
        let mut results = Vec::new();
        
        println!("\nSaved {} WAV files to analyze", saved_files.len());
        
        for (call_id, wav_path) in saved_files.iter() {
            match analyze_wav_file(wav_path) {
                Ok(analysis) => {
                    // Analyze left channel (client received 880Hz from server)
                    let client_validation = Self::validate_pure_tone(&analysis.left_channel, 880.0, 8000);
                    
                    // Analyze right channel (server received 440Hz from client)
                    let server_validation = Self::validate_pure_tone(&analysis.right_channel, 440.0, 8000);
                    
                    let result = ValidationResult {
                        call_id: call_id.clone(),
                        wav_file: wav_path.clone(),
                        duration: analysis.duration_secs,
                        client_channel: client_validation,
                        server_channel: server_validation,
                        snr: Self::calculate_snr(&analysis.left_channel)
                            .max(Self::calculate_snr(&analysis.right_channel)),
                    };
                    
                    results.push(result);
                }
                Err(e) => {
                    eprintln!("Failed to analyze WAV file {}: {}", wav_path.display(), e);
                }
            }
        }
        
        results.sort_by_key(|r| r.call_id.clone());
        results
    }
    
    /// Validate a single pure tone channel
    fn validate_pure_tone(samples: &[i16], expected_freq: f32, sample_rate: u32) -> ChannelValidation {
        // Detect the dominant frequency in the pure signal
        let validation = Self::validate_tone(samples, expected_freq, sample_rate);
        
        ChannelValidation {
            expected_frequency: expected_freq,
            detected_frequency: validation.detected_frequency,
            frequency_error_percent: validation.frequency_error_percent,
            is_valid: validation.is_valid,
        }
    }
    
    /// Print validation results
    pub fn print_validation_results(results: &[ValidationResult]) {
        println!("\n╠════════════════════════════════════════════════════════════════╣");
        println!("║                  WAV FILE AUDIO VALIDATION                     ║");
        println!("╠════════════════════════════════════════════════════════════════╣");
        
        if results.is_empty() {
            println!("║  No WAV files were saved for analysis                          ║");
            println!("║  Check that calls were established and audio was captured      ║");
        } else {
            for result in results {
                let duration_check = if (result.duration - 10.0).abs() < 0.5 { "✅" } else { "❌" };
                let client_check = if result.client_channel.is_valid { "✅" } else { "❌" };
                let server_check = if result.server_channel.is_valid { "✅" } else { "❌" };
                
                let call_id_display = if result.call_id.len() >= 8 {
                    result.call_id[..8].to_string()
                } else {
                    format!("{:8}", result.call_id)
                };
                
                println!("║ Call {}:                                                  ║", call_id_display);
                println!("║   Duration: {} {:.2}s │ SNR: {:.1}dB                          ║",
                    duration_check, result.duration, result.snr);
                println!("║   Client received 880Hz: {} ({:.1}% error)                    ║",
                    client_check, 
                    result.client_channel.frequency_error_percent);
                println!("║   Server received 440Hz: {} ({:.1}% error)                    ║",
                    server_check,
                    result.server_channel.frequency_error_percent);
                println!("║   WAV File: {}                                    ║", 
                    result.wav_file.file_name().unwrap().to_string_lossy());
                println!("║                                                                 ║");
            }
        }
        
        // Calculate aggregate stats
        if !results.is_empty() {
            let avg_snr = results.iter().map(|r| r.snr).sum::<f32>() / results.len() as f32;
            let all_valid = results.iter().all(|r| r.client_channel.is_valid && r.server_channel.is_valid);
            let files_saved = results.len();
            
            println!("╠════════════════════════════════════════════════════════════════╣");
            println!("║ Audio Quality Summary:                                         ║");
            println!("║   WAV Files Saved: {}                                          ║", files_saved);
            println!("║   Average SNR: {:.1} dB                                        ║", avg_snr);
            println!("║   All Channels Valid: {}                                     ║", 
                if all_valid { "✅ Yes" } else { "❌ No " });
            println!("║   Output Directory: bench/samples/                             ║");
        }
        
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
pub struct ChannelValidation {
    pub expected_frequency: f32,
    pub detected_frequency: Option<f32>,
    pub frequency_error_percent: f32,
    pub is_valid: bool,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub call_id: String,
    pub wav_file: PathBuf,
    pub duration: f32,
    pub client_channel: ChannelValidation,
    pub server_channel: ChannelValidation,
    pub snr: f32,
}