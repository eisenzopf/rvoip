# RVOIP Library Design Architecture V2 (Based on Actual Code)

## Existing Libraries and Their Real Dependencies

### Foundation Libraries (No Internal Dependencies)
- **infra-common**: Event system, configuration, infrastructure utilities
- **sip-core**: SIP message parsing, headers, URIs
- **codec-core**: Audio codecs (G.711, etc.)
- **users-core**: User management and authentication
- **auth-core**: OAuth2 and token-based authentication

### Libraries with Dependencies

#### Transport/Protocol Layer
```
rtp-core
└── infra-common

sip-transport
├── sip-core
└── infra-common
```

#### Media Processing Layer
```
media-core
├── rtp-core
│   └── infra-common
├── codec-core
└── infra-common
```

#### Dialog Management Layer
```
dialog-core
├── sip-core
├── sip-transport
│   ├── sip-core
│   └── infra-common
└── infra-common
```

#### Service Layer
```
registrar-core
├── sip-core
└── infra-common
```

#### Session Management Layer
```
session-core-v2
├── dialog-core
│   ├── sip-core
│   ├── sip-transport
│   │   ├── sip-core
│   │   └── infra-common
│   └── infra-common
├── media-core
│   ├── rtp-core
│   │   └── infra-common
│   ├── codec-core
│   └── infra-common
└── infra-common
```

## Complete Existing Library Dependency Matrix

| Library | infra-common | sip-core | codec-core | rtp-core | sip-transport | media-core | dialog-core | session-core-v2 | registrar-core | users-core | auth-core |
|---------|-------------|----------|------------|----------|---------------|------------|-------------|-----------------|----------------|------------|-----------|
| **infra-common** | - | - | - | - | - | - | - | - | - | - | - |
| **sip-core** | - | - | - | - | - | - | - | - | - | - | - |
| **codec-core** | - | - | - | - | - | - | - | - | - | - | - |
| **users-core** | - | - | - | - | - | - | - | - | - | - | - |
| **auth-core** | - | - | - | - | - | - | - | - | - | - | - |
| **rtp-core** | ✅ | - | - | - | - | - | - | - | - | - | - |
| **sip-transport** | ✅ | ✅ | - | - | - | - | - | - | - | - | - |
| **media-core** | ✅ | - | ✅ | ✅ | - | - | - | - | - | - | - |
| **dialog-core** | ✅ | ✅ | - | - | ✅ | - | - | - | - | - | - |
| **registrar-core** | ✅ | ✅ | - | - | - | - | - | - | - | - | - |
| **session-core-v2** | ✅ | - | - | - | - | ✅ | ✅ | - | - | - | - |

## Libraries That Need to Be Created

### 1. b2bua-core (NEW)
**Purpose**: Back-to-back user agent functionality for server applications
**Architecture Note**: Built directly on dialog-core, NOT on session-core-v2
**Expected Dependencies**:
```
b2bua-core
├── dialog-core
│   ├── sip-core
│   ├── sip-transport
│   │   ├── sip-core
│   │   └── infra-common
│   └── infra-common
└── infra-common

Note: b2bua-core does NOT use:
- session-core-v2 (that's for endpoints)
- media-core (delegates to media-server-core via API)
```

### 2. proxy-core (NEW)
**Purpose**: SIP proxy server functionality
**Expected Dependencies**:
```
proxy-core
├── dialog-core
│   ├── sip-core
│   ├── sip-transport
│   │   ├── sip-core
│   │   └── infra-common
│   └── infra-common
└── infra-common
```

### 3. media-server-core (NEW)
**Purpose**: Standalone media server (mixing, recording, IVR)
**Architecture Note**: Controlled via API by b2bua-core, handles all RTP processing
**Expected Dependencies**:
```
media-server-core
├── media-core
│   ├── rtp-core
│   │   └── infra-common
│   ├── codec-core
│   └── infra-common
└── infra-common

Note: May need direct socket management for RTP
Consider: Creating rtp-transport library if sip-transport is too SIP-specific
```

