# SIP Core Test Suite

This directory contains a comprehensive test suite for the `rvoip-sip-core` library, focusing on RFC compliance and edge cases in SIP message parsing and processing.

## Test Structure

The test suite is organized into several modules:

- `torture_tests.rs`: Main test harness with helper functions for RFC compliance testing
- `rfc_compliance/`: Directory containing test files based on RFC standards
  - `wellformed/`: Well-formed SIP messages that should parse successfully
  - `malformed/`: Malformed SIP messages that should be rejected
- `debug_parser.rs`: Utilities for debugging parser issues
- `parser_tests.rs`: Additional parser tests

## Running the Tests

To run the entire test suite:

```bash
cargo test -p rvoip-sip-core
```

To run specific test modules:

```bash
# Run RFC compliance tests
cargo test --test torture_tests --features="lenient_parsing"

# Run specific test case
cargo test --test torture_tests test_wellformed_messages --features="lenient_parsing"
cargo test --test torture_tests test_malformed_messages --features="lenient_parsing"
```

## Parsing Modes

The test suite validates both strict and lenient parsing modes:

### Strict Mode

- Enforces RFC 3261 compliance strictly
- Rejects messages with Content-Length mismatches
- Requires all mandatory headers to be present
- Used primarily for `test_malformed_messages` to ensure that invalid messages are properly rejected

### Lenient Mode

- More forgiving of minor deviations from the RFC
- Handles Content-Length mismatches (both too large and too small)
- Preserves unparseable headers as raw headers instead of failing
- Used primarily for `test_wellformed_messages` to handle edge cases in real-world SIP traffic

## Test Categories

### RFC 4475 Tests

These tests cover a wide range of valid and invalid SIP messages as defined in RFC 4475:

- Valid but unusual SIP messages, including syntactically-correct edge cases
- Invalid SIP messages that should be rejected
- Malformed SIP components (headers, URIs, etc.)
- Messages designed to test parser robustness

### RFC 5118 Tests

Tests focused on IPv6 support in SIP messages:

- IPv6 addresses in various SIP header fields
- Zone indices in IPv6 addresses
- Multicast IPv6 addresses
- Malformed IPv6 addresses

### Custom Torture Tests

Additional test cases not covered by the RFCs:

- Extremely long header values
- Unusual HTTP methods
- Unusual header names
- Malformed request URIs
- Exotic status codes
- Unexpected line endings
- Multiple headers with the same name
- And more

## Excluded Tests

Some tests are intentionally excluded from validation:

1. In `is_excluded_wellformed_test()`: Messages that are technically valid according to RFC 4475 but which we explicitly don't support for security or implementation reasons

2. In `skip_content_length_validation()`: Messages with known Content-Length issues that are part of torture testing

## Troubleshooting

If a test is failing, you can use the `debug_parser.rs` utilities to investigate:

```bash
cargo test --test debug_parser debug_parse_longreq
```

## Contributing New Tests

When adding new tests:

1. Place well-formed test files in `rfc_compliance/wellformed/`
2. Place malformed test files in `rfc_compliance/malformed/`
3. Follow the naming convention: `<section>_<description>.sip`
4. If needed, add an exclusion entry for special cases 