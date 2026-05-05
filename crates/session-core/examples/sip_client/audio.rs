use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc as std_mpsc, Arc, Mutex,
};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use rvoip_session_core::{EndpointAudioFrame, EndpointAudioSender, EndpointCall};

use crate::ui::UiEvent;

pub(crate) const SAMPLE_RATE: u32 = 8_000;
pub(crate) const FRAME_MS: u32 = 20;
pub(crate) const FRAME_SAMPLES: usize = (SAMPLE_RATE as usize * FRAME_MS as usize) / 1_000;

pub(crate) async fn start_cpal_audio(
    bridge: &AudioBridge,
    call: EndpointCall,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) -> Option<RunningAudio> {
    match bridge.start(call).await {
        Ok(audio) => Some(audio),
        Err(err) => {
            let _ = event_tx.send(UiEvent::Error(format!("audio failed: {err}")));
            None
        }
    }
}

#[derive(Clone)]
pub(crate) struct AudioBridge {
    input_device: Option<String>,
    output_device: Option<String>,
    muted: Arc<AtomicBool>,
    event_tx: mpsc::UnboundedSender<UiEvent>,
}

impl AudioBridge {
    pub(crate) fn new(
        input_device: Option<String>,
        output_device: Option<String>,
        event_tx: mpsc::UnboundedSender<UiEvent>,
    ) -> Self {
        Self {
            input_device,
            output_device,
            muted: Arc::new(AtomicBool::new(false)),
            event_tx,
        }
    }

    pub(crate) fn toggle_muted(&self) -> bool {
        let next = !self.muted.load(Ordering::SeqCst);
        self.muted.store(next, Ordering::SeqCst);
        next
    }

    pub(crate) async fn start(&self, call: EndpointCall) -> anyhow::Result<RunningAudio> {
        let audio = call.audio().await?;
        let (sender, mut receiver) = audio.split();
        let host = cpal::default_host();
        let input = choose_device(&host, true, self.input_device.as_deref())?;
        let output = choose_device(&host, false, self.output_device.as_deref())?;
        let input_name = input.name().unwrap_or_else(|_| "input".into());
        let output_name = output.name().unwrap_or_else(|_| "output".into());

        let input_config = input.default_input_config()?;
        let output_config = output.default_output_config()?;
        let input_sample_rate = input_config.sample_rate().0;
        let output_sample_rate = output_config.sample_rate().0;
        let input_channels = input_config.channels() as usize;
        let output_channels = output_config.channels() as usize;

        let (mic_tx, mut mic_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let playback_buffer = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(
            output_sample_rate as usize,
        )));
        let muted = self.muted.clone();
        let event_tx = self.event_tx.clone();

        let input_stream =
            build_input_stream(&input, &input_config.into(), input_channels, mic_tx, muted)?;
        let output_stream = build_output_stream(
            &output,
            &output_config.into(),
            output_channels,
            playback_buffer.clone(),
        )?;

        input_stream.play()?;
        output_stream.play()?;

        let input_task = tokio::spawn(async move {
            send_microphone_frames(&mut mic_rx, input_sample_rate, sender).await;
        });

        let output_task = tokio::spawn(async move {
            while let Some(frame) = receiver.recv().await {
                let mono = frame
                    .samples
                    .iter()
                    .map(|sample| *sample as f32 / i16::MAX as f32)
                    .collect::<Vec<_>>();
                let resampled = resample_linear(&mono, frame.sample_rate, output_sample_rate);
                if let Ok(mut buffer) = playback_buffer.lock() {
                    buffer.extend(resampled);
                    let max_len = output_sample_rate as usize * 2;
                    while buffer.len() > max_len {
                        buffer.pop_front();
                    }
                } else {
                    let _ = event_tx.send(UiEvent::Error("playback buffer poisoned".into()));
                    break;
                }
            }
        });

        let _ = self.event_tx.send(UiEvent::AudioStarted(format!(
            "{input_name} -> {output_name}"
        )));

        Ok(RunningAudio {
            input_stream,
            output_stream,
            input_task,
            output_task,
        })
    }
}

async fn send_microphone_frames(
    mic_rx: &mut mpsc::UnboundedReceiver<Vec<f32>>,
    input_sample_rate: u32,
    sender: EndpointAudioSender,
) {
    let mut mono_buffer = Vec::<f32>::new();
    let mut timestamp = 0u32;
    while let Some(samples) = mic_rx.recv().await {
        let resampled = resample_linear(&samples, input_sample_rate, SAMPLE_RATE);
        mono_buffer.extend(resampled);
        while mono_buffer.len() >= FRAME_SAMPLES {
            let chunk = mono_buffer.drain(..FRAME_SAMPLES).collect::<Vec<_>>();
            let pcm = chunk.into_iter().map(float_to_i16).collect::<Vec<_>>();
            let frame = EndpointAudioFrame::pcmu_sized_mono_8khz(pcm, timestamp);
            timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
            if sender.send(frame).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(FRAME_MS as u64)).await;
        }
    }
}

pub(crate) struct RunningAudio {
    input_stream: cpal::Stream,
    output_stream: cpal::Stream,
    input_task: tokio::task::JoinHandle<()>,
    output_task: tokio::task::JoinHandle<()>,
}