### 4. sbc-core (NEW)
**Purpose**: Session border controller
**Expected Dependencies**:
```
sbc-core
├── b2bua-core (optional, for B2BUA mode)
│   └── session-core-v2
│       └── ... (full tree)
├── proxy-core (optional, for proxy mode)
│   └── dialog-core
│       └── ... (full tree)
├── media-core (for RTP anchoring)
│   ├── rtp-core
│   │   └── infra-common
│   ├── codec-core
│   └── infra-common
├── sip-transport (for network edge)
│   ├── sip-core
│   └── infra-common
└── infra-common
```

## Server Implementation Using Libraries

### 1. Registrar Server
```rust
// Uses existing libraries only
registrar-core
sip-core
infra-common
users-core  // For authentication
```

### 2. Proxy Server
```rust
// Needs NEW library
proxy-core (NEW)
  └── dialog-core
  └── infra-common
```

### 3. B2BUA Call Center Server
```rust
// Needs NEW library
b2bua-core (NEW)
  └── dialog-core  // Direct usage, NOT via session-core-v2
  └── infra-common
  └── media-server-client (controls remote media servers via API)
```

### 4. Media Server
```rust
// Needs NEW library
media-server-core (NEW)
  └── media-core
  └── infra-common
  └── Direct UDP socket management for RTP
```

### 5. SBC Server
```rust
// Needs NEW library
sbc-core (NEW)
  └── b2bua-core (optional)
  └── proxy-core (optional)
  └── media-core
  └── sip-transport
```

## Library Usage Patterns

### For SIP Endpoints (Clients, Softphones)
```
Application (Softphone)
    └── session-core-v2
        ├── dialog-core (SIP signaling)
        └── media-core (local RTP processing)
```

### For B2BUA Servers (IVR, Queue, Conference)
```
B2BUA Application
    └── b2bua-core
        ├── dialog-core (SIP signaling)
        └── media-server-client → API → media-server-core (remote RTP)
```

### Key Difference
- **Endpoints**: Process media locally using media-core
- **B2BUA**: Delegates media to remote servers via API

## Key Architectural Insights

1. **infra-common** is the true foundation - provides event system, config, utilities
2. **sip-transport** handles SIP network I/O
3. **media-core** is pure media processing (no network I/O)
4. **session-core-v2** is for SIP endpoints ONLY (clients, phones)
5. **b2bua-core** is for B2BUA servers (built on dialog-core, NOT session-core-v2)
6. **Media servers** are separate processes controlled via API
7. **users-core** and **auth-core** are standalone (no RVOIP dependencies)

## Missing Critical Libraries

| Library | Purpose | Priority | Dependencies |
|---------|---------|----------|--------------|
| **proxy-core** | SIP proxy functionality | HIGH | dialog-core, infra-common |
| **b2bua-core** | Back-to-back user agent | HIGH | dialog-core, infra-common (NOT session-core-v2) |
| **media-server-core** | Media server operations | HIGH | media-core, infra-common |
| **sbc-core** | Session border controller | MEDIUM | b2bua-core, proxy-core, media-core |

## Corrected Architecture Understanding

- **session-core-v2** is ONLY for SIP endpoints (clients, softphones)
- **b2bua-core** is built directly on dialog-core for B2BUA servers
- **media-server-core** runs as separate process, controlled via API
- **infra-common** provides shared event bus used by all libraries
- **media-core** is pure processing, no network I/O
- B2BUA servers delegate ALL media to media servers (no local RTP)

## Implementation Priority

1. **First**: Clean up `session-core-v2` - remove B2BUA features
2. **Second**: Create `b2bua-core` on dialog-core - needed for call center
3. **Third**: Create `media-server-core` - needed for all media operations
4. **Fourth**: Create `proxy-core` - needed for routing
5. **Fifth**: Create `sbc-core` - needed for production edge