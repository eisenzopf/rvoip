# Example 2: Creating SIP Messages

This example demonstrates how to create SIP requests and responses using the builder pattern, macros, and shows how to integrate SDP content with SIP messages.

## What You'll Learn

- How to create SIP requests and responses using the SimpleRequestBuilder and SimpleResponseBuilder
- How to use macros for concise message creation
- How to set different types of headers in SIP messages
- How to work with URIs and addresses
- How to create and include SDP session descriptions
- How to use the ContentBuilderExt trait for simplified SDP handling
- How to serialize messages to bytes and parse them

## Running the Example

```bash
# Run the example
cargo run --example 02_creating_messages

# Run with debug logs for more detail
RUST_LOG=debug cargo run --example 02_creating_messages
```

## Code Walkthrough

The example is divided into five parts:

1. **Creating a SIP Request Using the Builder Pattern**
   - Demonstrates building a SIP INVITE request using SimpleRequestBuilder
   - Shows the simplified API for creating From, To, Via and other headers
   - Illustrates how to update an existing request with new headers

2. **Creating a SIP Response Using the Builder Pattern**
   - Shows how to create 200 OK, 180 Ringing, and 404 Not Found responses
   - Demonstrates the convenience constructors like ok(), ringing(), not_found()
   - Explains response status codes and reason phrases

3. **Using Macros for Concise Message Creation**
   - Demonstrates the compact `sip!` macro syntax for creating messages
   - Shows how to create both requests and responses with macros
   - Compares the macro approach with the builder pattern

4. **Creating Messages with Complex Bodies**
   - Shows how to include raw SDP (Session Description Protocol) bodies in SIP messages
   - Demonstrates setting Content-Type and Content-Length headers
   - Illustrates parsing a message and extracting its body

5. **Using the SDP Integration with Builder Pattern**
   - Demonstrates the SdpBuilder for creating structured SDP content
   - Shows how to create audio and video media descriptions
   - Uses the ContentBuilderExt trait to easily attach SDP to SIP messages
   - Illustrates parsing SDP content from a message body

## Key Concepts

### SIP Message Creation Methods

- **SimpleRequestBuilder**: Streamlined, high-level API for creating requests
- **SimpleResponseBuilder**: Streamlined, high-level API for creating responses
- **Macro Syntax**: Concise and readable, ideal for simple messages
- **Immutable Updates**: Using `.with_header()` to create modified copies

### Important Headers

- **From/To**: Identify participants in a SIP dialog
- **Via**: Track the path of requests and responses
- **CSeq**: Sequence number and method for matching requests/responses
- **Contact**: Direct communication URI for subsequent requests
- **Content-Type/Content-Length**: Describe the message body

### SDP Integration

- **SdpBuilder**: Fluent API for creating structured SDP session descriptions
- **ContentBuilderExt**: Extension trait adding SDP-specific methods to request and response builders
- **Media Descriptions**: Audio, video, and other media types
- **Media Attributes**: Direction, codecs, and other media parameters
- **SDP Parsing**: Extracting and interpreting SDP content from message bodies

### SIP URIs and Addresses

- **URI**: Identifies a SIP resource (sip:user@domain)
- **Address**: Combines a URI with a display name and parameters
- **Parameters**: Extend SIP headers with additional information (tags, branches)

### Next Steps

Once you're comfortable with creating SIP messages and integrating SDP content, you can move on to the next example which demonstrates a complete SIP dialog with transaction handling. 