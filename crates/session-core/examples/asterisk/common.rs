#![allow(dead_code)]

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_session_core::{
    api::unified::RegistrationHandle, types::Credentials, Config, Registration, SessionHandle,
    SipContactMode, StreamPeer,
};
use tokio::time::sleep;

pub type ExampleResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const SAMPLE_RATE: u32 = 8000;
pub const FRAME_SIZE: usize = 160;
pub const TONE_FRAMES: usize = 150;
pub const ENDPOINT_2001_TONE_HZ: f32 = 440.0;
pub const ENDPOINT_2002_TONE_HZ: f32 = 880.0;
pub const ENDPOINT_1001_TONE_HZ: f32 = ENDPOINT_2001_TONE_HZ;
pub const ENDPOINT_1002_TONE_HZ: f32 = ENDPOINT_2002_TONE_HZ;
pub const MIN_RECEIVED_SAMPLES: usize = 12_000;
pub const DOMINANCE_RATIO: f32 = 5.0;
pub const DEFAULT_POST_REGISTER_SETTLE_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsContactMode {
    ReachableContact,
    RegisteredFlowRfc5626,
    RegisteredFlowSymmetric,
}

impl TlsContactMode {
    fn from_env() -> ExampleResult<Self> {
        if env_bool("ASTERISK_TLS_FLOW_REUSE", false)? {
            return Ok(Self::RegisteredFlowSymmetric);
        }

        match env_string("ASTERISK_TLS_CONTACT_MODE", "reachable-contact")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "reachable-contact" | "reachable" | "listener" | "uas" => Ok(Self::ReachableContact),
            "registered-flow" | "registered-flow-rfc5626" | "rfc5626" | "outbound" => {
                Ok(Self::RegisteredFlowRfc5626)
            }
            "registered-flow-symmetric" | "symmetric" | "symmetric-transport"
            | "flow-reuse" | "client-only" => Ok(Self::RegisteredFlowSymmetric),
            other => Err(format!(
                "ASTERISK_TLS_CONTACT_MODE must be reachable-contact, registered-flow-rfc5626, or registered-flow-symmetric, got '{}'",
                other
            )
            .into()),
        }
    }

    fn uses_listener(self) -> bool {
        self == Self::ReachableContact
    }

    fn sip_contact_mode(self) -> SipContactMode {
        match self {
            Self::ReachableContact => SipContactMode::ReachableContact,
            Self::RegisteredFlowRfc5626 => SipContactMode::RegisteredFlowRfc5626,
            Self::RegisteredFlowSymmetric => SipContactMode::RegisteredFlowSymmetric,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ReachableContact => "reachable-contact",
            Self::RegisteredFlowRfc5626 => "registered-flow-rfc5626",
            Self::RegisteredFlowSymmetric => "registered-flow-symmetric",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointConfig {
    pub username: String,
    pub auth_username: String,
    pub password: String,
    pub sip_server: String,
    pub sip_port: u16,
    pub transport: String,
    pub local_ip: IpAddr,
    pub advertised_ip: IpAddr,
    pub local_port: u16,
    pub tls_local_port: Option<u16>,
    pub tls_contact_mode: TlsContactMode,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub output_dir: PathBuf,
}

impl EndpointConfig {
    pub fn registrar_uri(&self) -> String {
        format!(
            "{}:{}:{}{}",
            self.uri_scheme(),
            self.sip_server,
            self.sip_port,
            transport_suffix(&self.transport)
        )
    }

    pub fn aor_uri(&self) -> String {
        format!(
            "{}:{}@{}",
            self.uri_scheme(),
            self.username,
            self.sip_server
        )
    }

    pub fn contact_uri(&self) -> String {
        format!(
            "{}:{}@{}:{}{}",
            self.uri_scheme(),
            self.username,
            self.advertised_ip,
            self.contact_port(),
            transport_suffix(&self.transport)
        )
    }

    pub fn call_uri(&self, target: &str) -> String {
        let scheme = self.uri_scheme();
        if self.is_tls() {
            format!(
                "{}:{}@{}:{}{}",
                scheme,
                target,
                self.sip_server,
                self.sip_port,
                transport_suffix(&self.transport)
            )
        } else if self.sip_port == default_port_for_transport(&self.transport) {
            format!("{}:{}@{}", scheme, target, self.sip_server)
        } else {
            format!(
                "{}:{}@{}:{}",
                scheme, target, self.sip_server, self.sip_port
            )
        }
    }

    pub fn outbound_call_uri(&self, target: &str) -> String {
        let key = format!("ENDPOINT_{}_CALL_URI", self.username);
        std::env::var(&key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.call_uri(target))
    }

    pub fn stream_config(&self) -> Config {
        let mut config = Config::on(&self.username, self.local_ip, self.local_port);
        config.local_uri = self.aor_uri();
        config.contact_uri = Some(self.contact_uri());
        config.sip_contact_mode = if self.is_tls() {
            self.tls_contact_mode.sip_contact_mode()
        } else {
            SipContactMode::ReachableContact
        };
        config.credentials = Some(Credentials::new(&self.auth_username, &self.password));
        config.media_port_start = self.media_port_start;
        config.media_port_end = self.media_port_end;
        config
    }

    pub fn tls_srtp_stream_config(&self) -> ExampleResult<Config> {
        if !self.is_tls() {
            return Err("tls_srtp_hold_resume requires SIP_TRANSPORT=TLS".into());
        }

        let mut config = self.stream_config();
        match self.tls_contact_mode {
            TlsContactMode::ReachableContact => {
                let tls_port = self.tls_local_port.ok_or_else(|| {
                    "TLS reachable-contact mode requires ENDPOINT_<user>_TLS_LOCAL_PORT".to_string()
                })?;
                config = config.tls_reachable_contact(
                    SocketAddr::new(self.local_ip, tls_port),
                    required_path("TLS_CERT_PATH")?,
                    required_path("TLS_KEY_PATH")?,
                );
            }
            TlsContactMode::RegisteredFlowRfc5626 => {
                config = config.tls_registered_flow_rfc5626(self.sip_instance_urn());
            }
            TlsContactMode::RegisteredFlowSymmetric => {
                config = config.tls_registered_flow_symmetric(self.sip_instance_urn());
            }
        }
        config.tls_extra_ca_path = optional_path("TLS_CA_PATH");
        config.tls_client_cert_path = optional_path("TLS_CLIENT_CERT_PATH");
        config.tls_client_key_path = optional_path("TLS_CLIENT_KEY_PATH");
        #[cfg(feature = "dev-insecure-tls")]
        {
            config.tls_insecure_skip_verify = env_bool("TLS_INSECURE", false)?;
        }
        config.offer_srtp = true;
        config.srtp_required = env_bool("ASTERISK_TLS_SRTP_REQUIRED", true)?;
        Ok(config)
    }

    pub fn registration(&self) -> Registration {
        Registration::new(self.registrar_uri(), &self.auth_username, &self.password)
            .from_uri(self.aor_uri())
            .contact_uri(self.contact_uri())
    }

    fn is_tls(&self) -> bool {
        self.transport.eq_ignore_ascii_case("tls")
    }

    fn uri_scheme(&self) -> &'static str {
        if self.is_tls() {
            "sips"
        } else {
            "sip"
        }
    }

    fn contact_port(&self) -> u16 {
        if self.is_tls() && self.tls_contact_mode.uses_listener() {
            self.tls_local_port
                .unwrap_or(self.local_port.saturating_add(1))
        } else {
            self.local_port
        }
    }

    fn sip_instance_urn(&self) -> String {
        std::env::var(format!("ENDPOINT_{}_SIP_INSTANCE", self.username))
            .or_else(|_| std::env::var("SIP_INSTANCE"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| deterministic_sip_instance(&self.username))
    }
}

#[derive(Debug, Clone)]
pub struct ToneAnalysis {
    pub samples: usize,
    pub expected_hz: f32,
    pub rejected_hz: f32,
    pub expected_magnitude: f32,
    pub rejected_magnitude: f32,
    pub ratio: f32,
}

pub fn load_env() {
    let env_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/asterisk/.env");
    let _ = dotenvy::from_filename(env_path);
    let _ = dotenvy::dotenv();
}

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_dialog_core=warn".into()),
        )
        .try_init();
}

