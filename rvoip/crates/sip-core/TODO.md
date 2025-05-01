# rvoip-sip-core: TODO List

## API Consistency Improvements

- [ ] **Standardize Method vs. Field Access**
  - Ensure consistent access patterns across the library (methods preferred over direct field access)
  - Add method wrappers for fields that need to be publicly accessible
  - Fix methods like `version()`, `status_code()`, `uri()` for consistent API

- [ ] **Complete TypedHeaderTrait Implementations**
  - Implement `TypedHeaderTrait` for all common SIP headers:
    - [x] From
    - [x] To
    - [x] Via
    - [x] Contact
    - [x] CSeq
    - [x] RecordRoute
    - [x] Route
    - [ ] Other standard headers

## Extended Functionality

- [ ] **Header Access Utilities**
  - [ ] Add `typed_headers<T>()` method to get multiple headers of the same type
  - [ ] Implement `headers_by_name(name: &str)` for string-based header access
  - [ ] Ensure consistent raw header access methods

- [ ] **SIP Message Utilities**
  - [ ] Add convenience methods for common operations
  - [ ] Ensure builder patterns are consistent and intuitive

## Documentation and Examples

- [ ] **Update Tutorial Examples**
  - [ ] Fix existing examples to work with current API
  - [ ] Add examples demonstrating best practices
  - [ ] Ensure examples follow consistent patterns

- [ ] **API Documentation**
  - [ ] Complete documentation for all public methods and types
  - [ ] Add usage examples in documentation

## Testing

- [ ] **Expand Test Coverage**
  - [ ] Add tests for all header types
  - [ ] Add tests for edge cases
  - [ ] Ensure consistent behavior across different SIP message types 