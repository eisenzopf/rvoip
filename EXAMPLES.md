# RVOIP Examples Guide

This guide provides an overview of all examples available across the RVOIP implementation crates, demonstrating various VoIP use cases and architectural patterns.

## 📋 Example Overview

The RVOIP project includes comprehensive examples across three implementation crates:

- **`rvoip-simple`**: Developer-friendly APIs for basic VoIP applications
- **`rvoip-presets`**: Pre-configured templates for common deployment scenarios
- **`rvoip-builder`**: Flexible composition patterns for advanced custom systems

## 🚀 Getting Started

### Prerequisites

```bash
# Install Rust (latest stable)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone the repository
git clone https://github.com/your-org/rvoip.git
cd rvoip
```

### Running Examples

```bash
# Navigate to a specific crate
cd rvoip-simple

# Run an example
cargo run --example p2p_secure_call

# Run with logs
RUST_LOG=info cargo run --example desktop_softphone
```

## 📞 rvoip-simple Examples

**Location**: `rvoip-simple/examples/`

### 1. P2P Secure Calling (`p2p_secure_call.rs`)
**Purpose**: Demonstrates peer-to-peer secure calling using ZRTP encryption without requiring a central server.

**Key Features**:
- Direct peer-to-peer connections
- ZRTP encryption with SAS verification
- NAT traversal with ICE
- Real-time quality monitoring

**Use Cases**:
- Privacy-focused communications
- Decentralized calling systems
- Direct device-to-device calling

```bash
cargo run --example p2p_secure_call
```

### 2. Desktop Softphone (`desktop_softphone.rs`)
**Purpose**: Shows a comprehensive desktop softphone application with advanced features.

**Key Features**:
- Multi-line support
- Call transfer and conferencing
- Presence and messaging
- Quality monitoring and adaptation
- Enterprise directory integration

**Use Cases**:
- Business desktop applications
- Contact center agent interfaces
- Enterprise softphones

```bash
cargo run --example desktop_softphone
```

### 3. Mobile VoIP Client (`mobile_voip_client.rs`)
**Purpose**: Demonstrates mobile-optimized VoIP with battery and bandwidth considerations.

**Key Features**:
- Battery optimization
- Network adaptation (WiFi/cellular)
- Background call handling
- Push notifications
- Mobile-specific codecs

**Use Cases**:
- Mobile VoIP applications
- Cross-platform calling apps
- Battery-efficient communication

```bash
cargo run --example mobile_voip_client
```

### 4. WebRTC Browser Client (`webrtc_browser_client.rs`)
**Purpose**: Shows browser-based WebRTC implementation with JavaScript integration.

**Key Features**:
- Browser WebRTC APIs
- Real-time communication
- Screen sharing
- Media constraints
- Cross-browser compatibility

**Use Cases**:
- Web-based calling
- Browser plugins
- WebRTC gateways

```bash
cargo run --example webrtc_browser_client
```

### 5. Conference Bridge (`conference_bridge.rs`)
**Purpose**: Implements multi-party audio/video conferencing with advanced features.

**Key Features**:
- Multiple participants
- Audio mixing and video composition
- Screen sharing and presentation mode
- Recording and streaming
- Moderator controls

**Use Cases**:
- Video conferencing platforms
- Webinar systems
- Collaboration tools

```bash
cargo run --example conference_bridge
```

## 🏢 rvoip-presets Examples

**Location**: `rvoip-presets/examples/`

### 1. Enterprise PBX (`enterprise_pbx.rs`)
**Purpose**: Demonstrates a complete enterprise PBX system deployment and management.

**Key Features**:
- Multi-site deployment
- Advanced call routing
- Hunt groups and queues
- Reporting and analytics
- Integration with business systems

**Use Cases**:
- Corporate phone systems
- Multi-location businesses
- Call center deployments

```bash
cd rvoip-presets
cargo run --example enterprise_pbx
```

### 2. Healthcare Solution (`healthcare_solution.rs`)
**Purpose**: Shows HIPAA-compliant VoIP implementation for medical facilities.

