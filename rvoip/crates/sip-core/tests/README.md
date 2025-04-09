# SIP Core Test Suite

This directory contains a comprehensive test suite for the `rvoip-sip-core` library, focusing on RFC compliance and edge cases in SIP message parsing and processing.

## Test Structure

The test suite is organized into several modules:

- `torture_tests.rs`: Main test harness with helper functions and basic SIP message tests
- `rfc4475.rs`: Tests based on [RFC 4475](https://tools.ietf.org/html/rfc4475) - SIP Torture Test Messages
- `rfc5118.rs`: Tests based on [RFC 5118](https://tools.ietf.org/html/rfc5118) - SIP IPv6 Torture Tests
- `custom_torture.rs`: Additional custom tests for edge cases not covered by the RFCs

## Running the Tests

To run the entire test suite:

```bash
cargo test -p rvoip-sip-core
```

To run specific test modules:

```bash
cargo test -p rvoip-sip-core --test torture_tests
cargo test -p rvoip-sip-core --test rfc4475
cargo test -p rvoip-sip-core --test rfc5118
cargo test -p rvoip-sip-core --test custom_torture
```

## Current Status

As of the initial implementation, many tests are failing due to specific differences between the expected behavior in the tests and the actual implementation of the SIP parser. These failures highlight areas where:

1. The parser may be too lenient or too strict compared to the RFC requirements
2. The parser normalizes header values differently than expected in the tests
3. The error handling differs from what the tests expect

These failures serve as a valuable guide for improving the SIP parser's compliance with the RFCs.

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

## Future Improvements

1. Fix the implementation of the SIP parser to handle the failing test cases
2. Make the test validation more robust and more in line with the actual parser behavior
3. Add more test cases for scenarios not covered by the RFCs
4. Add property-based testing for randomly generated SIP messages

## Contribution

When contributing to the test suite:

1. Make sure your tests are well-documented with references to relevant RFC sections
2. Include both valid and invalid test cases
3. Test edge cases and unusual situations
4. Consider adding custom tests for scenarios not covered by the RFCs 