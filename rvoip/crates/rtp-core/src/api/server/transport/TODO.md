# Server Transport Module Refactoring Plan

## Overview

The `server_transport_impl.rs` file has grown to nearly 2300 lines, making it difficult to maintain and understand. This plan outlines how to refactor it into smaller, logical modules of no more than 200 lines each, while maintaining the same API surface.

## Proposed Directory Structure

```
src/api/server/transport/
├── mod.rs                           (existing file, with updated exports)
├── server_transport_impl.rs         (simplified core file, ~7 lines)
├── default.rs                       (implementation file with DefaultMediaTransportServer, ~700 lines)
├── core/
│   ├── mod.rs                       (exports core components)
│   ├── connection.rs                (client connection management, ~150 lines)
│   ├── frame.rs                     (frame sending/receiving, ~200 lines)
│   └── events.rs                    (event handling, ~100 lines)
├── media/
│   ├── mod.rs                       (exports media components)
│   ├── mix.rs                       (media mixing, ~150 lines)
│   ├── csrc.rs                      (CSRC management, ~150 lines)
│   └── extensions.rs                (header extensions, ~100 lines)
├── rtcp/
│   ├── mod.rs                       (exports RTCP components)
│   ├── reports.rs                   (RTCP reports, ~120 lines)
│   └── app_packets.rs               (RTCP application packets, ~120 lines)
├── security/
│   ├── mod.rs                       (exports security components)
│   └── server_security.rs           (security context handling, ~100 lines)
├── ssrc/
│   ├── mod.rs                       (exports SSRC components)
│   └── demux.rs                     (SSRC demultiplexing, ~100 lines)
└── stats/
    ├── mod.rs                       (exports stats components)
    ├── quality.rs                   (quality estimation, ~100 lines)
    └── metrics.rs                   (server metrics, ~100 lines)
```

## Module Responsibilities

1. **server_transport_impl.rs**: Re-exports the DefaultMediaTransportServer from the default module
   
2. **default.rs**: Contains the actual DefaultMediaTransportServer implementation, its struct definition and trait implementation that delegates to other modules

3. **core/**:
   - **connection.rs**: Client connection establishment, management, and disconnection
   - **frame.rs**: Basic frame sending/receiving/broadcasting logic
   - **events.rs**: Event subscription and callback management

4. **media/**:
   - **mix.rs**: Media mixing functionality for conferences
   - **csrc.rs**: CSRC (Contributing Source) management
   - **extensions.rs**: RTP header extensions

5. **rtcp/**:
   - **reports.rs**: RTCP sender/receiver reports
   - **app_packets.rs**: RTCP application-defined packets, BYE packets, XR packets

6. **security/**:
   - **server_security.rs**: Security context handling, DTLS, SRTP for server

7. **ssrc/**:
   - **demux.rs**: SSRC demultiplexing functionality

8. **stats/**:
   - **quality.rs**: Quality level estimation
   - **metrics.rs**: Server-specific metrics and statistics

## Progress

- [x] **Phase 1: Setup**
  - [x] Create the directory structure
  - [x] Create main mod.rs files for each subdirectory
  - [x] Create placeholders for all module files

- [x] **Phase 2: Core Implementation**
  - [x] Extract core structure and initialization to simplified server_transport_impl.rs
  - [x] Create connection.rs with client connection management functionality
  - [x] Create frame.rs with send/receive/broadcast frame logic
  - [x] Create events.rs with event handling logic
  - [x] Implement delegate methods in default.rs that call into module functions
  - [x] Fix compatibility issues between the module interfaces and their usage

- [x] **Phase 3: Media Implementation**
  - [x] Create mix.rs with media mixing logic
  - [x] Create csrc.rs with CSRC management
  - [x] Create extensions.rs with header extension handling
  - [x] Implement actual functionality in media modules
  - [x] Update default.rs to delegate to media module functions

- [x] **Phase 4: RTCP Implementation**
  - [x] Create reports.rs with RTCP report functionality
  - [x] Create app_packets.rs with application packet handling
  - [x] Implement actual functionality in RTCP modules
  - [x] Update default.rs to delegate to RTCP module functions

- [ ] **Phase 5: Security & SSRC Implementation**
  - [x] Create placeholder for server_security.rs with security context handling
  - [x] Create placeholder for demux.rs with SSRC demultiplexing
  - [x] Implement actual functionality in security and SSRC modules

- [ ] **Phase 6: Stats Implementation**
  - [ ] Create placeholder for quality.rs with quality estimation logic
  - [ ] Create placeholder for metrics.rs with server metrics
  - [ ] Implement actual functionality in stats modules

- [ ] **Phase 7: Integration & Testing**
  - [ ] Update server_transport_impl.rs to use all modules
  - [ ] Fix compilation errors between module interfaces and server_transport_impl.rs
  - [ ] Create default.rs with DefaultMediaTransportServer implementation
  - [ ] Update server_transport_impl.rs to just re-export from default.rs
  - [ ] Run tests to verify functionality
  - [ ] Fix any issues identified during testing

## Current Status

Phase 4 has been completed. The RTCP functionality has been implemented in their respective modules:

1. **RTCP reports** functionality in reports.rs - including sending sender/receiver reports and getting RTCP statistics
2. **RTCP application packets** functionality in app_packets.rs - including APP, BYE, and XR packets for VoIP metrics

Previously, we implemented media functionality in Phase 3, core functionality in Phase 2, and set up the directory structure in Phase 1.

## Implementation Strategy

1. Work on one module at a time, starting with core functionality
2. Extract code from the original server_transport_impl.rs to the appropriate module
3. Ensure all public APIs remain unchanged
4. Add comprehensive documentation to each module
5. Run tests after each module is completed to verify functionality

## Key Components to Extract

1. **ClientConnection struct**: Moved to core/connection.rs
2. **DefaultMediaTransportServer struct**: Core structure stays in default.rs
3. **handle_client method**: Moved to core/connection.rs
4. **CSRC management methods**: Moved to media/csrc.rs
5. **Header extension methods**: Moved to media/extensions.rs
6. **RTCP methods**: Moved to rtcp/reports.rs and rtcp/app_packets.rs
7. **Security initialization**: Moved to security/server_security.rs
8. **SSRC demultiplexing**: To be moved to ssrc/demux.rs
9. **Quality estimation**: To be moved to stats/quality.rs
10. **Frame broadcasting**: Moved to core/frame.rs

## Benefits

- Improved code organization and readability
- Easier maintenance and bug fixing
- Better separation of concerns
- Smaller, more focused files
- More manageable for future contributors
- Consistent structure with client transport implementation 