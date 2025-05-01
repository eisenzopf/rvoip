# Example 7: Multipart Message Handling

This example demonstrates how to work with multipart MIME bodies in SIP messages. Multipart messages allow combining multiple content types in a single SIP message, which is useful for advanced use cases like call transfers, presence information, and providing alternative formats of the same content.

## What You'll Learn

- How to create multipart MIME bodies in SIP messages
- How to parse incoming multipart messages
- How to work with different multipart subtypes (mixed, alternative, related)
- How to handle Content-Type and Content-Disposition headers for multipart contents
- Common real-world use cases for multipart messages in SIP applications

## Running the Example

```bash
# Run the example
cargo run --example 07_multipart_messages

# Run with debug logs to see the actual SIP messages
RUST_LOG=debug cargo run --example 07_multipart_messages
```

## Code Walkthrough

The example is divided into four parts:

1. **Basic Multipart Message Creation**
   - Shows how to create a multipart/mixed body with multiple content types
   - Demonstrates including SDP and XML content in the same message
   - Illustrates proper Content-Type header handling with boundary parameter
   - Shows how to create and verify Content-Length for multipart bodies

2. **Parsing Multipart Messages**
   - Shows how to detect and parse incoming multipart messages
   - Demonstrates extracting individual parts from a multipart body
   - Illustrates handling Content-Type and Content-Disposition for each part
   - Shows content-specific processing based on MIME types

3. **REFER with Replaces Example**
   - Demonstrates a real-world call transfer scenario
   - Shows how to work with the Replaces header in call transfers
   - Illustrates using multipart bodies for resource lists in conferencing
   - Demonstrates XML content handling for resource lists

4. **Session Descriptions with Alternative Formats**
   - Shows how to provide multiple versions of the same content
   - Demonstrates multipart/alternative for backward compatibility
   - Illustrates combining SDP and JSON session descriptions
   - Shows how clients can select the most appropriate content format

## Key Concepts

### Multipart MIME Types

SIP supports several multipart MIME subtypes based on RFC 2046:

1. **multipart/mixed**: Contains independent parts that can be processed separately
2. **multipart/alternative**: Contains multiple representations of the same content
3. **multipart/related**: Contains parts with parent-child relationships
4. **multipart/form-data**: Used for web form submissions (less common in SIP)

### MIME Part Structure

Each part in a multipart body has:
- Content-Type header specifying the MIME type
- Optional Content-Disposition header indicating how to present the content
- Optional Content-ID, Content-Transfer-Encoding, etc.
- Body content

### Common Use Cases

1. **REFER with Resource Lists**: For referring multiple contacts or conference participants
2. **Rich Presence Information**: Combining presence status with avatar images or other media
3. **Alternative Session Formats**: Providing both SDP and JSON/XML session descriptions
4. **Mixed Media Information**: Combining textual and binary information in a single message
5. **Security Information**: Including encrypted content along with signatures

### Best Practices

- Always set the correct Content-Type and boundary parameter
- Set the correct Content-Length for the entire multipart body
- Include appropriate Content-Disposition for each part
- Process multipart content according to the multipart subtype semantics
- Be prepared to handle unknown content types gracefully

## Next Steps

After mastering multipart message handling, you can move on to Example 8 which covers building a complete SIP client application. 