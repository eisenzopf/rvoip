# Restructuring Notice

The RVOIP SIP Client library has been restructured to improve maintainability. The large monolithic files have been broken down into smaller, more focused modules:

## Call Module Structure

The `call.rs` file has been broken down into:

- `call/mod.rs` - Main exports and organization
- `call/types.rs` - Core types: CallDirection, CallState, StateChangeError
- `call/events.rs` - CallEvent enum
- `call/call.rs` - Main Call struct and implementation
- `call/registry_interface.rs` - CallRegistryInterface trait
- `call/weak_call.rs` - WeakCall struct and implementation
- `call/utils.rs` - Helper functions for call-related operations

## Client Module Structure

The `client.rs` file has been broken down into:

- `client/mod.rs` - Main exports and organization
- `client/events.rs` - SipClientEvent enum
- `client/registration.rs` - Registration struct and implementation
- `client/client.rs` - Main SipClient implementation
- `client/lightweight.rs` - LightweightClient implementation
- `client/utils.rs` - Helper functions and traits for client-related operations

## Public API

The public API remains unchanged, and all previously exported types and functions are still available.

## Implementation Notes

Some of the method implementations are currently stubbed out and will need to be filled in with the
original code logic. This restructuring addresses the file organization without modifying the core logic. 