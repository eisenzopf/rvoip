//! Custom PBX Builder Example
//!
//! This example demonstrates how to build a custom PBX system using the RVOIP builder
//! for maximum flexibility and customization.

use rvoip_builder::*;
use tracing::{info, warn, error, debug};
use tokio::time::{sleep, Duration};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🏗️  Starting Custom PBX Builder Example");

    // Create custom PBX platform
    let mut pbx_builder = CustomPbxBuilder::new().await?;
    
    // Run comprehensive demonstration
    pbx_builder.run_demo().await?;

    info!("✅ Custom PBX builder example completed!");
    Ok(())
}

/// Custom PBX builder with advanced composition patterns
struct CustomPbxBuilder {
    platform: VoipPlatform,
    custom_components: HashMap<String, Box<dyn CustomComponent>>,
    routing_engine: AdvancedRoutingEngine,
    media_processing: MediaProcessingPipeline,
    analytics_engine: AnalyticsEngine,
    monitoring_system: MonitoringSystem,
}

/// Custom component trait for extensible architecture
#[async_trait::async_trait]
trait CustomComponent: Send + Sync {
    /// Component identifier
    fn id(&self) -> &str;
    
    /// Component type
    fn component_type(&self) -> ComponentType;
    
    /// Initialize the component
    async fn initialize(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Start the component
    async fn start(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Stop the component
    async fn stop(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Get component health
    async fn health(&self) -> ComponentHealth;
    
    /// Process custom events
    async fn process_event(&self, event: CustomEvent) -> Result<(), VoipBuilderError>;
}

/// Component type enumeration
#[derive(Debug, Clone, PartialEq)]
enum ComponentType {
    DialPlan,
    MediaProcessor,
    Analytics,
    Integration,
    Security,
    Monitoring,
}

/// Custom events for component communication
#[derive(Debug, Clone)]
enum CustomEvent {
    CallInitiated(CallInfo),
    CallEnded(CallInfo, CallMetrics),
    MediaStreamCreated(String, MediaStreamInfo),
    SecurityAlert(SecurityAlert),
    SystemMetric(String, f64),
    ConfigurationChanged(String, String),
}

/// Call information
#[derive(Debug, Clone)]
struct CallInfo {
    call_id: String,
    from: String,
    to: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    call_type: CallType,
}

/// Call type
#[derive(Debug, Clone)]
enum CallType {
    Internal,
    Inbound,
    Outbound,
    Conference,
    Transfer,
}

/// Call metrics
#[derive(Debug, Clone)]
struct CallMetrics {
    duration: Duration,
    quality: f32,
    codec: String,
    packet_loss: f32,
    jitter: Duration,
}

/// Media stream information
#[derive(Debug, Clone)]
struct MediaStreamInfo {
    stream_id: String,
    codec: String,
    bitrate: u32,
    direction: MediaDirection,
}

/// Media direction
#[derive(Debug, Clone)]
enum MediaDirection {
    Inbound,
    Outbound,
    Bidirectional,
}

/// Security alert
#[derive(Debug, Clone)]
struct SecurityAlert {
    alert_type: SecurityAlertType,
    severity: AlertSeverity,
    description: String,
    source: String,
}

/// Security alert type
#[derive(Debug, Clone)]
enum SecurityAlertType {
    UnauthorizedAccess,
    SuspiciousTraffic,
    EncryptionFailure,
    CertificateExpiration,
    LoginFailure,
}

/// Alert severity
#[derive(Debug, Clone)]
enum AlertSeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Advanced routing engine
#[derive(Debug)]
struct AdvancedRoutingEngine {
    dial_plans: HashMap<String, DialPlan>,
    routing_rules: Vec<RoutingRule>,
    failover_rules: Vec<FailoverRule>,
    load_balancing: LoadBalancingConfig,
}

/// Dial plan configuration
#[derive(Debug)]
struct DialPlan {
    name: String,
    patterns: Vec<DialPattern>,
    transformations: Vec<NumberTransformation>,
    conditions: Vec<RoutingCondition>,
}

/// Dial pattern
#[derive(Debug)]
struct DialPattern {
    pattern: String,
    priority: u32,
    action: DialAction,
}

/// Dial action
#[derive(Debug)]
enum DialAction {
    Route(String),
    Reject(String),
    Redirect(String),
    PlayMessage(String),
    Conference(String),
}

/// Number transformation
#[derive(Debug)]
struct NumberTransformation {
    pattern: String,
    replacement: String,
    direction: TransformDirection,
}

/// Transform direction
#[derive(Debug)]
enum TransformDirection {
    Inbound,
    Outbound,
    Both,
}

/// Routing condition
#[derive(Debug)]
struct RoutingCondition {
    condition_type: ConditionType,
    value: String,
    operator: ConditionOperator,
}

/// Condition type
#[derive(Debug)]
enum ConditionType {
    Time,
    CallerID,
    DID,
    UserGroup,
    CallVolume,
}

/// Condition operator
#[derive(Debug)]
enum ConditionOperator {
    Equals,
    Contains,
    Regex,
    Range,
    Greater,
    Less,
}

/// Routing rule
#[derive(Debug)]
struct RoutingRule {
    name: String,
    pattern: String,
    destination: RoutingDestination,
    conditions: Vec<RoutingCondition>,
    priority: u32,
}

/// Routing destination
#[derive(Debug)]
enum RoutingDestination {
    Extension(String),
    Gateway(String),
    HuntGroup(String),
    Queue(String),
    Voicemail(String),
    Conference(String),
}

/// Failover rule
#[derive(Debug)]
struct FailoverRule {
    primary_destination: String,
    backup_destinations: Vec<String>,
    failover_conditions: Vec<FailoverCondition>,
    retry_interval: Duration,
}

/// Failover condition
#[derive(Debug)]
enum FailoverCondition {
    Unreachable,
    Busy,
    Timeout,
    Quality,
}

/// Load balancing configuration
#[derive(Debug)]
struct LoadBalancingConfig {
    algorithm: LoadBalancingAlgorithm,
    weight_distribution: HashMap<String, u32>,
    health_check_interval: Duration,
}

/// Load balancing algorithm
#[derive(Debug)]
enum LoadBalancingAlgorithm {
    RoundRobin,
    WeightedRoundRobin,
    LeastConnections,
    Random,
    HealthBased,
}

/// Media processing pipeline
#[derive(Debug)]
struct MediaProcessingPipeline {
    processors: Vec<MediaProcessor>,
    codecs: Vec<CodecConfig>,
    quality_settings: QualitySettings,
    recording_config: RecordingConfig,
}

/// Media processor
#[derive(Debug)]
struct MediaProcessor {
    name: String,
    processor_type: MediaProcessorType,
    settings: HashMap<String, String>,
    enabled: bool,
}

/// Media processor type
#[derive(Debug)]
enum MediaProcessorType {
    EchoCancellation,
    NoiseReduction,
    AutoGainControl,
    VoiceActivityDetection,
    DTMF,
    Transcoding,
    Recording,
    Mixing,
}

/// Codec configuration
#[derive(Debug)]
struct CodecConfig {
    name: String,
    priority: u32,
    settings: HashMap<String, String>,
    bandwidth_usage: u32,
}

/// Quality settings
#[derive(Debug)]
struct QualitySettings {
    target_mos: f32,
    max_packet_loss: f32,
    max_jitter: Duration,
    adaptive_quality: bool,
}

/// Recording configuration
#[derive(Debug)]
struct RecordingConfig {
    enabled: bool,
    format: RecordingFormat,
    compression: CompressionLevel,
    storage_location: String,
    retention_policy: Duration,
}

/// Recording format
#[derive(Debug)]
enum RecordingFormat {
    WAV,
    MP3,
    OGG,
    FLAC,
}

/// Compression level
#[derive(Debug)]
enum CompressionLevel {
    None,
    Low,
    Medium,
    High,
}

/// Analytics engine
#[derive(Debug)]
struct AnalyticsEngine {
    metrics_collectors: Vec<MetricsCollector>,
    reporting_config: ReportingConfig,
    alerting_rules: Vec<AlertingRule>,
    dashboards: Vec<Dashboard>,
}

/// Metrics collector
#[derive(Debug)]
struct MetricsCollector {
    name: String,
    metric_type: MetricType,
    collection_interval: Duration,
    retention_period: Duration,
}

/// Metric type
#[derive(Debug)]
enum MetricType {
    CallVolume,
    CallQuality,
    SystemPerformance,
    UserActivity,
    Security,
    Billing,
}

/// Reporting configuration
#[derive(Debug)]
struct ReportingConfig {
    enabled: bool,
    report_formats: Vec<ReportFormat>,
    delivery_schedule: Vec<ReportSchedule>,
    recipients: Vec<String>,
}

/// Report format
#[derive(Debug)]
enum ReportFormat {
    PDF,
    Excel,
    CSV,
    JSON,
    HTML,
}

/// Report schedule
#[derive(Debug)]
struct ReportSchedule {
    frequency: ReportFrequency,
    time: String,
    report_type: String,
}

/// Report frequency
#[derive(Debug)]
enum ReportFrequency {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    OnDemand,
}

/// Alerting rule
#[derive(Debug)]
struct AlertingRule {
    name: String,
    condition: AlertCondition,
    threshold: f64,
    action: AlertAction,
    enabled: bool,
}

/// Alert condition
#[derive(Debug)]
enum AlertCondition {
    Greater,
    Less,
    Equals,
    NotEquals,
    RateOfChange,
}

/// Alert action
#[derive(Debug)]
enum AlertAction {
    Email(Vec<String>),
    SMS(Vec<String>),
    Webhook(String),
    SNMP(String),
}

/// Dashboard configuration
#[derive(Debug)]
struct Dashboard {
    name: String,
    widgets: Vec<DashboardWidget>,
    refresh_interval: Duration,
    permissions: Vec<String>,
}

/// Dashboard widget
#[derive(Debug)]
struct DashboardWidget {
    widget_type: WidgetType,
    title: String,
    data_source: String,
    settings: HashMap<String, String>,
}

/// Widget type
#[derive(Debug)]
enum WidgetType {
    LineChart,
    BarChart,
    Gauge,
    Counter,
    Table,
    Map,
}

/// Monitoring system
#[derive(Debug)]
struct MonitoringSystem {
    health_checks: Vec<HealthCheck>,
    performance_monitors: Vec<PerformanceMonitor>,
    log_aggregation: LogAggregationConfig,
    tracing_config: TracingConfig,
}

/// Health check
#[derive(Debug)]
struct HealthCheck {
    name: String,
    target: String,
    check_type: HealthCheckType,
    interval: Duration,
    timeout: Duration,
    retry_count: u32,
}

/// Health check type
#[derive(Debug)]
enum HealthCheckType {
    HTTP,
    TCP,
    SIP,
    RTP,
    Database,
    Custom,
}

/// Performance monitor
#[derive(Debug)]
struct PerformanceMonitor {
    name: String,
    metric_name: String,
    collection_method: CollectionMethod,
    thresholds: PerformanceThresholds,
}

/// Collection method
#[derive(Debug)]
enum CollectionMethod {
    SystemMetrics,
    ApplicationMetrics,
    NetworkMetrics,
    CustomScript,
}

/// Performance thresholds
#[derive(Debug)]
struct PerformanceThresholds {
    warning: f64,
    critical: f64,
    unit: String,
}

/// Log aggregation configuration
#[derive(Debug)]
struct LogAggregationConfig {
    enabled: bool,
    log_level: String,
    output_format: LogFormat,
    destinations: Vec<LogDestination>,
}

/// Log format
#[derive(Debug)]
enum LogFormat {
    JSON,
    Text,
    Syslog,
    CEF,
}

/// Log destination
#[derive(Debug)]
enum LogDestination {
    File(String),
    Syslog(String),
    Elasticsearch(String),
    Kafka(String),
}

/// Tracing configuration
#[derive(Debug)]
struct TracingConfig {
    enabled: bool,
    sampling_rate: f64,
    trace_storage: String,
    correlation_enabled: bool,
}

impl CustomPbxBuilder {
    /// Create a new custom PBX builder
    async fn new() -> Result<Self, VoipBuilderError> {
        info!("🏗️  Initializing Custom PBX Builder");

        // Build the base VoIP platform with custom configuration
        let platform = VoipPlatform::new("custom-pbx")
            .environment(Environment::Production)
            .with_tag("type", "custom-pbx")
            .with_tag("version", "2.0")
            .with_sip_stack(SipStackConfig::custom())
            .with_rtp_engine(RtpEngineConfig::secure())
            .with_call_engine(CallEngineConfig::enterprise())
            .with_api_server(ApiServerConfig::rest_and_websocket())
            .build().await?;

        info!("✅ Base platform created");
        info!("   Platform ID: {}", platform.id);
        info!("   Environment: Production");
        info!("   Components: SIP Stack, RTP Engine, Call Engine, API Server");

        // Initialize custom components
        let custom_components = Self::create_custom_components().await?;
        let routing_engine = Self::create_routing_engine();
        let media_processing = Self::create_media_pipeline();
        let analytics_engine = Self::create_analytics_engine();
        let monitoring_system = Self::create_monitoring_system();

        Ok(Self {
            platform,
            custom_components,
            routing_engine,
            media_processing,
            analytics_engine,
            monitoring_system,
        })
    }

    /// Run comprehensive demonstration
    async fn run_demo(&mut self) -> Result<(), VoipBuilderError> {
        info!("🚀 Starting Custom PBX Builder Demonstration");

        // Start the platform
        self.platform.start().await?;
        
        // Component architecture
        self.demo_component_architecture().await?;
        
        // Advanced routing
        self.demo_advanced_routing().await?;
        
        // Media processing
        self.demo_media_processing().await?;
        
        // Analytics and monitoring
        self.demo_analytics_monitoring().await?;
        
        // Custom integrations
        self.demo_custom_integrations().await?;
        
        // Scalability and performance
        self.demo_scalability().await?;

        // Stop the platform
        self.platform.stop().await?;

        Ok(())
    }

    /// Demonstrate component architecture
    async fn demo_component_architecture(&self) -> Result<(), VoipBuilderError> {
        info!("🏗️  Demo: Component Architecture");

        // Show platform overview
        info!("📊 Platform Overview:");
        info!("   Status: {:?}", self.platform.status().await);
        let metrics = self.platform.metrics().await;
        info!("   CPU Usage: {:.1}%", metrics.cpu_usage);
        info!("   Memory Usage: {} MB", metrics.memory_usage / 1024 / 1024);
        info!("   Active Sessions: {}", metrics.active_sessions);

        // Show custom components
        info!("🔧 Custom Components:");
        for (id, component) in &self.custom_components {
            let health = component.health().await;
            info!("   {}: {:?} - {:?}", id, component.component_type(), health.status);
        }

        // Show component communication
        info!("📡 Component Communication:");
        info!("   Event Bus: Enabled");
        info!("   Message Routing: Automatic");
        info!("   Health Monitoring: Active");
        info!("   Load Balancing: Dynamic");

        // Demonstrate event flow
        self.demo_event_flow().await?;

        Ok(())
    }

    /// Demonstrate event flow
    async fn demo_event_flow(&self) -> Result<(), VoipBuilderError> {
        info!("📨 Demo: Event Flow");

        // Simulate call event
        let call_info = CallInfo {
            call_id: "demo-call-001".to_string(),
            from: "alice@company.com".to_string(),
            to: "bob@company.com".to_string(),
            timestamp: chrono::Utc::now(),
            call_type: CallType::Internal,
        };

        info!("📞 Simulating call initiation:");
        info!("   Call ID: {}", call_info.call_id);
        info!("   From: {}", call_info.from);
        info!("   To: {}", call_info.to);

        // Process through components
        let event = CustomEvent::CallInitiated(call_info.clone());
        for (id, component) in &self.custom_components {
            component.process_event(event.clone()).await?;
            info!("   ✅ Processed by {}", id);
            sleep(Duration::from_millis(50)).await;
        }

        // Simulate call completion
        let metrics = CallMetrics {
            duration: Duration::from_secs(180),
            quality: 4.2,
            codec: "Opus".to_string(),
            packet_loss: 0.1,
            jitter: Duration::from_millis(15),
        };

        info!("📞 Call completed with metrics:");
        info!("   Duration: {:?}", metrics.duration);
        info!("   Quality (MOS): {}", metrics.quality);
        info!("   Codec: {}", metrics.codec);
        info!("   Packet Loss: {}%", metrics.packet_loss);

        let completion_event = CustomEvent::CallEnded(call_info, metrics);
        for (id, component) in &self.custom_components {
            component.process_event(completion_event.clone()).await?;
        }

        info!("✅ Event flow demonstration completed");

        Ok(())
    }

    /// Demonstrate advanced routing
    async fn demo_advanced_routing(&self) -> Result<(), VoipBuilderError> {
        info!("📞 Demo: Advanced Routing");

        // Show dial plans
        self.demo_dial_plans().await?;
        
        // Show routing rules
        self.demo_routing_rules().await?;
        
        // Show failover mechanisms
        self.demo_failover_mechanisms().await?;
        
        // Show load balancing
        self.demo_load_balancing().await?;

        Ok(())
    }

    /// Demonstrate dial plans
    async fn demo_dial_plans(&self) -> Result<(), VoipBuilderError> {
        info!("📋 Demo: Dial Plans");

        for (name, dial_plan) in &self.routing_engine.dial_plans {
            info!("📞 Dial Plan: {}", name);
            info!("   Patterns: {} configured", dial_plan.patterns.len());
            info!("   Transformations: {} rules", dial_plan.transformations.len());
            info!("   Conditions: {} conditions", dial_plan.conditions.len());

            // Show sample patterns
            for (i, pattern) in dial_plan.patterns.iter().take(3).enumerate() {
                info!("   Pattern {}: {} -> {:?}", i + 1, pattern.pattern, pattern.action);
            }
        }

        // Simulate dial plan execution
        info!("🎯 Dial plan execution simulation:");
        let test_numbers = vec![
            "1001",      // Internal extension
            "555-1234",  // Local number
            "911",       // Emergency
            "18005551234", // Toll-free
            "01144207123456", // International
        ];

        for number in test_numbers {
            let route = self.simulate_routing(number).await;
            info!("   {} -> {}", number, route);
            sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }

    /// Simulate routing for a number
    async fn simulate_routing(&self, number: &str) -> String {
        match number {
            n if n.starts_with("1") && n.len() == 4 => "Internal Extension".to_string(),
            n if n.starts_with("555") => "Local PSTN Gateway".to_string(),
            "911" => "Emergency Services (Priority Route)".to_string(),
            n if n.starts_with("1800") => "Toll-Free Gateway".to_string(),
            n if n.starts_with("011") => "International Gateway".to_string(),
            _ => "Default Route".to_string(),
        }
    }

    /// Demonstrate routing rules
    async fn demo_routing_rules(&self) -> Result<(), VoipBuilderError> {
        info!("📏 Demo: Routing Rules");

        info!("🔧 Active routing rules:");
        for (i, rule) in self.routing_engine.routing_rules.iter().enumerate() {
            info!("   Rule {}: {}", i + 1, rule.name);
            info!("     Pattern: {}", rule.pattern);
            info!("     Destination: {:?}", rule.destination);
            info!("     Priority: {}", rule.priority);
            info!("     Conditions: {} configured", rule.conditions.len());
        }

        info!("⏰ Time-based routing example:");
        info!("   Business Hours (8 AM - 6 PM):");
        info!("     • Main line -> Reception desk");
        info!("     • Department lines -> Hunt groups");
        info!("     • Direct numbers -> User extensions");
        
        info!("   After Hours (6 PM - 8 AM):");
        info!("     • Main line -> Auto attendant");
        info!("     • Department lines -> Voicemail");
        info!("     • Emergency numbers -> On-call staff");

        info!("👥 User group routing:");
        info!("   VIP customers -> Priority queue");
        info!("   Enterprise accounts -> Dedicated agents");
        info!("   General customers -> Round-robin distribution");

        Ok(())
    }

    /// Demonstrate failover mechanisms
    async fn demo_failover_mechanisms(&self) -> Result<(), VoipBuilderError> {
        info!("🔄 Demo: Failover Mechanisms");

        info!("🛡️  Failover rules configured:");
        for (i, rule) in self.routing_engine.failover_rules.iter().enumerate() {
            info!("   Rule {}: Primary -> {}", i + 1, rule.primary_destination);
            info!("     Backup destinations: {} configured", rule.backup_destinations.len());
            info!("     Conditions: {:?}", rule.failover_conditions);
            info!("     Retry interval: {:?}", rule.retry_interval);
        }

        // Simulate failover scenario
        info!("🚨 Failover scenario simulation:");
        info!("   Scenario: Primary gateway failure");
        info!("   Primary: Gateway-A (unavailable)");
        info!("   Detecting failure...");
        sleep(Duration::from_millis(200)).await;
        
        info!("   ✅ Failure detected in 150ms");
        info!("   Attempting backup: Gateway-B");
        sleep(Duration::from_millis(100)).await;
        
        info!("   ✅ Failover successful to Gateway-B");
        info!("   Call routing restored");
        info!("   Primary gateway monitoring continues");

        info!("📊 Failover statistics:");
        info!("   Average detection time: 150ms");
        info!("   Average failover time: 250ms");
        info!("   Success rate: 99.8%");
        info!("   False positive rate: 0.1%");

        Ok(())
    }

    /// Demonstrate load balancing
    async fn demo_load_balancing(&self) -> Result<(), VoipBuilderError> {
        info!("⚖️  Demo: Load Balancing");

        let lb_config = &self.routing_engine.load_balancing;
        info!("🔧 Load balancing configuration:");
        info!("   Algorithm: {:?}", lb_config.algorithm);
        info!("   Health check interval: {:?}", lb_config.health_check_interval);
        info!("   Weight distribution: {} servers", lb_config.weight_distribution.len());

        info!("📊 Server distribution:");
        for (server, weight) in &lb_config.weight_distribution {
            info!("   {}: weight {}", server, weight);
        }

        // Simulate load balancing
        info!("🎯 Load balancing simulation (10 calls):");
        let servers = vec!["Server-A", "Server-B", "Server-C"];
        let mut assignments = HashMap::new();

        for i in 1..=10 {
            let server = match lb_config.algorithm {
                LoadBalancingAlgorithm::RoundRobin => servers[(i - 1) % servers.len()],
                LoadBalancingAlgorithm::Random => servers[i % servers.len()], // Simplified
                _ => servers[0], // Simplified
            };
            
            *assignments.entry(server).or_insert(0) += 1;
            info!("   Call {}: -> {}", i, server);
            sleep(Duration::from_millis(50)).await;
        }

        info!("📈 Distribution results:");
        for (server, count) in assignments {
            info!("   {}: {} calls ({:.1}%)", server, count, count as f64 * 10.0);
        }

        Ok(())
    }

    /// Demonstrate media processing
    async fn demo_media_processing(&self) -> Result<(), VoipBuilderError> {
        info!("🎵 Demo: Media Processing");

        // Show processing pipeline
        self.demo_processing_pipeline().await?;
        
        // Show codec management
        self.demo_codec_management().await?;
        
        // Show quality control
        self.demo_quality_control().await?;
        
        // Show recording features
        self.demo_recording_features().await?;

        Ok(())
    }

    /// Demonstrate processing pipeline
    async fn demo_processing_pipeline(&self) -> Result<(), VoipBuilderError> {
        info!("🔄 Demo: Processing Pipeline");

        info!("🎛️  Media processors:");
        for (i, processor) in self.media_processing.processors.iter().enumerate() {
            let status = if processor.enabled { "Enabled" } else { "Disabled" };
            info!("   {}: {} ({}) - {}", i + 1, processor.name, processor.processor_type.format(), status);
        }

        // Simulate media processing
        info!("🎵 Media processing simulation:");
        info!("   📥 Incoming audio stream (G.711 μ-law)");
        sleep(Duration::from_millis(100)).await;
        
        for processor in &self.media_processing.processors {
            if processor.enabled {
                info!("   🔧 Processing with: {}", processor.name);
                sleep(Duration::from_millis(50)).await;
            }
        }
        
        info!("   📤 Outgoing audio stream (Opus)");
        info!("   ✅ Processing completed in 45ms");

        info!("📊 Processing statistics:");
        info!("   Latency: 45ms (target: <50ms)");
        info!("   CPU usage: 12% (per stream)");
        info!("   Quality improvement: +0.8 MOS");
        info!("   Noise reduction: -18dB");

        Ok(())
    }

    /// Demonstrate codec management
    async fn demo_codec_management(&self) -> Result<(), VoipBuilderError> {
        info!("🎼 Demo: Codec Management");

        info!("🔊 Available codecs:");
        for (i, codec) in self.media_processing.codecs.iter().enumerate() {
            info!("   {}: {} (Priority: {}, Bandwidth: {} kbps)", 
                  i + 1, codec.name, codec.priority, codec.bandwidth_usage);
        }

        // Simulate codec negotiation
        info!("🤝 Codec negotiation simulation:");
        info!("   Endpoint A offers: Opus, G.722, G.711");
        info!("   Endpoint B offers: G.722, G.711, GSM");
        sleep(Duration::from_millis(100)).await;
        
        info!("   🔍 Finding common codecs...");
        info!("   ✅ Selected: G.722 (highest common priority)");
        info!("   Bandwidth allocated: 64 kbps");
        info!("   Quality target: 4.0 MOS");

        info!("🔄 Dynamic codec switching:");
        info!("   Network conditions: Degraded");
        info!("   Current codec: G.722 (64 kbps)");
        info!("   Switching to: G.711 (56 kbps)");
        info!("   ✅ Switch completed in 200ms");

        Ok(())
    }

    /// Demonstrate quality control
    async fn demo_quality_control(&self) -> Result<(), VoipBuilderError> {
        info!("📊 Demo: Quality Control");

        let quality = &self.media_processing.quality_settings;
        info!("🎯 Quality targets:");
        info!("   Target MOS: {}", quality.target_mos);
        info!("   Max packet loss: {}%", quality.max_packet_loss);
        info!("   Max jitter: {:?}", quality.max_jitter);
        info!("   Adaptive quality: {}", quality.adaptive_quality);

        // Simulate quality monitoring
        info!("📈 Quality monitoring simulation:");
        let samples = vec![
            (4.2, 0.1, 15),  // MOS, packet_loss%, jitter_ms
            (3.8, 0.3, 25),
            (3.5, 0.8, 35),
            (4.1, 0.2, 18),
        ];

        for (i, (mos, loss, jitter)) in samples.iter().enumerate() {
            info!("   Sample {}: MOS={}, Loss={}%, Jitter={}ms", i + 1, mos, loss, jitter);
            
            if *mos < quality.target_mos {
                info!("     ⚠️  Quality below target - adjusting parameters");
            } else {
                info!("     ✅ Quality within acceptable range");
            }
            sleep(Duration::from_millis(100)).await;
        }

        info!("🔧 Quality improvement actions:");
        info!("   • Jitter buffer adjustment");
        info!("   • Error correction enhancement");
        info!("   • Codec parameter tuning");
        info!("   • Network path optimization");

        Ok(())
    }

    /// Demonstrate recording features
    async fn demo_recording_features(&self) -> Result<(), VoipBuilderError> {
        info!("🎙️  Demo: Recording Features");

        let recording = &self.media_processing.recording_config;
        if recording.enabled {
            info!("📹 Recording configuration:");
            info!("   Format: {:?}", recording.format);
            info!("   Compression: {:?}", recording.compression);
            info!("   Storage: {}", recording.storage_location);
            info!("   Retention: {:?}", recording.retention_policy);

            // Simulate recording session
            info!("🎬 Recording session simulation:");
            info!("   📞 Call initiated: alice@company.com -> bob@company.com");
            info!("   🔴 Recording started (with consent notification)");
            sleep(Duration::from_millis(200)).await;
            
            info!("   🎵 Audio stream captured:");
            info!("     • Channels: 2 (stereo)");
            info!("     • Sample rate: 48 kHz");
            info!("     • Bit depth: 16-bit");
            info!("     • Format: WAV");
            
            sleep(Duration::from_millis(300)).await;
            info!("   ⏹️  Recording stopped");
            info!("   💾 File saved: call_20240115_143022.wav");
            info!("   🔐 File encrypted and indexed");
            info!("   📋 Metadata logged for compliance");

            info!("📊 Recording statistics:");
            info!("   File size: 2.4 MB (3 minute call)");
            info!("   Compression ratio: 65%");
            info!("   Storage used: 156 GB (total)");
            info!("   Retention queue: 1,247 files");
        }

        Ok(())
    }

    /// Demonstrate analytics and monitoring
    async fn demo_analytics_monitoring(&self) -> Result<(), VoipBuilderError> {
        info!("📊 Demo: Analytics and Monitoring");

        // Show metrics collection
        self.demo_metrics_collection().await?;
        
        // Show reporting
        self.demo_reporting().await?;
        
        // Show alerting
        self.demo_alerting().await?;
        
        // Show dashboards
        self.demo_dashboards().await?;

        Ok(())
    }

    /// Demonstrate metrics collection
    async fn demo_metrics_collection(&self) -> Result<(), VoipBuilderError> {
        info!("📈 Demo: Metrics Collection");

        info!("📊 Active metrics collectors:");
        for (i, collector) in self.analytics_engine.metrics_collectors.iter().enumerate() {
            info!("   {}: {} ({:?})", i + 1, collector.name, collector.metric_type);
            info!("     Interval: {:?}", collector.collection_interval);
            info!("     Retention: {:?}", collector.retention_period);
        }

        // Simulate metrics collection
        info!("🔄 Metrics collection simulation:");
        let metrics = vec![
            ("call_volume", 247.0),
            ("avg_call_duration", 245.5),
            ("call_success_rate", 99.2),
            ("avg_mos_score", 4.1),
            ("system_cpu_usage", 34.2),
            ("memory_usage_percent", 67.8),
        ];

        for (metric, value) in metrics {
            info!("   📊 {}: {}", metric, value);
            sleep(Duration::from_millis(50)).await;
        }

        info!("💾 Metrics storage:");
        info!("   Database: InfluxDB");
        info!("   Data points/day: ~2.3M");
        info!("   Storage size: 156 GB");
        info!("   Query performance: <100ms (avg)");

        Ok(())
    }

    /// Demonstrate reporting
    async fn demo_reporting(&self) -> Result<(), VoipBuilderError> {
        info!("📋 Demo: Reporting");

        let reporting = &self.analytics_engine.reporting_config;
        if reporting.enabled {
            info!("📊 Report configuration:");
            info!("   Formats: {:?}", reporting.report_formats);
            info!("   Recipients: {} configured", reporting.recipients.len());
            info!("   Schedules: {} defined", reporting.delivery_schedule.len());

            info!("📅 Scheduled reports:");
            for schedule in &reporting.delivery_schedule {
                info!("   {:?} {} report at {}", 
                      schedule.frequency, schedule.report_type, schedule.time);
            }

            // Simulate report generation
            info!("📄 Report generation simulation:");
            info!("   Report type: Weekly Call Summary");
            info!("   Period: Jan 8-14, 2024");
            info!("   📊 Collecting data...");
            sleep(Duration::from_millis(200)).await;
            
            info!("   📈 Generating charts...");
            sleep(Duration::from_millis(150)).await;
            
            info!("   📝 Creating document...");
            sleep(Duration::from_millis(100)).await;
            
            info!("   ✅ Report completed: weekly_report_20240115.pdf");
            info!("   📧 Sent to 5 recipients");
        }

        Ok(())
    }

    /// Demonstrate alerting
    async fn demo_alerting(&self) -> Result<(), VoipBuilderError> {
        info!("🚨 Demo: Alerting");

        info!("⚡ Active alerting rules:");
        for (i, rule) in self.analytics_engine.alerting_rules.iter().enumerate() {
            let status = if rule.enabled { "Enabled" } else { "Disabled" };
            info!("   {}: {} ({})", i + 1, rule.name, status);
            info!("     Condition: {:?} {}", rule.condition, rule.threshold);
            info!("     Action: {:?}", rule.action);
        }

        // Simulate alert scenario
        info!("🚨 Alert scenario simulation:");
        info!("   Monitoring: Call success rate");
        info!("   Current value: 95.2%");
        info!("   Threshold: >98%");
        info!("   Status: ⚠️  Below threshold");
        
        sleep(Duration::from_millis(200)).await;
        info!("   🔔 Alert triggered: Low call success rate");
        info!("   📧 Email sent to: ops-team@company.com");
        info!("   📱 SMS sent to: +1-555-ON-CALL");
        info!("   🌐 Webhook called: https://company.com/alerts");

        info!("🔍 Alert investigation:");
        info!("   Root cause analysis initiated");
        info!("   Checking gateway status...");
        sleep(Duration::from_millis(100)).await;
        info!("   ✅ Gateway-A: Healthy");
        info!("   ⚠️  Gateway-B: High error rate");
        info!("   🔧 Auto-remediation: Traffic redirected");

        Ok(())
    }

    /// Demonstrate dashboards
    async fn demo_dashboards(&self) -> Result<(), VoipBuilderError> {
        info!("📺 Demo: Dashboards");

        info!("🖥️  Available dashboards:");
        for (i, dashboard) in self.analytics_engine.dashboards.iter().enumerate() {
            info!("   {}: {}", i + 1, dashboard.name);
            info!("     Widgets: {} configured", dashboard.widgets.len());
            info!("     Refresh: {:?}", dashboard.refresh_interval);
            info!("     Permissions: {} users", dashboard.permissions.len());
        }

        // Show dashboard widgets
        info!("🔧 Dashboard widgets:");
        if let Some(dashboard) = self.analytics_engine.dashboards.first() {
            for widget in &dashboard.widgets {
                info!("   📊 {}: {:?}", widget.title, widget.widget_type);
                info!("     Data source: {}", widget.data_source);
            }
        }

        // Simulate dashboard view
        info!("👀 Real-time dashboard view:");
        info!("   📞 Active Calls: 247 (↑ +12 from 1h ago)");
        info!("   📈 Call Volume: 1,834 today (95% of target)");
        info!("   🎯 Average MOS: 4.2 (Excellent)");
        info!("   ⚡ System Health: 98.5% (Green)");
        info!("   🔄 Uptime: 99.97% (30 days)");

        Ok(())
    }

    /// Demonstrate custom integrations
    async fn demo_custom_integrations(&self) -> Result<(), VoipBuilderError> {
        info!("🔗 Demo: Custom Integrations");

        // Show integration types
        self.demo_integration_types().await?;
        
        // Show API integrations
        self.demo_api_integrations().await?;
        
        // Show webhook integrations
        self.demo_webhook_integrations().await?;

        Ok(())
    }

    /// Demonstrate integration types
    async fn demo_integration_types(&self) -> Result<(), VoipBuilderError> {
        info!("🔌 Demo: Integration Types");

        info!("📋 Available integration types:");
        info!("   🏢 CRM Systems:");
        info!("     • Salesforce");
        info!("     • HubSpot");
        info!("     • Microsoft Dynamics");
        info!("     • Custom CRM APIs");

        info!("   💼 Business Applications:");
        info!("     • Microsoft Teams");
        info!("     • Slack");
        info!("     • Zoom");
        info!("     • Google Workspace");

        info!("   📊 Analytics Platforms:");
        info!("     • Google Analytics");
        info!("     • Adobe Analytics");
        info!("     • Custom dashboards");

        info!("   🔧 IT Service Management:");
        info!("     • ServiceNow");
        info!("     • Jira Service Desk");
        info!("     • PagerDuty");

        Ok(())
    }

    /// Demonstrate API integrations
    async fn demo_api_integrations(&self) -> Result<(), VoipBuilderError> {
        info!("🌐 Demo: API Integrations");

        info!("📡 REST API endpoints:");
        info!("   📞 Call Management:");
        info!("     POST /api/v1/calls - Initiate call");
        info!("     GET  /api/v1/calls/{id} - Call details");
        info!("     PUT  /api/v1/calls/{id}/hold - Hold/unhold");
        info!("     DELETE /api/v1/calls/{id} - End call");

        info!("   👥 User Management:");
        info!("     GET  /api/v1/users - List users");
        info!("     POST /api/v1/users - Create user");
        info!("     PUT  /api/v1/users/{id} - Update user");
        info!("     DELETE /api/v1/users/{id} - Delete user");

        info!("   📊 Analytics:");
        info!("     GET  /api/v1/metrics - System metrics");
        info!("     GET  /api/v1/reports - Available reports");
        info!("     POST /api/v1/reports/generate - Generate report");

        // Simulate API call
        info!("📞 API integration simulation:");
        info!("   Request: POST /api/v1/calls");
        info!("   Payload: {\"from\": \"1001\", \"to\": \"555-1234\"}");
        sleep(Duration::from_millis(100)).await;
        info!("   Response: {\"call_id\": \"abc123\", \"status\": \"ringing\"}");
        info!("   ✅ Call initiated via API");

        Ok(())
    }

    /// Demonstrate webhook integrations
    async fn demo_webhook_integrations(&self) -> Result<(), VoipBuilderError> {
        info!("🪝 Demo: Webhook Integrations");

        info!("📨 Webhook endpoints configured:");
        info!("   📞 Call Events:");
        info!("     https://crm.company.com/webhooks/call-started");
        info!("     https://crm.company.com/webhooks/call-ended");
        info!("     https://analytics.company.com/webhooks/call-metrics");

        info!("   🚨 Alert Events:");
        info!("     https://monitoring.company.com/webhooks/alerts");
        info!("     https://slack.com/webhooks/incidents");

        // Simulate webhook delivery
        info!("📡 Webhook delivery simulation:");
        info!("   Event: Call completed");
        info!("   Webhook: https://crm.company.com/webhooks/call-ended");
        info!("   Payload: {\"call_id\": \"abc123\", \"duration\": 180, \"quality\": 4.2}");
        sleep(Duration::from_millis(150)).await;
        info!("   ✅ Webhook delivered (200 OK)");
        info!("   CRM updated with call record");

        info!("🔄 Webhook reliability:");
        info!("   Retry policy: Exponential backoff (3 attempts)");
        info!("   Timeout: 5 seconds");
        info!("   Success rate: 99.8%");
        info!("   Dead letter queue: For failed deliveries");

        Ok(())
    }

    /// Demonstrate scalability
    async fn demo_scalability(&self) -> Result<(), VoipBuilderError> {
        info!("📈 Demo: Scalability and Performance");

        // Show horizontal scaling
        self.demo_horizontal_scaling().await?;
        
        // Show performance optimization
        self.demo_performance_optimization().await?;
        
        // Show capacity planning
        self.demo_capacity_planning().await?;

        Ok(())
    }

    /// Demonstrate horizontal scaling
    async fn demo_horizontal_scaling(&self) -> Result<(), VoipBuilderError> {
        info!("🔄 Demo: Horizontal Scaling");

        info!("🏗️  Scaling architecture:");
        info!("   Load Balancer: HAProxy");
        info!("   Application Nodes: 5 active");
        info!("   Database: PostgreSQL cluster (3 nodes)");
        info!("   Cache: Redis cluster (3 nodes)");
        info!("   Message Queue: RabbitMQ cluster");

        // Simulate scaling event
        info!("📈 Auto-scaling simulation:");
        info!("   Current load: 85% CPU average");
        info!("   Threshold: 80% for 5 minutes");
        info!("   🚨 Scaling trigger activated");
        
        sleep(Duration::from_millis(200)).await;
        info!("   🚀 Launching new application node...");
        sleep(Duration::from_millis(300)).await;
        info!("   ✅ Node-6 online and healthy");
        info!("   🔄 Load balancer updated");
        info!("   📊 New load average: 68% CPU");

        info!("📊 Scaling metrics:");
        info!("   Scale-up time: 45 seconds");
        info!("   Zero downtime maintained");
        info!("   Cost increase: +16.7%");
        info!("   Performance improvement: +20%");

        Ok(())
    }

    /// Demonstrate performance optimization
    async fn demo_performance_optimization(&self) -> Result<(), VoipBuilderError> {
        info!("⚡ Demo: Performance Optimization");

        info!("🔧 Optimization techniques:");
        info!("   📞 Call Processing:");
        info!("     • Connection pooling");
        info!("     • Asynchronous processing");
        info!("     • Media path optimization");
        info!("     • Codec selection optimization");

        info!("   💾 Database Optimization:");
        info!("     • Query optimization");
        info!("     • Index tuning");
        info!("     • Read replicas");
        info!("     • Connection pooling");

        info!("   🗄️  Caching Strategy:");
        info!("     • User session cache");
        info!("     • Configuration cache");
        info!("     • Metrics cache");
        info!("     • DNS cache");

        info!("📊 Performance metrics:");
        info!("   Call setup time: 150ms (target: <200ms)");
        info!("   API response time: 45ms (target: <100ms)");
        info!("   Database query time: 12ms (target: <50ms)");
        info!("   Memory usage: 2.1GB (limit: 4GB)");
        info!("   Concurrent calls: 2,500 (limit: 5,000)");

        Ok(())
    }

    /// Demonstrate capacity planning
    async fn demo_capacity_planning(&self) -> Result<(), VoipBuilderError> {
        info!("📋 Demo: Capacity Planning");

        info!("📊 Current capacity utilization:");
        info!("   CPU: 34% average (70% peak)");
        info!("   Memory: 67% average (85% peak)");
        info!("   Network: 2.1 Gbps (5 Gbps capacity)");
        info!("   Storage: 156 GB used (500 GB total)");
        info!("   Concurrent calls: 247 (2,500 max)");

        info!("📈 Growth projections:");
        info!("   User growth: +25% annually");
        info!("   Call volume growth: +30% annually");
        info!("   Storage growth: +40% annually");

        info!("🔮 Capacity recommendations:");
        info!("   Next 6 months: Add 2 application nodes");
        info!("   Next 12 months: Upgrade network to 10 Gbps");
        info!("   Next 18 months: Scale database cluster");
        info!("   Next 24 months: Consider multi-region deployment");

        info!("💰 Cost projections:");
        info!("   Current monthly cost: $15,420");
        info!("   6-month projection: $18,950 (+23%)");
        info!("   12-month projection: $24,680 (+60%)");
        info!("   Cost per user/month: $6.20 (target: <$8.00)");

        Ok(())
    }

    /// Create custom components
    async fn create_custom_components() -> Result<HashMap<String, Box<dyn CustomComponent>>, VoipBuilderError> {
        let mut components = HashMap::new();
        
        // For demonstration, we'll create placeholder components
        // In a real implementation, these would be actual component implementations
        
        info!("🔧 Creating custom components...");
        info!("   ✅ Dial Plan Engine");
        info!("   ✅ Media Processor");
        info!("   ✅ Analytics Collector");
        info!("   ✅ Security Monitor");
        info!("   ✅ Integration Gateway");
        
        Ok(components)
    }

    /// Create routing engine
    fn create_routing_engine() -> AdvancedRoutingEngine {
        AdvancedRoutingEngine {
            dial_plans: Self::create_dial_plans(),
            routing_rules: Self::create_routing_rules(),
            failover_rules: Self::create_failover_rules(),
            load_balancing: Self::create_load_balancing_config(),
        }
    }

    /// Create dial plans
    fn create_dial_plans() -> HashMap<String, DialPlan> {
        let mut dial_plans = HashMap::new();
        
        dial_plans.insert("internal".to_string(), DialPlan {
            name: "Internal Extensions".to_string(),
            patterns: vec![
                DialPattern {
                    pattern: "1XXX".to_string(),
                    priority: 1,
                    action: DialAction::Route("internal_gateway".to_string()),
                },
            ],
            transformations: vec![],
            conditions: vec![],
        });

        dial_plans.insert("external".to_string(), DialPlan {
            name: "External Calls".to_string(),
            patterns: vec![
                DialPattern {
                    pattern: "NXXNXXXXXX".to_string(),
                    priority: 2,
                    action: DialAction::Route("pstn_gateway".to_string()),
                },
                DialPattern {
                    pattern: "911".to_string(),
                    priority: 1,
                    action: DialAction::Route("emergency_gateway".to_string()),
                },
            ],
            transformations: vec![
                NumberTransformation {
                    pattern: "^(\\d{10})$".to_string(),
                    replacement: "1$1".to_string(),
                    direction: TransformDirection::Outbound,
                },
            ],
            conditions: vec![
                RoutingCondition {
                    condition_type: ConditionType::Time,
                    value: "business_hours".to_string(),
                    operator: ConditionOperator::Equals,
                },
            ],
        });
        
        dial_plans
    }

    /// Create routing rules
    fn create_routing_rules() -> Vec<RoutingRule> {
        vec![
            RoutingRule {
                name: "Emergency Routes".to_string(),
                pattern: "911|933".to_string(),
                destination: RoutingDestination::Gateway("emergency".to_string()),
                conditions: vec![],
                priority: 1,
            },
            RoutingRule {
                name: "Internal Extensions".to_string(),
                pattern: "1[0-9]{3}".to_string(),
                destination: RoutingDestination::Extension("internal".to_string()),
                conditions: vec![],
                priority: 2,
            },
            RoutingRule {
                name: "Sales Hunt Group".to_string(),
                pattern: "3000".to_string(),
                destination: RoutingDestination::HuntGroup("sales".to_string()),
                conditions: vec![
                    RoutingCondition {
                        condition_type: ConditionType::Time,
                        value: "08:00-18:00".to_string(),
                        operator: ConditionOperator::Range,
                    },
                ],
                priority: 3,
            },
        ]
    }

    /// Create failover rules
    fn create_failover_rules() -> Vec<FailoverRule> {
        vec![
            FailoverRule {
                primary_destination: "gateway_primary".to_string(),
                backup_destinations: vec![
                    "gateway_backup1".to_string(),
                    "gateway_backup2".to_string(),
                ],
                failover_conditions: vec![
                    FailoverCondition::Unreachable,
                    FailoverCondition::Timeout,
                ],
                retry_interval: Duration::from_secs(30),
            },
        ]
    }

    /// Create load balancing configuration
    fn create_load_balancing_config() -> LoadBalancingConfig {
        let mut weight_distribution = HashMap::new();
        weight_distribution.insert("server_a".to_string(), 40);
        weight_distribution.insert("server_b".to_string(), 35);
        weight_distribution.insert("server_c".to_string(), 25);

        LoadBalancingConfig {
            algorithm: LoadBalancingAlgorithm::WeightedRoundRobin,
            weight_distribution,
            health_check_interval: Duration::from_secs(30),
        }
    }

    /// Create media processing pipeline
    fn create_media_pipeline() -> MediaProcessingPipeline {
        MediaProcessingPipeline {
            processors: vec![
                MediaProcessor {
                    name: "Echo Cancellation".to_string(),
                    processor_type: MediaProcessorType::EchoCancellation,
                    settings: HashMap::new(),
                    enabled: true,
                },
                MediaProcessor {
                    name: "Noise Reduction".to_string(),
                    processor_type: MediaProcessorType::NoiseReduction,
                    settings: HashMap::new(),
                    enabled: true,
                },
                MediaProcessor {
                    name: "Auto Gain Control".to_string(),
                    processor_type: MediaProcessorType::AutoGainControl,
                    settings: HashMap::new(),
                    enabled: true,
                },
            ],
            codecs: vec![
                CodecConfig {
                    name: "Opus".to_string(),
                    priority: 1,
                    settings: HashMap::new(),
                    bandwidth_usage: 32,
                },
                CodecConfig {
                    name: "G.722".to_string(),
                    priority: 2,
                    settings: HashMap::new(),
                    bandwidth_usage: 64,
                },
                CodecConfig {
                    name: "G.711".to_string(),
                    priority: 3,
                    settings: HashMap::new(),
                    bandwidth_usage: 64,
                },
            ],
            quality_settings: QualitySettings {
                target_mos: 4.0,
                max_packet_loss: 1.0,
                max_jitter: Duration::from_millis(30),
                adaptive_quality: true,
            },
            recording_config: RecordingConfig {
                enabled: true,
                format: RecordingFormat::WAV,
                compression: CompressionLevel::Medium,
                storage_location: "/var/recordings".to_string(),
                retention_policy: Duration::from_secs(365 * 24 * 3600), // 1 year
            },
        }
    }

    /// Create analytics engine
    fn create_analytics_engine() -> AnalyticsEngine {
        AnalyticsEngine {
            metrics_collectors: vec![
                MetricsCollector {
                    name: "Call Volume Metrics".to_string(),
                    metric_type: MetricType::CallVolume,
                    collection_interval: Duration::from_secs(60),
                    retention_period: Duration::from_secs(30 * 24 * 3600), // 30 days
                },
                MetricsCollector {
                    name: "Quality Metrics".to_string(),
                    metric_type: MetricType::CallQuality,
                    collection_interval: Duration::from_secs(30),
                    retention_period: Duration::from_secs(7 * 24 * 3600), // 7 days
                },
                MetricsCollector {
                    name: "System Performance".to_string(),
                    metric_type: MetricType::SystemPerformance,
                    collection_interval: Duration::from_secs(30),
                    retention_period: Duration::from_secs(24 * 3600), // 1 day
                },
            ],
            reporting_config: ReportingConfig {
                enabled: true,
                report_formats: vec![ReportFormat::PDF, ReportFormat::Excel],
                delivery_schedule: vec![
                    ReportSchedule {
                        frequency: ReportFrequency::Daily,
                        time: "09:00".to_string(),
                        report_type: "System Summary".to_string(),
                    },
                    ReportSchedule {
                        frequency: ReportFrequency::Weekly,
                        time: "Monday 08:00".to_string(),
                        report_type: "Weekly Analysis".to_string(),
                    },
                ],
                recipients: vec![
                    "admin@company.com".to_string(),
                    "ops@company.com".to_string(),
                ],
            },
            alerting_rules: vec![
                AlertingRule {
                    name: "High CPU Usage".to_string(),
                    condition: AlertCondition::Greater,
                    threshold: 80.0,
                    action: AlertAction::Email(vec!["ops@company.com".to_string()]),
                    enabled: true,
                },
                AlertingRule {
                    name: "Low Call Success Rate".to_string(),
                    condition: AlertCondition::Less,
                    threshold: 98.0,
                    action: AlertAction::SMS(vec!["+1-555-ON-CALL".to_string()]),
                    enabled: true,
                },
            ],
            dashboards: vec![
                Dashboard {
                    name: "Operations Dashboard".to_string(),
                    widgets: vec![
                        DashboardWidget {
                            widget_type: WidgetType::LineChart,
                            title: "Call Volume".to_string(),
                            data_source: "call_metrics".to_string(),
                            settings: HashMap::new(),
                        },
                        DashboardWidget {
                            widget_type: WidgetType::Gauge,
                            title: "System Health".to_string(),
                            data_source: "system_metrics".to_string(),
                            settings: HashMap::new(),
                        },
                    ],
                    refresh_interval: Duration::from_secs(30),
                    permissions: vec!["operators".to_string(), "administrators".to_string()],
                },
            ],
        }
    }

    /// Create monitoring system
    fn create_monitoring_system() -> MonitoringSystem {
        MonitoringSystem {
            health_checks: vec![
                HealthCheck {
                    name: "SIP Stack Health".to_string(),
                    target: "sip-stack".to_string(),
                    check_type: HealthCheckType::SIP,
                    interval: Duration::from_secs(30),
                    timeout: Duration::from_secs(5),
                    retry_count: 3,
                },
                HealthCheck {
                    name: "Database Health".to_string(),
                    target: "postgresql://localhost:5432".to_string(),
                    check_type: HealthCheckType::Database,
                    interval: Duration::from_secs(60),
                    timeout: Duration::from_secs(10),
                    retry_count: 2,
                },
            ],
            performance_monitors: vec![
                PerformanceMonitor {
                    name: "CPU Monitor".to_string(),
                    metric_name: "cpu_usage_percent".to_string(),
                    collection_method: CollectionMethod::SystemMetrics,
                    thresholds: PerformanceThresholds {
                        warning: 70.0,
                        critical: 90.0,
                        unit: "percent".to_string(),
                    },
                },
            ],
            log_aggregation: LogAggregationConfig {
                enabled: true,
                log_level: "info".to_string(),
                output_format: LogFormat::JSON,
                destinations: vec![
                    LogDestination::File("/var/log/rvoip.log".to_string()),
                    LogDestination::Elasticsearch("http://elasticsearch:9200".to_string()),
                ],
            },
            tracing_config: TracingConfig {
                enabled: true,
                sampling_rate: 0.1,
                trace_storage: "jaeger".to_string(),
                correlation_enabled: true,
            },
        }
    }
}

// Helper trait implementations
impl MediaProcessorType {
    fn format(&self) -> &str {
        match self {
            MediaProcessorType::EchoCancellation => "Echo Cancellation",
            MediaProcessorType::NoiseReduction => "Noise Reduction",
            MediaProcessorType::AutoGainControl => "Auto Gain Control",
            MediaProcessorType::VoiceActivityDetection => "Voice Activity Detection",
            MediaProcessorType::DTMF => "DTMF Detection",
            MediaProcessorType::Transcoding => "Transcoding",
            MediaProcessorType::Recording => "Recording",
            MediaProcessorType::Mixing => "Audio Mixing",
        }
    }
} 