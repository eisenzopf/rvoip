# Example 1: Basic SIP Message Parsing

This example demonstrates how to parse raw SIP messages into structured types and how to access the various components of a SIP message.

## What You'll Learn

- How to parse raw SIP messages using the `parse_message` function
- How to determine if a message is a request or response
- How to access typed headers using the `typed_header` and `typed_headers` methods
- How to extract parameters from headers (e.g., tags, branch parameters)
- How to handle messages with multiple headers of the same type
- Basic structure of SIP requests and responses

## Running the Example

```bash
# Run the example
cargo run --example 01_basic_parsing

# Run with debug logs for more detail
RUST_LOG=debug cargo run --example 01_basic_parsing
```

## Code Walkthrough

The example is divided into three parts:

1. **Parsing a SIP INVITE Request**
   - Demonstrates parsing a basic SIP INVITE request
   - Shows how to access common headers like From, To, Contact, and Via
   - Extracts the tag from the From header and the branch parameter from Via

2. **Parsing a SIP Response**
   - Shows how to parse a 200 OK response
   - Demonstrates accessing status code and reason phrase
   - Handles multiple Via headers which are common in responses
   - Checks for the To tag which is important for dialog establishment

3. **Handling Multiple Headers**
   - Shows how to work with multiple headers of the same type (Record-Route)
   - Demonstrates iterating through headers and checking for parameters
   - Shows alternative ways to access headers (by name vs. by type)

## Key Concepts

### Message Types

SIP defines two main message types:
- **Requests**: Used to initiate actions (INVITE, BYE, ACK, etc.)
- **Responses**: Used to reply to requests (200 OK, 404 Not Found, etc.)

### Important Headers

- **From**: Identifies the initiator of the request
- **To**: Identifies the intended recipient of the request
- **Via**: Shows the path taken by the request so far
- **CSeq**: Contains a sequence number and method name for request matching
- **Call-ID**: Unique identifier for the call
- **Contact**: Contains a URI for direct communication
- **Record-Route**: Used by proxies to stay in the signaling path

### Next Steps

Once you're comfortable with parsing SIP messages, you can move on to the next example which demonstrates how to create SIP messages using both the builder pattern and macros. 