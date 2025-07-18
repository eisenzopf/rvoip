# Server Security Module Refactoring Plan

## Overview

The `server_security_impl.rs` file has grown to over 1100 lines, making it difficult to maintain and understand. This plan outlines how to refactor it into smaller, logical modules of no more than 200 lines each, while maintaining the same API surface.

## Proposed Directory Structure

```
src/api/server/security/
├── mod.rs                           (existing file, with updated exports)
├── default.rs                       (implementation file with DefaultServerSecurityContext, ~200 lines)
├── core/
│   ├── mod.rs                       (exports core components)
│   ├── connection.rs                (DTLS connection management, ~150 lines)
│   └── context.rs                   (Security context basics, ~150 lines)
├── client/
│   ├── mod.rs                       (exports client components)
│   └── context.rs                   (DefaultClientSecurityContext implementation, ~200 lines)
├── dtls/
│   ├── mod.rs                       (exports DTLS components)
│   ├── handshake.rs                 (DTLS handshake handling, ~150 lines)
│   └── transport.rs                 (DTLS transport handling, ~150 lines)
├── srtp/
│   ├── mod.rs                       (exports SRTP components)
│   └── keys.rs                      (SRTP key management, ~150 lines)
└── util/
    ├── mod.rs                       (exports utility functions)
    └── conversion.rs                (Type conversion utilities, ~100 lines)
```

## Module Responsibilities

1. **mod.rs**: Exports the DefaultServerSecurityContext directly from the default module
   
2. **default.rs**: Contains the actual DefaultServerSecurityContext implementation with trait implementation that delegates to other modules

3. **core/**:
   - **connection.rs**: DTLS connection setup and management
   - **context.rs**: Base security context functionality shared by client and server

4. **client/**:
   - **context.rs**: DefaultClientSecurityContext implementation

5. **dtls/**:
   - **handshake.rs**: DTLS handshake state machine and processing
   - **transport.rs**: DTLS transport setup and management

6. **srtp/**:
   - **keys.rs**: SRTP key management

7. **util/**:
   - **conversion.rs**: Type conversion utilities

## Progress

- [x] **Phase 1: Setup**
  - [x] Create the directory structure
  - [x] Create main mod.rs files for each subdirectory
  - [x] Create placeholders for all module files

- [x] **Phase 2: Core Implementation**
  - [x] Create default.rs with basic DefaultServerSecurityContext implementation
  - [x] Create connection.rs with DTLS connection management functionality
  - [x] Create context.rs with basic security context functionality
  - [x] Implement delegate methods in default.rs that call into module functions
  - [x] Fix compatibility issues between the module interfaces and their usage

- [x] **Phase 3: Client Implementation**
  - [x] Create context.rs with DefaultClientSecurityContext implementation
  - [x] Extract client-specific methods to the client module
  - [x] Update default.rs to delegate to client module functions
  - [x] Fix module visibility and trait implementation compatibility issues

- [x] **Phase 4: DTLS Implementation**
  - [x] Create handshake.rs with DTLS handshake functionality
  - [x] Create transport.rs with DTLS transport functionality
  - [x] Implement actual functionality in DTLS modules
  - [x] Update default.rs to delegate to DTLS module functions

- [x] **Phase 5: SRTP Implementation**
  - [x] Create keys.rs with SRTP key management functionality
  - [x] Implement actual functionality in SRTP module
  - [x] Update default.rs to delegate to SRTP module functions

- [x] **Phase 6: Utilities**
  - [x] Create conversion.rs with type conversion utilities
  - [x] Implement actual utility functions
  - [x] Update other modules to use utility functions

- [x] **Phase 7: Integration & Testing**
  - [x] Update mod.rs to export DefaultServerSecurityContext from default.rs
  - [x] Remove server_security_impl.rs
  - [x] Update server/mod.rs to reference DefaultServerSecurityContext from security module directly
  - [x] Run tests to verify functionality
  - [x] Fix any issues identified during testing

## Current Status

All phases completed. The server security implementation has been completely refactored into smaller, more focused modules. The original monolithic server_security_impl.rs file has been removed, and all references to it have been updated to use the new modular structure. All code compiles successfully, and example applications have been updated to use the new structure.

## Implementation Strategy

1. Work on one module at a time, starting with core functionality
2. Extract code from the original server_security_impl.rs to the appropriate module
3. Ensure all public APIs remain unchanged
4. Add comprehensive documentation to each module
5. Run tests after each module is completed to verify functionality

## Key Components to Extract

1. **DefaultClientSecurityContext struct**: Will move to client/context.rs
2. **DefaultServerSecurityContext struct**: Core structure stays in default.rs
3. **DTLS connection methods**: Move to dtls/handshake.rs and dtls/transport.rs
4. **SRTP key extraction**: Move to srtp/keys.rs
5. **Type conversion methods**: Move to util/conversion.rs
6. **Security context initialization**: Move to core/context.rs
7. **Connection management**: Move to core/connection.rs

## Benefits

- Improved code organization and readability
- Easier maintenance and bug fixing
- Better separation of concerns
- Smaller, more focused files
- More manageable for future contributors
- Consistent structure with server transport implementation 