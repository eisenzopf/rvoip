# rvoip-sip-core: TODO List

## API Consistency Improvements

- [ ] **Standardize Method vs. Field Access**
  - Ensure consistent access patterns across the library (methods preferred over direct field access)
  - Add method wrappers for fields that need to be publicly accessible
  - Fix methods like `version()`, `status_code()`, `uri()` for consistent API

- [x] **Complete TypedHeaderTrait Implementations**
  - Implement `TypedHeaderTrait` for all common SIP headers:
    - [x] From
    - [x] To
    - [x] Via
    - [x] Contact
    - [x] CSeq
    - [x] RecordRoute
    - [x] Route
    - [x] CallId
    - [x] MaxForwards
    - [x] ContentLength
    - [x] ContentType
    - [x] Expires
    - [x] Authorization
    - [x] WwwAuthenticate
    - [x] ProxyAuthenticate
    - [x] ProxyAuthorization
    - [x] AuthenticationInfo
    - [x] Accept
    - [x] Allow
    - [x] ReplyTo
    - [x] ReferTo
    - [x] Warning
    - [x] ContentDisposition
    - [x] AcceptLanguage
    - [x] Organization
    - [x] Priority
    - [x] Subject
    - [x] Server
    - [x] InReplyTo
    - [x] RetryAfter
    - [x] ErrorInfo
    - [x] CallInfo
    - [x] Supported
    - [x] Unsupported
    - [x] AcceptEncoding
    - [x] AlertInfo
    - [x] ContentEncoding
    - [x] ContentLanguage
    - [x] Date
    - [x] MinExpires
    - [x] MimeVersion
    - [x] ProxyRequire
    - [x] Timestamp
    - [x] UserAgent

## Extended Functionality

- [x] **Header Access Utilities**
  - [x] Implement in Request struct:
    - [x] `typed_headers<T>()` method to get multiple headers of the same type
    - [x] `headers_by_name(name: &str)` method for string-based header access
    - [x] `raw_header_value(name: &HeaderName)` method for raw header access
    - [x] `has_header(name: &HeaderName)` method to check header presence
    - [x] `header_names()` method to list all header names in the message
  - [x] Implement in Response struct:
    - [x] `typed_headers<T>()` method to get multiple headers of the same type
    - [x] `headers_by_name(name: &str)` method for string-based header access
    - [x] `raw_header_value(name: &HeaderName)` method for raw header access
    - [x] `has_header(name: &HeaderName)` method to check header presence
    - [x] `header_names()` method to list all header names in the message
  - [x] Implement in Message enum:
    - [x] `typed_headers<T>()` method to get multiple headers of the same type
    - [x] `headers_by_name(name: &str)` method for string-based header access
    - [x] `raw_header_value(name: &HeaderName)` method for raw header access
    - [x] `has_header(name: &HeaderName)` method to check header presence
    - [x] `header_names()` method to list all header names in the message
  - [x] Create HeaderAccess trait to consolidate shared functionality
  - [ ] Add comprehensive documentation and examples
  - [x] Add unit tests for all header access methods

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