pub fn endpoint_config(
    username: &str,
    default_local_port: u16,
    default_media_start: u16,
    default_media_end: u16,
) -> ExampleResult<EndpointConfig> {
    let prefix = format!("ENDPOINT_{}", username);
    let sip_server = env_string("SIP_SERVER", "192.168.1.103");
    let transport = env_string("SIP_TRANSPORT", "UDP").to_lowercase();
    let sip_port = if transport == "tls" {
        env_u16("SIP_TLS_PORT", 5061)?
    } else {
        env_u16("SIP_PORT", 5060)?
    };
    let auth_username = env_string(&format!("{}_AUTH_USERNAME", prefix), username);
    let password = env_string("SIP_PASSWORD", "password123");
    let local_ip: IpAddr = env_string("LOCAL_IP", "0.0.0.0").parse()?;
    let advertised_ip = match std::env::var("ADVERTISED_IP") {
        Ok(value) => value.parse()?,
        Err(_) if !local_ip.is_unspecified() => local_ip,
        Err(_) => {
            return Err("ADVERTISED_IP is required when LOCAL_IP is 0.0.0.0 or ::".into());
        }
    };
    let local_port = env_u16(&format!("{}_LOCAL_PORT", prefix), default_local_port)?;
    let tls_contact_mode = TlsContactMode::from_env()?;
    let tls_local_port = if transport == "tls" {
        Some(env_u16(
            &format!("{}_TLS_LOCAL_PORT", prefix),
            default_local_port.saturating_add(1),
        )?)
    } else {
        None
    };
    let media_port_start = env_u16(&format!("{}_MEDIA_PORT_START", prefix), default_media_start)?;
    let media_port_end = env_u16(&format!("{}_MEDIA_PORT_END", prefix), default_media_end)?;
    let output_dir = std::env::var("AUDIO_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/asterisk/output"));

    Ok(EndpointConfig {
        username: username.to_string(),
        auth_username,
        password,
        sip_server,
        sip_port,
        transport,
        local_ip,
        advertised_ip,
        local_port,
        tls_local_port,
        tls_contact_mode,
        media_port_start,
        media_port_end,
        output_dir,
    })
}

