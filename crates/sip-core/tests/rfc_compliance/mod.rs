// RFC Compliance Test Modules

// This directory contains tests for validating the SIP parser against
// SIP RFC test cases, including RFC 4475 (SIP Torture Tests).

// Current active modules
pub mod torture_test; // RFC 4475 torture tests
pub mod macro_builder_tests; // Tests for macro builder roundtrip conversion

// Future modules - uncomment when implemented
// pub mod rfc3261_examples; // Examples from core SIP spec RFC 3261
// pub mod rfc5118_test; // Tests for Internationalized domain names and URIs RFC 5118 