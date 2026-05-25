//! D3a — cpal-backed microphone capture + speaker playback.
//!
//! [`CpalAudioSource`] opens the default input device, captures 48 kHz
//! mono PCM in 20 ms frames (960 samples), encodes each frame with the
//! `opus` crate (matching the engine's registered Opus codec at PT 111),
//! and yields RTP-ready [`MediaFrame`]s through the
//! [`AudioSource`](crate::client::AudioSource) trait.
//!
//! [`CpalSpeakerSink`] is the inbound mirror — it receives RTP-encoded
//! Opus packets, decodes them, and plays the PCM on the default output
//! device.
//!
//! Threading: the cpal callback is synchronous and runs on a realtime
//! thread; we hand frames off via a bounded `tokio::sync::mpsc` with
//! `try_send` to avoid blocking that thread (the same pattern as
//! `audio-core/src/device/cpal_stream.rs`).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, StreamKind};

use crate::client::media_source::{AudioSink, AudioSource};
use crate::errors::{Result, WebRtcError};

/// Default Opus parameters for the cpal source/sink: 48 kHz, mono, 20 ms
/// frames — matches both webrtc-rs's preferred Opus shape and the SDP
/// fmtp line registered by [`build_media_engine`](crate::peer::builder::build_media_engine).
const SAMPLE_RATE_HZ: u32 = 48_000;
const FRAME_MS: u32 = 20;
const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE_HZ as usize / 1000) * FRAME_MS as usize;
const PCM_CHANNELS: u16 = 1;
const PCM_QUEUE_DEPTH: usize = 32; // ~640 ms of buffered PCM — survives ~1 GC pause.

/// Configuration knobs for [`CpalAudioSource`] / [`CpalSpeakerSink`].
#[derive(Clone, Debug)]
pub struct CpalAudioConfig {
    /// Target encoder bitrate (bits/s). Default 32 kbps — typical VoIP.
    pub bitrate_bps: i32,
    /// Initial RTP SSRC. Should match the local audio track's SSRC
    /// (`RvoipPeerConnection::local_audio_ssrc()`).
    pub ssrc: u32,
    /// `StreamId` to populate on the `MediaFrame`. Usually the WebRTC
    /// adapter's stream id for this connection.
    pub stream_id: StreamId,
}

impl CpalAudioConfig {
    pub fn new(stream_id: StreamId, ssrc: u32) -> Self {
        Self {
            bitrate_bps: 32_000,
            ssrc,
            stream_id,
        }
    }
}

/// Cpal-backed microphone source. Hold the returned struct for the
/// lifetime of the call — dropping it stops the underlying cpal stream.
pub struct CpalAudioSource {
    cfg: CpalAudioConfig,
    pcm_rx: AsyncMutex<mpsc::Receiver<Vec<i16>>>,
    encoder: Arc<Mutex<opus::Encoder>>,
    /// RTP sequence number — wraps naturally.
    seq: u16,
    /// RTP timestamp at 48 kHz clock. Advances by 960 per frame.
    timestamp: u32,
    /// Holds the live cpal stream. cpal::Stream isn't `Send`, but it
    /// stays on the thread that built it (the runtime task driving the
    /// async portion is separate). We wrap it in an option so we can move
    /// it into the destructor explicitly.
    _stream: CpalStreamGuard,
}