pub fn post_register_settle_duration() -> ExampleResult<Duration> {
    let secs = std::env::var("POST_REGISTER_SETTLE_SECS")
        .unwrap_or_else(|_| DEFAULT_POST_REGISTER_SETTLE_SECS.to_string())
        .parse()?;
    Ok(Duration::from_secs(secs))
}

pub async fn register_endpoint(
    peer: &mut StreamPeer,
    cfg: &EndpointConfig,
) -> ExampleResult<RegistrationHandle> {
    println!(
        "[{}] Local bind: {}:{}",
        cfg.username, cfg.local_ip, cfg.local_port
    );
    if cfg.is_tls() {
        println!(
            "[{}] TLS mode:   {}{}",
            cfg.username,
            cfg.tls_contact_mode.label(),
            cfg.tls_local_port
                .map(|port| format!(" (listener {}:{})", cfg.local_ip, port))
                .unwrap_or_default()
        );
    }
    println!("[{}] AOR:        {}", cfg.username, cfg.aor_uri());
    println!("[{}] Contact:    {}", cfg.username, cfg.contact_uri());
    println!("[{}] Registrar:  {}", cfg.username, cfg.registrar_uri());
    println!(
        "[{}] Media ports: {}-{}",
        cfg.username, cfg.media_port_start, cfg.media_port_end
    );
    println!("[{}] Registering...", cfg.username);

    let handle = peer.register_with(cfg.registration()).await?;
    wait_for_registration(peer, &handle, &cfg.username).await?;
    println!("[{}] Registered.", cfg.username);
    Ok(handle)
}

pub async fn wait_for_registration(
    peer: &StreamPeer,
    handle: &RegistrationHandle,
    username: &str,
) -> ExampleResult<()> {
    for _ in 0..50 {
        if peer.is_registered(handle).await? {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("endpoint {} did not register within 10s", username).into())
}

pub fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..FRAME_SIZE)
        .map(|j| {
            let t = (frame_num * FRAME_SIZE + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

pub async fn exchange_tone_and_record(
    handle: &SessionHandle,
    tone_hz: f32,
    output_dir: &Path,
    output_name: &str,
    hangup_after_tone: bool,
) -> ExampleResult<PathBuf> {
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();

    let received_buf = Arc::new(Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });

    for i in 0..TONE_FRAMES {
        let frame = AudioFrame::new(
            generate_tone(tone_hz, i),
            SAMPLE_RATE,
            1,
            (i * FRAME_SIZE) as u32,
        );
        if sender.send(frame).await.is_err() {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }

    drop(sender);
    if hangup_after_tone {
        println!("[audio] Tone complete; hanging up.");
        handle.hangup().await?;
        handle.wait_for_end(Some(Duration::from_secs(8))).await.ok();
    } else {
        handle
            .wait_for_end(Some(Duration::from_secs(10)))
            .await
            .ok();
    }

    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if recv_task.is_finished() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    recv_task.abort();

    let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();
    save_wav(output_dir, output_name, &received)
}

pub fn save_wav(out_dir: &Path, name: &str, samples: &[i16]) -> ExampleResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join(name);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    println!("Saved {} ({} samples)", path.display(), samples.len());
    Ok(path)
}

pub fn read_wav(path: &Path) -> ExampleResult<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let samples = reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?;
    Ok(samples)
}

pub fn analyze_wav(path: &Path, expected_hz: f32, rejected_hz: f32) -> ExampleResult<ToneAnalysis> {
    analyze_samples(&read_wav(path)?, expected_hz, rejected_hz)
}

pub fn analyze_samples(
    samples: &[i16],
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let expected_magnitude = goertzel_magnitude(samples, SAMPLE_RATE as f32, expected_hz);
    let rejected_magnitude = goertzel_magnitude(samples, SAMPLE_RATE as f32, rejected_hz);
    let ratio = if rejected_magnitude > 1.0 {
        expected_magnitude / rejected_magnitude
    } else {
        f32::INFINITY
    };

    Ok(ToneAnalysis {
        samples: samples.len(),
        expected_hz,
        rejected_hz,
        expected_magnitude,
        rejected_magnitude,
        ratio,
    })
}

pub fn assert_audio_path(
    path: &Path,
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let analysis = analyze_wav(path, expected_hz, rejected_hz)?;
    if analysis.samples < MIN_RECEIVED_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            path.display(),
            analysis.samples,
            MIN_RECEIVED_SAMPLES
        )
        .into());
    }
    if analysis.ratio < DOMINANCE_RATIO {
        return Err(format!(
            "{}: {:.0}Hz magnitude {:.1} vs {:.0}Hz magnitude {:.1}, ratio {:.2} (expected at least {:.2})",
            path.display(),
            analysis.expected_hz,
            analysis.expected_magnitude,
            analysis.rejected_hz,
            analysis.rejected_magnitude,
            analysis.ratio,
            DOMINANCE_RATIO
        )
        .into());
    }
    Ok(analysis)
}

