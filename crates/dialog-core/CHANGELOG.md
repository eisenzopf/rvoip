# Changelog

All notable changes to the dialog-core crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial dialog-core crate structure
- RFC 3261 dialog management layer
- DialogManager main interface
- Core dialog types (DialogId, Dialog, DialogState)
- Session coordination events for integration with session-core
- Basic error handling and recovery framework
- SIP protocol handler stubs (INVITE, BYE, REGISTER, UPDATE)
- Request/response routing framework
- SDP negotiation coordination framework
- Dialog recovery and failure handling framework

### Changed
- N/A (initial release)

### Deprecated
- N/A (initial release)

### Removed
- N/A (initial release)

### Fixed
- N/A (initial release)

### Security
- N/A (initial release)

## [0.1.0] - TBD

### Added
- Initial release of dialog-core
- Basic dialog management functionality
- Integration with transaction-core layer
- Session coordination with session-core 