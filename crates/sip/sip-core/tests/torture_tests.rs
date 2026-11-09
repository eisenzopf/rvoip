//! The RFC 4475 and RFC 5118 torture suites as a test target of their own,
//! for the release gate `test.rfc4475`. The same modules also run as part of
//! the `lib` target (`rfc_compliance`).

#[path = "rfc_compliance/torture_test.rs"]
mod torture_test;

#[path = "rfc_compliance/torture_element_test.rs"]
mod torture_element_test;