pub fn print_analysis(label: &str, path: &Path, analysis: &ToneAnalysis) {
    println!(
        "{}: {} samples, {:.0}Hz magnitude {:.1}, {:.0}Hz magnitude {:.1}, ratio {:.2}",
        label,
        analysis.samples,
        analysis.expected_hz,
        analysis.expected_magnitude,
        analysis.rejected_hz,
        analysis.rejected_magnitude,
        analysis.ratio
    );
    println!("{} WAV: {}", label, path.display());
}

pub fn goertzel_magnitude(samples: &[i16], sample_rate: f32, target_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f32;
    let k = (0.5 + (n * target_hz) / sample_rate).floor();
    let omega = (2.0 * std::f32::consts::PI * k) / n;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &s in samples {
        let q0 = coeff * q1 - q2 + (s as f32);
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

fn env_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> ExampleResult<u16> {
    Ok(std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .parse()?)
}

fn env_bool(key: &str, default: bool) -> ExampleResult<bool> {
    let value = match std::env::var(key) {
        Ok(value) => value,
        Err(_) => return Ok(default),
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{} must be a boolean value", key).into()),
    }
}

fn deterministic_sip_instance(username: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in username.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!(
        "urn:uuid:00000000-0000-4000-8000-{:012x}",
        hash & 0xffff_ffff_ffff
    )
}

fn required_path(key: &str) -> ExampleResult<PathBuf> {
    let value =
        std::env::var(key).map_err(|_| format!("{} must be set for SIP_TRANSPORT=TLS", key))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{} must not be empty", key).into());
    }
    Ok(PathBuf::from(value))
}

fn optional_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_port_for_transport(transport: &str) -> u16 {
    if transport.eq_ignore_ascii_case("tls") {
        5061
    } else {
        5060
    }
}

fn transport_suffix(transport: &str) -> &'static str {
    match transport.to_lowercase().as_str() {
        "tcp" => ";transport=tcp",
        "tls" => ";transport=tls",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_440hz_wav_passes_440hz_detection() {
        let samples = (0..TONE_FRAMES)
            .flat_map(|i| generate_tone(ENDPOINT_1001_TONE_HZ, i))
            .collect::<Vec<_>>();
        let tmp =
            std::env::temp_dir().join(format!("rvoip_asterisk_440_{}.wav", uuid::Uuid::new_v4()));
        let out_dir = tmp.parent().unwrap();
        let name = tmp.file_name().unwrap().to_string_lossy().to_string();
        save_wav(out_dir, &name, &samples).unwrap();

        let analysis =
            assert_audio_path(&tmp, ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ).unwrap();
        assert!(analysis.ratio >= DOMINANCE_RATIO);
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn generated_880hz_wav_passes_880hz_detection() {
        let samples = (0..TONE_FRAMES)
            .flat_map(|i| generate_tone(ENDPOINT_1002_TONE_HZ, i))
            .collect::<Vec<_>>();
        let tmp =
            std::env::temp_dir().join(format!("rvoip_asterisk_880_{}.wav", uuid::Uuid::new_v4()));
        let out_dir = tmp.parent().unwrap();
        let name = tmp.file_name().unwrap().to_string_lossy().to_string();
        save_wav(out_dir, &name, &samples).unwrap();

        let analysis =
            assert_audio_path(&tmp, ENDPOINT_1002_TONE_HZ, ENDPOINT_1001_TONE_HZ).unwrap();
        assert!(analysis.ratio >= DOMINANCE_RATIO);
        let _ = std::fs::remove_file(tmp);
    }
}
