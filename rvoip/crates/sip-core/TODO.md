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

- [ ] **Enhance Builder Patterns for All Headers**
  - Add convenient builder methods for all header types, rather than relying on generic `header()` method:
    - [x] **Authentication Headers**:
      - [x] Authorization (`.authorization()` or `.auth_digest()`)
      - [x] WwwAuthenticate (`.www_authenticate()`)
      - [x] ProxyAuthenticate (`.proxy_authenticate()`)
      - [x] ProxyAuthorization (`.proxy_authorization()`)
      - [x] AuthenticationInfo (`.auth_info()`)
    - [x] **Content-Related Headers**:
      - [x] ContentEncoding (`.content_encoding()`)
      - [x] ContentLanguage (`.content_language()`)
      - [x] ContentDisposition (`.content_disposition()`)
    - [x] **Accept Headers**:
      - [x] Accept (`.accept()`)
      - [x] AcceptEncoding (`.accept_encoding()`)
      - [x] AcceptLanguage (`.accept_language()`)
    - [x] **Routing Headers**:
      - [x] RecordRoute (`.record_route()`)
      - [x] Route (`.route()`)
    - [x] **Feature/Capability Headers**:
      - [x] Allow (`.allow()`)
      - [x] Supported (`.supported()`)
      - [x] Unsupported (`.unsupported()`)
      - [x] Require (`.require()`)
    - [ ] **Informational Headers**:
      - [ ] UserAgent (`.user_agent()`)
      - [ ] Server (`.server()`)
      - [ ] Warning (`.warning()`)
      - [ ] Date (`.date()`)
      - [ ] Timestamp (`.timestamp()`)
      - [ ] Organization (`.organization()`)
      - [ ] Subject (`.subject()`)
      - [ ] Priority (`.priority()`)
      - [ ] MimeVersion (`.mime_version()`)
    - [ ] **Session Management Headers**:
      - [ ] Expires (`.expires()`)
      - [ ] MinExpires (`.min_expires()`)
      - [ ] RetryAfter (`.retry_after()`)
    - [ ] **Reference/Redirection Headers**:
      - [ ] ReplyTo (`.reply_to()`)
      - [ ] ReferTo (`.refer_to()`)
      - [ ] InReplyTo (`.in_reply_to()`)
      - [ ] ErrorInfo (`.error_info()`)
      - [ ] CallInfo (`.call_info()`)
      - [ ] AlertInfo (`.alert_info()`)
  - Each builder method should:
    - Accept appropriate parameters based on the header's structure
    - Handle reasonable error cases gracefully
    - Return self for method chaining
    - Include comprehensive documentation and examples

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