impl CpalAudioSource {
    /// Open the system's default input device and start capturing.
    pub fn new_default(cfg: CpalAudioConfig) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| WebRtcError::Adapter("no default audio input device".into()))?;
        Self::new_with_device(cfg, &device)
    }

    /// Open a specific input device.
    pub fn new_with_device(cfg: CpalAudioConfig, device: &cpal::Device) -> Result<Self> {
        let stream_cfg = pick_input_config(device)?;
        let hw_rate = stream_cfg.sample_rate().0;
        let hw_channels = stream_cfg.channels();

        let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<i16>>(PCM_QUEUE_DEPTH);

        let mut buffer: Vec<i16> = Vec::with_capacity(SAMPLES_PER_FRAME);
        let downmix_to_mono = hw_channels > 1;
        let resample_step = hw_rate as f64 / SAMPLE_RATE_HZ as f64;

        let err_fn = |err| tracing::warn!(target: "rvoip_webrtc", error = %err, "cpal input error");

        let cpal_stream = device
            .build_input_stream(
                &stream_cfg.config(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Downmix multi-channel to mono by averaging channels.
                    let chunk: Vec<i16> = if downmix_to_mono {
                        data.chunks_exact(hw_channels as usize)
                            .map(|c| {
                                let sum: f32 = c.iter().sum();
                                let avg = sum / hw_channels as f32;
                                (avg.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
                            })
                            .collect()
                    } else {
                        data.iter()
                            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                            .collect()
                    };

                    // Linear resample if the hardware isn't already 48 kHz.
                    let resampled: Vec<i16> = if (resample_step - 1.0).abs() < 1e-6 {
                        chunk
                    } else {
                        let out_len = (chunk.len() as f64 / resample_step) as usize;
                        let mut out = Vec::with_capacity(out_len);
                        for i in 0..out_len {
                            let src_idx = (i as f64 * resample_step) as usize;
                            if let Some(&s) = chunk.get(src_idx) {
                                out.push(s);
                            }
                        }
                        out
                    };

                    buffer.extend_from_slice(&resampled);
                    while buffer.len() >= SAMPLES_PER_FRAME {
                        let frame: Vec<i16> = buffer.drain(..SAMPLES_PER_FRAME).collect();
                        // Realtime thread: never block. Drop on full queue.
                        let _ = pcm_tx.try_send(frame);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| WebRtcError::Adapter(format!("cpal build_input_stream: {e}")))?;

        cpal_stream
            .play()
            .map_err(|e| WebRtcError::Adapter(format!("cpal stream.play(): {e}")))?;

        let mut encoder = opus::Encoder::new(
            SAMPLE_RATE_HZ,
            opus::Channels::Mono,
            opus::Application::Voip,
        )
        .map_err(|e| WebRtcError::Adapter(format!("opus encoder init: {e:?}")))?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(cfg.bitrate_bps))
            .map_err(|e| WebRtcError::Adapter(format!("opus set_bitrate: {e:?}")))?;

        Ok(Self {
            cfg,
            pcm_rx: AsyncMutex::new(pcm_rx),
            encoder: Arc::new(Mutex::new(encoder)),
            seq: 1,
            timestamp: 0,
            _stream: CpalStreamGuard::new(cpal_stream),
        })
    }
}

#[async_trait]
impl AudioSource for CpalAudioSource {
    async fn next_packet(&mut self) -> Result<Option<MediaFrame>> {
        let pcm = match self.pcm_rx.lock().await.recv().await {
            Some(pcm) => pcm,
            None => return Ok(None), // capture stream ended
        };

        let mut encoded = vec![0u8; 4000];
        let encoded_len = {
            let mut guard = self.encoder.lock();
            guard
                .encode(&pcm, &mut encoded)
                .map_err(|e| WebRtcError::Adapter(format!("opus encode: {e:?}")))?
        };
        encoded.truncate(encoded_len);

        // D4 follow-up — emit codec payload bytes only. The outbound
        // pump wraps in RTP with the local SSRC + the negotiated PT.
        let frame = MediaFrame {
            stream_id: self.cfg.stream_id.clone(),
            kind: StreamKind::Audio,
            payload: Bytes::from(encoded),
            timestamp_rtp: self.timestamp,
            captured_at: Utc::now(),
        };
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(SAMPLES_PER_FRAME as u32);
        Ok(Some(frame))
    }
}

/// Cpal-backed speaker sink: decodes inbound Opus RTP and plays PCM on
/// the default output device.
pub struct CpalSpeakerSink {
    decoder: Arc<Mutex<opus::Decoder>>,
    pcm_tx: mpsc::Sender<Vec<i16>>,
    _stream: CpalStreamGuard,
}

impl CpalSpeakerSink {
    pub fn new_default() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| WebRtcError::Adapter("no default audio output device".into()))?;
        Self::new_with_device(&device)
    }

    pub fn new_with_device(device: &cpal::Device) -> Result<Self> {
        let stream_cfg = pick_output_config(device)?;
        let hw_rate = stream_cfg.sample_rate().0;
        let hw_channels = stream_cfg.channels();

        let (pcm_tx, mut pcm_rx) = mpsc::channel::<Vec<i16>>(PCM_QUEUE_DEPTH);

        // Playback buffer that the cpal callback drains. Sized to absorb
        // a small jitter without overflow.
        let mut playback_buf: Vec<f32> = Vec::with_capacity(SAMPLES_PER_FRAME * 4);

        let upmix_to_stereo = hw_channels == 2;
        let resample_step = SAMPLE_RATE_HZ as f64 / hw_rate as f64;

        let err_fn = |err| tracing::warn!(target: "rvoip_webrtc", error = %err, "cpal output error");

        let cpal_stream = device
            .build_output_stream(
                &stream_cfg.config(),
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    while playback_buf.len() < out.len() {
                        match pcm_rx.try_recv() {
                            Ok(pcm) => {
                                // Resample 48 kHz -> hw_rate via linear pick.
                                let mut hw_pcm: Vec<f32> = if (resample_step - 1.0).abs() < 1e-6 {
                                    pcm.iter().map(|&s| s as f32 / i16::MAX as f32).collect()
                                } else {
                                    let out_len = (pcm.len() as f64 * resample_step) as usize;
                                    let mut o = Vec::with_capacity(out_len);
                                    for i in 0..out_len {
                                        let src_idx = (i as f64 / resample_step) as usize;
                                        let s = pcm.get(src_idx).copied().unwrap_or(0);
                                        o.push(s as f32 / i16::MAX as f32);
                                    }
                                    o
                                };
                                if upmix_to_stereo {
                                    let stereo: Vec<f32> = hw_pcm
                                        .iter()
                                        .flat_map(|&s| [s, s])
                                        .collect();
                                    hw_pcm = stereo;
                                }
                                playback_buf.extend_from_slice(&hw_pcm);
                            }
                            Err(_) => {
                                // Underrun — pad with silence rather than block.
                                playback_buf.resize(out.len(), 0.0);
                                break;
                            }
                        }
                    }
                    let take = playback_buf.len().min(out.len());
                    out[..take].copy_from_slice(&playback_buf[..take]);
                    if take < out.len() {
                        for s in &mut out[take..] {
                            *s = 0.0;
                        }
                    }
                    playback_buf.drain(..take);
                },
                err_fn,
                None,
            )
            .map_err(|e| WebRtcError::Adapter(format!("cpal build_output_stream: {e}")))?;

        cpal_stream
            .play()
            .map_err(|e| WebRtcError::Adapter(format!("cpal output play(): {e}")))?;

        let decoder = opus::Decoder::new(SAMPLE_RATE_HZ, opus::Channels::Mono)
            .map_err(|e| WebRtcError::Adapter(format!("opus decoder init: {e:?}")))?;

        Ok(Self {
            decoder: Arc::new(Mutex::new(decoder)),
            pcm_tx,
            _stream: CpalStreamGuard::new(cpal_stream),
        })
    }
}