impl Drop for RunningAudio {
    fn drop(&mut self) {
        let _ = &self.input_stream;
        let _ = &self.output_stream;
        self.input_task.abort();
        self.output_task.abort();
    }
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    tx: mpsc::UnboundedSender<Vec<f32>>,
    muted: Arc<AtomicBool>,
) -> anyhow::Result<cpal::Stream> {
    let err_fn = |err| eprintln!("input stream error: {err}");
    let sample_format = device.default_input_config()?.sample_format();
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| {
                send_input_samples(data, channels, &tx, &muted);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                let converted = data
                    .iter()
                    .map(|sample| *sample as f32 / i16::MAX as f32)
                    .collect::<Vec<_>>();
                send_input_samples(&converted, channels, &tx, &muted);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                let converted = data
                    .iter()
                    .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect::<Vec<_>>();
                send_input_samples(&converted, channels, &tx, &muted);
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported input sample format {other:?}"),
    };
    Ok(stream)
}

fn send_input_samples(
    data: &[f32],
    channels: usize,
    tx: &mpsc::UnboundedSender<Vec<f32>>,
    muted: &AtomicBool,
) {
    if muted.load(Ordering::SeqCst) {
        let _ = tx.send(vec![0.0; data.len() / channels.max(1)]);
    } else {
        let _ = tx.send(mix_to_mono(data, channels));
    }
}

fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    playback_buffer: Arc<Mutex<VecDeque<f32>>>,
) -> anyhow::Result<cpal::Stream> {
    let err_fn = |err| eprintln!("output stream error: {err}");
    let sample_format = device.default_output_config()?.sample_format();
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config,
            move |data: &mut [f32], _| fill_output(data, channels, &playback_buffer, |s| s),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            config,
            move |data: &mut [i16], _| fill_output(data, channels, &playback_buffer, float_to_i16),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            config,
            move |data: &mut [u16], _| {
                fill_output(data, channels, &playback_buffer, |sample| {
                    ((sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16
                })
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported output sample format {other:?}"),
    };
    Ok(stream)
}

fn fill_output<T: Copy>(
    data: &mut [T],
    channels: usize,
    playback_buffer: &Arc<Mutex<VecDeque<f32>>>,
    convert: impl Fn(f32) -> T,
) {
    let zero = convert(0.0);
    if let Ok(mut buffer) = playback_buffer.lock() {
        for frame in data.chunks_mut(channels.max(1)) {
            let sample = buffer.pop_front().unwrap_or(0.0);
            let converted = convert(sample);
            for out in frame {
                *out = converted;
            }
        }
    } else {
        for out in data {
            *out = zero;
        }
    }
}

fn mix_to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect()
}

pub(crate) fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == to_rate {
        return input.to_vec();
    }
    let out_len = ((input.len() as u64 * to_rate as u64) / from_rate as u64).max(1) as usize;
    let ratio = from_rate as f32 / to_rate as f32;
    let mut output = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f32 * ratio;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        output.push(a + (b - a) * frac);
    }
    output
}

pub(crate) fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn choose_device(
    host: &cpal::Host,
    input: bool,
    selector: Option<&str>,
) -> anyhow::Result<cpal::Device> {
    if let Some(selector) = selector {
        let devices = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        }
        .collect::<Vec<_>>();

        if let Ok(index) = selector.parse::<usize>() {
            return devices
                .into_iter()
                .nth(index)
                .ok_or_else(|| anyhow::anyhow!("audio device index {index} not found"));
        }

        let needle = selector.to_ascii_lowercase();
        return devices
            .into_iter()
            .find(|device| {
                device
                    .name()
                    .map(|name| name.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow::anyhow!("audio device matching '{selector}' not found"));
    }

    if input {
        host.default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no default input device"))
    } else {
        host.default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no default output device"))
    }
}

pub(crate) fn list_audio_devices() -> anyhow::Result<()> {
    println!("Input devices:");
    print_device_list(true);
    println!();
    println!("Output devices:");
    print_device_list(false);
    Ok(())
}

pub(crate) fn audio_device_summary(
    input_selector: Option<&str>,
    output_selector: Option<&str>,
    active_route: &str,
) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Active route: {}",
            if active_route.is_empty() {
                "stopped"
            } else {
                active_route
            }
        ),
        format!("Input selector: {}", input_selector.unwrap_or("default")),
        format!("Output selector: {}", output_selector.unwrap_or("default")),
        String::new(),
        "Input devices:".into(),
    ];

    append_device_lines(&mut lines, true);
    lines.push(String::new());
    lines.push("Output devices:".into());
    append_device_lines(&mut lines, false);
    lines
}

fn print_device_list(input: bool) {
    match collect_device_names(input, Duration::from_secs(3)) {
        Ok(names) if names.is_empty() => println!("  <none>"),
        Ok(names) => {
            for (idx, name) in names.iter().enumerate() {
                println!("  {idx}: {name}");
            }
        }
        Err(message) => println!("  <{message}>"),
    }
}

fn append_device_lines(lines: &mut Vec<String>, input: bool) {
    match collect_device_names(input, Duration::from_secs(1)) {
        Ok(names) if names.is_empty() => lines.push("  <none>".into()),
        Ok(names) => {
            for (idx, name) in names.iter().enumerate() {
                lines.push(format!("  {idx}: {name}"));
            }
        }
        Err(message) => lines.push(format!("  <{message}>")),
    }
}

fn collect_device_names(input: bool, timeout: Duration) -> Result<Vec<String>, String> {
    let (tx, rx) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = collect_device_names_inner(input).map_err(|err| err.to_string());
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(std_mpsc::RecvTimeoutError::Timeout) => {
            Err("timed out while enumerating devices".into())
        }
        Err(std_mpsc::RecvTimeoutError::Disconnected) => {
            Err("device enumeration thread stopped".into())
        }
    }
}

fn collect_device_names_inner(input: bool) -> anyhow::Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = if input {
        host.input_devices()?
    } else {
        host.output_devices()?
    };
    Ok(devices
        .map(|device| device.name().unwrap_or_else(|_| "<unknown>".into()))
        .collect())
}
