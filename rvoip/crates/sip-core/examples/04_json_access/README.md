# JSON Access Example

This example demonstrates how to use the JSON representation and access layer for SIP types. The JSON layer provides a way to:

1. Convert SIP types to and from JSON
2. Access SIP message fields using path-based notation (similar to JavaScript object access)
3. Query complex data structures using a JSONPath-like query language
4. Perform round-trip conversions between SIP messages and JSON

## Running the Example

To run this example:

```bash
cargo run --example 04_json_access
```

## Example Features

### 1. Converting SIP Messages to JSON

SIP messages can be converted to JSON format, which is useful for:
- Logging and debugging
- Sending over APIs
- Integrating with JavaScript or web applications
- Storing in document databases

```rust
let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    .from("Alice", "sip:alice@atlanta.com", Some("1928301774"))
    .build();

// Convert to JSON string
let json = request.to_json_string_pretty().unwrap();
```

### 2. Path-based Field Access

Access specific fields in a SIP message using dot notation paths:

```rust
// Access the From header's display name
let from_name = response.get_path("headers.from.display_name").as_str().unwrap_or("Not found");

// Access the first Via header's branch parameter
let via_branch = response.get_path("headers.via[0].branch").as_str().unwrap_or("Not found");
```

### 3. Query-based Access

Use a JSONPath-like query syntax for more complex data access:

```rust
// Get all Via branches
let branches = request.query("$.headers.via[*].branch");

// Find all display names anywhere in the message
let display_names = request.query("$..display_name");
```

### 4. JSON Round-trip Conversion

Convert SIP messages to JSON and back:

```rust
// Convert to JSON
let json_str = request.to_json_string().unwrap();

// Create new request from JSON
let new_request = SipRequest::from_json_str(&json_str).unwrap();
```

## Benefits

The JSON access layer provides:

- A more intuitive interface for developers familiar with JSON
- Simple access to deeply nested fields without needing to know the exact Rust types
- Ability to create SIP messages from JSON data (e.g., from configuration files or APIs)
- Consistent pattern for accessing any SIP type across the library 