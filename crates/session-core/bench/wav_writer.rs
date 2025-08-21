/// WAV file writer for received audio capture
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use hound::{WavWriter, WavSpec, SampleFormat};
use std::collections::HashMap;

/// Captures received audio streams for a single call
#[derive(Debug, Clone)]
pub struct WavCapture {
    pub call_id: String,
    /// Audio received by the client (880Hz from server)
    pub client_received: Vec<i16>,
    /// Audio received by the server (440Hz from client)
    pub server_received: Vec<i16>,
    /// Track if we've started capturing
    pub capture_started: bool,
}

impl WavCapture {
    pub fn new(call_id: String) -> Self {
        Self {
            call_id,
            client_received: Vec::new(),
            server_received: Vec::new(),
            capture_started: false,
        }
    }

    /// Add samples received by the client
    pub fn add_client_received(&mut self, samples: &[i16]) {
        self.client_received.extend_from_slice(samples);
        self.capture_started = true;
    }

    /// Add samples received by the server
    pub fn add_server_received(&mut self, samples: &[i16]) {
        self.server_received.extend_from_slice(samples);
        self.capture_started = true;
    }

    /// Save the captured audio as a stereo WAV file
    /// Left channel: Client's received audio (880Hz from server)
    /// Right channel: Server's received audio (440Hz from client)
    pub fn save_to_wav(&self, output_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
        // Create output directory if it doesn't exist
        std::fs::create_dir_all(output_dir)?;

        // Generate filename
        let filename = format!("{}_received.wav", self.call_id);
        let file_path = output_dir.join(&filename);

        // Create WAV spec for stereo output
        let spec = WavSpec {
            channels: 2,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };

        // Create WAV writer
        let mut writer = WavWriter::create(&file_path, spec)?;

        // Write stereo samples
        let max_len = self.client_received.len().max(self.server_received.len());

        for i in 0..max_len {
            // Left channel: client's received audio (880Hz from server)
            let client_sample = self.client_received.get(i).copied().unwrap_or(0);

            // Right channel: server's received audio (440Hz from client)
            let server_sample = self.server_received.get(i).copied().unwrap_or(0);

            // Write stereo sample
            writer.write_sample(client_sample)?;
            writer.write_sample(server_sample)?;
        }

        writer.finalize()?;
        Ok(file_path)
    }

    /// Save separate mono WAV files for detailed analysis
    pub fn save_separate_wavs(&self, output_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
        std::fs::create_dir_all(output_dir)?;
        let mut paths = Vec::new();

        // Helper function to save mono WAV
        let save_mono = |samples: &[i16], name: &str| -> Result<PathBuf, Box<dyn std::error::Error>> {
            let path = output_dir.join(name);
            let spec = WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::create(&path, spec)?;
            for &sample in samples {
                writer.write_sample(sample)?;
            }
            writer.finalize()?;
            Ok(path)
        };

        // Save received audio streams separately for detailed analysis
        if !self.client_received.is_empty() {
            paths.push(save_mono(&self.client_received, &format!("{}_client_received_880Hz.wav", self.call_id))?);
        }
        if !self.server_received.is_empty() {
            paths.push(save_mono(&self.server_received, &format!("{}_server_received_440Hz.wav", self.call_id))?);
        }

        Ok(paths)
    }
}

/// Manages WAV captures for multiple calls
#[derive(Debug)]
pub struct WavCaptureManager {
    captures: Arc<Mutex<HashMap<String, WavCapture>>>,
    output_dir: PathBuf,
}

impl WavCaptureManager {
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            captures: Arc::new(Mutex::new(HashMap::new())),
            output_dir,
        }
    }

    /// Initialize capture for a call
    pub async fn init_capture(&self, call_id: String) {
        let mut captures = self.captures.lock().await;
        captures.insert(call_id.clone(), WavCapture::new(call_id));
    }

    /// Add client received audio
    pub async fn add_client_received(&self, call_id: &str, samples: &[i16]) {
        let mut captures = self.captures.lock().await;
        if let Some(capture) = captures.get_mut(call_id) {
            capture.add_client_received(samples);
        }
    }

    /// Add server received audio
    pub async fn add_server_received(&self, call_id: &str, samples: &[i16]) {
        let mut captures = self.captures.lock().await;
        if let Some(capture) = captures.get_mut(call_id) {
            capture.add_server_received(samples);
        }
    }

    /// Save all captures to WAV files
    pub async fn save_all(&self) -> Result<Vec<(String, PathBuf)>, Box<dyn std::error::Error>> {
        let captures = self.captures.lock().await;
        let mut results = Vec::new();

        for (call_id, capture) in captures.iter() {
            if capture.capture_started {
                let path = capture.save_to_wav(&self.output_dir)?;
                results.push((call_id.clone(), path));
                
                // Also save separate files for detailed analysis
                let _separate_paths = capture.save_separate_wavs(&self.output_dir)?;
            }
        }

        Ok(results)
    }

    /// Clean up old WAV files in the output directory
    pub fn cleanup_old_files(&self) -> std::io::Result<()> {
        if self.output_dir.exists() {
            for entry in std::fs::read_dir(&self.output_dir)? {
                let entry = entry?;
                if let Some(ext) = entry.path().extension() {
                    if ext == "wav" {
                        std::fs::remove_file(entry.path())?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Load and analyze a WAV file
pub fn analyze_wav_file(path: &Path) -> Result<WavAnalysis, Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    
    let samples: Vec<i16> = reader.samples::<i16>()
        .collect::<Result<Vec<_>, _>>()?;
    
    let mut left_channel = Vec::new();
    let mut right_channel = Vec::new();
    
    if spec.channels == 2 {
        // Stereo: separate channels
        for (i, &sample) in samples.iter().enumerate() {
            if i % 2 == 0 {
                left_channel.push(sample);
            } else {
                right_channel.push(sample);
            }
        }
    } else {
        // Mono: duplicate to both channels for consistency
        left_channel = samples.clone();
        right_channel = samples;
    }
    
    Ok(WavAnalysis {
        path: path.to_path_buf(),
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        duration_secs: left_channel.len() as f32 / spec.sample_rate as f32,
        left_channel,  // Client received (880Hz)
        right_channel, // Server received (440Hz)
    })
}

#[derive(Debug)]
pub struct WavAnalysis {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_secs: f32,
    pub left_channel: Vec<i16>,  // Client received audio
    pub right_channel: Vec<i16>, // Server received audio
}