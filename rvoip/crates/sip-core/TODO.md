# rvoip-sip-core: TODO List

## API Consistency Improvements

- [x] **Standardize Method vs. Field Access**
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

- [x] **Builder Patterns**
  - [x] Implement `SimpleRequestBuilder` to create requests more easily
  - [x] Implement `SimpleResponseBuilder` to create responses more easily
  - [x] Fix `HeaderSetter` trait usage for consistency
  - [x] Enable method chaining for all builders

- [ ] **Builder Update Patterns**
  - [ ] Create simpler syntax for updating existing SIP requests and responses
  - [ ] Add extension traits for common header updates (e.g., `with_subject()`, `with_priority()`)
  - [ ] Implement `RequestUpdater` and `ResponseUpdater` classes for builder-like updating
  - [ ] Add header-specific update methods (e.g., `update_from()`, `update_via()`)
  - [ ] Create "modified copy" capabilities for builders
  - [ ] Add batch operation support for more efficient multiple updates

- [x] **Routing Headers**
  - [x] **RecordRoute**:
    - Add RecordRoute entry manipulation methods
    - Add RecordRoute to request and response builders
  - [x] **Route**:
    - Add Route manipulation methods for UAC/UAS processing
    - Add Route to request and response builders
  - [x] **Path**:
    - Implement TypedHeaderTrait
    - Add to request and response builders

- [x] **Feature/Capability Headers**
  - [x] **Require**: Add builder helpers
  - [x] **Supported**: Add builder helpers
  - [x] **Unsupported**: Add builder helpers
  - [x] **ProxyRequire**: Add builder helpers

- [x] **Information Headers**
  - [x] **User-Agent**: 
    - Add builder methods for User-Agent
    - Add helper variants for common values
  - [x] **Server**: 
    - Add builder methods for Server header
    - Add helper variants for common values

- [x] **Session/Status Info Headers**
  - [x] **CallID**: Ensure access and manipulation is consistent
  - [x] **InReplyTo**: Add builder methods
  - [x] **ReplyTo**: Add builder methods

- [x] **Media/Content Headers**
  - [x] **Accept Headers**:
    - Implement `accept()` method on builders
    - Add helper methods for common types
  - [x] **Content Headers**:
    - Implement content type handler helpers
    - Add multipart content generation
    - Add text/plain shortcuts
    - Add SDP handling

- [x] **Authentication Headers**
  - [x] **WWW-Authenticate**: Ensure integrated access in Response
  - [x] **Authorization**: Ensure integrated access in Request
  - [x] **Proxy-Authenticate**: Add helper methods 
  - [x] **Proxy-Authorization**: Add helper methods 
  - [x] **Authentication-Info**: Add helper methods

## Feature Improvements

- [x] **SDP Integration**
  - [x] Complete SDP building API improvements:
    - Easier creation of typical audio/video configs
    - Default values for common formats 
    - WebRTC BUNDLE configuration
  - [x] Add SDP/SIP interoperability functions
    - Map SDP connection data to SIP Contact
    - Auto-generate o-line from SIP fields 
  - [x] Add multimedia session convenience helpers

- [ ] **Serialization Format Support**
  - [ ] Add serde support for core types
  - [ ] Add JSON format conversion
  - [ ] Add structured logging of messages

- [ ] **Parsing Improvements**
  - [ ] Enhance ABNF compliance in corner cases
  - [ ] Better error reporting for malformed messages
  - [ ] Support more extensions and custom headers

- [ ] **Authentication Support**
  - [ ] Complete Digest authentication
  - [ ] Add NTLM authentication 
  - [ ] Add Basic authentication
  - [ ] Add auth challenges helper

## Documentation

- [ ] **Add more examples**
  - [ ] Basic REGISTER transaction  
  - [ ] INVITE dialog with SDP negotiation
  - [ ] Authentication flow example
  - [ ] Proxy routing example
  - [ ] B2BUA example
  - [ ] Message parsing example

- [ ] **Improve documentation**
  - [ ] Add more doc tests
  - [ ] Add architecture overview  
  - [ ] Add SIP protocol reference links
  - [ ] Explain RFC compliance details

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
  - [x] Add comprehensive documentation and examples
  - [x] Add unit tests for all header access methods

- [ ] **SIP Message Utilities**
  - [ ] Add convenience methods for common operations
  - [ ] Ensure builder patterns are consistent and intuitive

## Documentation and Examples

- [x] **Update Tutorial Examples**
  - [x] Fix existing examples to work with current API
  - [x] Add examples demonstrating best practices
  - [x] Ensure examples follow consistent patterns

- [ ] **API Documentation**
  - [ ] Complete documentation for all public methods and types
  - [ ] Add usage examples in documentation

## Testing

- [ ] **Expand Test Coverage**
  - [ ] Add tests for all header types
  - [ ] Add tests for edge cases
  - [ ] Ensure consistent behavior across different SIP message types 