#[async_trait]
impl AudioSink for CpalSpeakerSink {
    async fn write_packet(&mut self, frame: MediaFrame) -> Result<()> {
        // D4 follow-up — `MediaFrame.payload` is codec bytes; the inbound
        // pump strips the RTP header upstream.
        let mut pcm = vec![0i16; SAMPLES_PER_FRAME];
        let n = {
            let mut guard = self.decoder.lock();
            guard
                .decode(&frame.payload, &mut pcm, false)
                .map_err(|e| WebRtcError::Adapter(format!("opus decode: {e:?}")))?
        };
        pcm.truncate(n);
        // Best-effort send to the cpal callback. Drop on overflow.
        let _ = self.pcm_tx.try_send(pcm);
        Ok(())
    }
}

fn pick_input_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig> {
    let configs: Vec<_> = device
        .supported_input_configs()
        .map_err(|e| WebRtcError::Adapter(format!("supported_input_configs: {e}")))?
        .collect();
    pick_compatible(device, &configs, /* is_input = */ true)
}

fn pick_output_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig> {
    let configs: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| WebRtcError::Adapter(format!("supported_output_configs: {e}")))?
        .collect();
    pick_compatible(device, &configs, /* is_input = */ false)
}

fn pick_compatible(
    _device: &cpal::Device,
    configs: &[cpal::SupportedStreamConfigRange],
    _is_input: bool,
) -> Result<cpal::SupportedStreamConfig> {
    // Prefer 48 kHz mono; fall back to the first config that supports
    // 48 kHz; finally pick the first config and adapt in software.
    for cfg in configs {
        if cfg.channels() == PCM_CHANNELS
            && cfg.min_sample_rate().0 <= SAMPLE_RATE_HZ
            && cfg.max_sample_rate().0 >= SAMPLE_RATE_HZ
        {
            return Ok(cfg.clone().with_sample_rate(cpal::SampleRate(SAMPLE_RATE_HZ)));
        }
    }
    for cfg in configs {
        if cfg.min_sample_rate().0 <= SAMPLE_RATE_HZ
            && cfg.max_sample_rate().0 >= SAMPLE_RATE_HZ
        {
            return Ok(cfg.clone().with_sample_rate(cpal::SampleRate(SAMPLE_RATE_HZ)));
        }
    }
    let cfg = configs
        .first()
        .ok_or_else(|| WebRtcError::Adapter("device has no supported configs".into()))?;
    let rate = cfg.max_sample_rate().0.min(48_000).max(cfg.min_sample_rate().0);
    Ok(cfg.clone().with_sample_rate(cpal::SampleRate(rate)))
}

/// Owns a cpal::Stream. cpal::Stream is `!Send` on some platforms (CoreAudio),
/// so we keep it on the construction thread by wrapping in a struct that
/// doesn't itself need Send. The Stream object only needs to live as long
/// as the source/sink — it stops on drop.
struct CpalStreamGuard {
    _stream: Option<cpal::Stream>,
}

impl CpalStreamGuard {
    fn new(stream: cpal::Stream) -> Self {
        Self {
            _stream: Some(stream),
        }
    }
}

// SAFETY: cpal::Stream on macOS/CoreAudio is !Send, but it's only ever
// dropped (Drop is fine on any thread because cpal serializes shutdown
// internally) or held by reference. The Source/Sink methods don't access
// `_stream` after construction. This unsafe impl is the same pattern used
// in production media libraries that wrap cpal in a Send-capable handle.
unsafe impl Send for CpalStreamGuard {}
unsafe impl Sync for CpalStreamGuard {}