**Key Features**:
- HIPAA compliance features
- Emergency protocols (Code Blue, Code Red)
- Medical staff management
- Patient privacy protection
- Telemedicine integration

**Use Cases**:
- Hospital communication systems
- Medical practice solutions
- Telemedicine platforms
- Emergency response systems

```bash
cargo run --example healthcare_solution
```

### 3. WebRTC Platform (`webrtc_platform.rs`)
**Purpose**: Demonstrates building a modern WebRTC-based communication platform.

**Key Features**:
- Browser-to-browser calling
- Signaling server architecture
- Media server scaling
- Recording and streaming
- Global CDN integration

**Use Cases**:
- Video conferencing platforms
- Online meeting solutions
- Live streaming platforms
- Browser-based communication

```bash
cargo run --example webrtc_platform
```

### 4. Contact Center (`contact_center.rs`)
**Purpose**: Shows advanced contact center features and customer service optimization.

**Key Features**:
- Automatic call distribution (ACD)
- Interactive voice response (IVR)
- Queue management
- Agent dashboards
- Customer journey tracking

**Use Cases**:
- Customer service centers
- Sales teams
- Support operations
- Help desk solutions

```bash
cargo run --example contact_center
```

### 5. Cloud Communications (`cloud_communications.rs`)
**Purpose**: Demonstrates cloud-native VoIP deployment with auto-scaling.

**Key Features**:
- Multi-region deployment
- Auto-scaling capabilities
- Load balancing
- Disaster recovery
- API-first architecture

**Use Cases**:
- SaaS communication platforms
- Global VoIP services
- Scalable communication solutions

```bash
cargo run --example cloud_communications
```

## 🏗️ rvoip-builder Examples

**Location**: `rvoip-builder/examples/`

### 1. Custom PBX Builder (`custom_pbx_builder.rs`)
**Purpose**: Shows how to build a custom PBX system using flexible composition patterns.

**Key Features**:
- Component-based architecture
- Advanced routing engine
- Media processing pipeline
- Analytics and monitoring
- Custom integrations

**Use Cases**:
- Custom PBX solutions
- Specialized communication systems
- Integration with existing infrastructure

```bash
cd rvoip-builder
cargo run --example custom_pbx_builder
```

### 2. Microservice Composition (`microservice_composition.rs`)
**Purpose**: Demonstrates building VoIP systems using microservice architecture.

**Key Features**:
- Microservice decomposition
- Service discovery
- Inter-service communication
- Distributed tracing
- Circuit breakers

**Use Cases**:
- Cloud-native architectures
- Scalable VoIP platforms
- DevOps-friendly deployments

```bash
cargo run --example microservice_composition
```

### 3. Multi-Protocol Gateway (`multi_protocol_gateway.rs`)
**Purpose**: Shows building gateways that support multiple VoIP protocols.

**Key Features**:
- SIP, H.323, WebRTC support
- Protocol translation
- Codec transcoding
- Unified routing
- Legacy system integration

**Use Cases**:
- Protocol interoperability
- Legacy system modernization
- Unified communication platforms

```bash
cargo run --example multi_protocol_gateway
```

## 🔧 Development Guide

### Example Structure

Each example follows a consistent structure:

```
examples/
├── example_name.rs          # Main example code
├── README.md               # Example-specific documentation
└── config/                 # Configuration files (if needed)
    ├── development.yml
    ├── production.yml
    └── test.yml
```

### Adding New Examples

1. **Choose the appropriate crate** based on complexity level
2. **Follow naming conventions**: Use descriptive snake_case names
3. **Include comprehensive documentation** with use cases and features
4. **Add logging** using the `tracing` crate for observability
5. **Provide realistic scenarios** with practical demonstrations

### Example Template

```rust
//! Example Name
//!
//! Brief description of what this example demonstrates.
//!
//! ## Features
//! - Feature 1
//! - Feature 2
//! 
//! ## Use Cases
//! - Use case 1
//! - Use case 2

use rvoip_simple::*;
use tracing::{info, warn, error};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Example");

    // Your example code here

    info!("✅ Example completed!");
    Ok(())
}
```

## 📊 Example Complexity Levels

### 🟢 Beginner (rvoip-simple)
- **Audience**: Developers new to VoIP
- **Focus**: Common use cases with minimal configuration
- **Learning curve**: Low
- **Customization**: Limited but sufficient for most needs

### 🟡 Intermediate (rvoip-presets)
- **Audience**: Developers deploying VoIP solutions
- **Focus**: Industry-specific configurations
- **Learning curve**: Medium
- **Customization**: High within preset boundaries

### 🔴 Advanced (rvoip-builder)
- **Audience**: System architects and VoIP experts
- **Focus**: Maximum flexibility and custom architectures
- **Learning curve**: High
- **Customization**: Complete control over all aspects

## 🔍 Feature Matrix

| Feature | Simple | Presets | Builder |
|---------|--------|---------|---------|
| Basic Calling | ✅ | ✅ | ✅ |
| Security (SRTP/ZRTP) | ✅ | ✅ | ✅ |
| Video Conferencing | ✅ | ✅ | ✅ |
| Screen Sharing | ✅ | ✅ | ✅ |
| Recording | ✅ | ✅ | ✅ |
| WebRTC Support | ✅ | ✅ | ✅ |
| Enterprise Features | ❌ | ✅ | ✅ |
| Industry Compliance | ❌ | ✅ | ✅ |
| Custom Architecture | ❌ | ❌ | ✅ |
| Microservices | ❌ | ❌ | ✅ |
| Advanced Analytics | ❌ | ✅ | ✅ |
| Multi-Protocol | ❌ | ❌ | ✅ |

## 🚦 Running Specific Scenarios

### Development Environment
```bash
# Quick testing and development
RUST_LOG=debug cargo run --example p2p_secure_call

# Development with file logging
RUST_LOG=info cargo run --example enterprise_pbx 2>&1 | tee example.log
```

### Production Simulation
```bash
# Production-like configuration
RUST_LOG=warn cargo run --release --example healthcare_solution

# Performance testing
cargo run --release --example webrtc_platform -- --users 1000
```

### Integration Testing
```bash
# Run all examples for integration testing
for example in $(ls examples/*.rs | sed 's/examples\///g' | sed 's/\.rs//g'); do
    echo "Running $example..."
    cargo run --example $example --quiet || echo "Failed: $example"
done
```

## 📚 Learning Path

### 1. Start with Simple Examples
- Begin with `p2p_secure_call.rs` to understand basic concepts
- Move to `desktop_softphone.rs` for application features
- Try `mobile_voip_client.rs` for mobile considerations

### 2. Explore Industry Solutions
- Study `healthcare_solution.rs` for compliance requirements
- Review `enterprise_pbx.rs` for business features
- Examine `webrtc_platform.rs` for modern web technologies

### 3. Advanced Architecture
- Dive into `custom_pbx_builder.rs` for flexible design
- Understand `microservice_composition.rs` for scalability
- Master `multi_protocol_gateway.rs` for interoperability

## 🤝 Contributing Examples

We welcome contributions of new examples! Please:

1. **Follow the existing patterns** and documentation style
2. **Test thoroughly** across different scenarios
3. **Include realistic data** and use cases
4. **Add appropriate logging** and error handling
5. **Update this guide** with your new example

### Submission Process

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/new-example`
3. Add your example following the template
4. Test with `cargo test` and `cargo run --example your_example`
5. Update documentation and this guide
6. Submit a pull request with detailed description

## 🔗 Additional Resources

- [RVOIP Core Documentation](../README.md)
- [Architecture Guide](../ARCHITECTURE.md)
- [API Reference](https://docs.rs/rvoip)
- [Community Forum](https://github.com/your-org/rvoip/discussions)
- [Issue Tracker](https://github.com/your-org/rvoip/issues)

## 🆘 Support

If you encounter issues with any examples:

1. **Check the logs** with `RUST_LOG=debug`
2. **Review the documentation** for each example
3. **Search existing issues** on GitHub
4. **Create a new issue** with:
   - Example name and crate
   - Operating system and Rust version
   - Complete error logs
   - Steps to reproduce

---

*Happy coding with RVOIP! 🎉* 