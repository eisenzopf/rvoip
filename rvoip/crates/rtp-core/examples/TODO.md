# RTP-Core Examples TODO

## Analysis Summary

Based on the analysis of the examples output log, here's a breakdown of the ERROR and WARNING messages:

- **Total ERROR and WARNING messages**: 282
- **Frame-related error messages**: 123 (121 "Error receiving frame"/"No frame received"/"Server receive error" + 2 "Client connection timed out")
- **Percentage of frame-related errors**: Approximately 44% of all ERROR and WARNING messages are directly related to sending and receiving frames.

The vast majority of these frame-related errors are timeout issues, specifically:
- "Error receiving frame: Timeout error: No frame received within timeout period"
- "Server receive error: Timeout error: No frame received within timeout period"
- "Client connection timed out after X seconds"

These errors are concentrated in a few examples, particularly:
- `api_ssrc_demultiplexing.rs`
- `api_ssrc_demux.rs`
- `api_basic.rs`
- `api_srtp.rs`

## Fix Findings

After investigating and fixing the `api_basic.rs` example, we've discovered:

1. **Root Cause**: The primary issue was related to DTLS handshake failures in the security layer.
2. **Solution**: Explicitly disabling security by setting `SecurityMode::None` in both client and server configurations resolves the connection issue.
3. **Results**: With security disabled:
   - Client connects successfully to the server
   - Connection verification passes
   - RTCP packets are exchanged correctly
4. **Remaining Issues**: There are still timeouts when sending frames, but the basic connection is established.

## Tasks by File

### api_basic.rs

- [x] Increase timeout durations and add retry mechanism
- [x] Add better error handling and logging for connection issues
- [x] Add synchronization between client and server
- [x] Add connection verification step
- [x] Disable DTLS security to bypass handshake issues
- [ ] Investigate remaining frame sending issues

### api_ssrc_demultiplexing.rs

- [ ] Explicitly disable security by setting `SecurityMode::None`
- [ ] Increase the timeout duration for frame receiving operations
- [ ] Add synchronization mechanism to ensure server is ready before client attempts to send frames
- [ ] Add explicit error handling for timeout scenarios
- [ ] Add more robust event notification when frames are successfully received
- [ ] Consider reducing the number of receive attempts to avoid excessive warnings

### api_ssrc_demux.rs

- [ ] Explicitly disable security by setting `SecurityMode::None`
- [ ] Increase timeout duration for receive operations
- [ ] Add logging to verify the SSRC registration process is working correctly
- [ ] Add confirmation when frames are actually sent to help diagnose if the issue is with sending or receiving
- [ ] Ensure the server and client are properly synchronized before attempting communications
- [ ] Consider implementing a retry mechanism with backoff instead of continuous polling

### api_srtp.rs

- [ ] Note: This example specifically tests SRTP, so disabling security is not an option
- [ ] Focus on fixing the DTLS handshake issues rather than bypassing them
- [ ] Ensure SRTP security parameters are correctly initialized before attempting communication
- [ ] Add verification steps to confirm encryption/decryption is working properly
- [ ] Add more detailed logging around the security handshake process
- [ ] Consider simplifying the example to isolate and fix the timeout issues

## Security Recommendations

Based on our findings:

1. **Basic Examples**: Explicitly disable security in basic examples that don't need to demonstrate secure communication.
2. **Security Examples**: Keep security enabled only in examples specifically demonstrating secure features.
3. **Documentation**: Add clear comments indicating whether an example uses security or not.
4. **DTLS Fixes**: For examples that need security, investigate and fix the DTLS handshake implementation.

## General Improvements

For all examples with timeout issues:

- [ ] Consider standardizing the timeout handling across all examples
- [ ] Add a configurable timeout parameter that can be adjusted for different environments
- [ ] Implement proper cleanup of resources even when timeouts occur
- [ ] Add better documentation explaining expected timeout behavior
- [ ] Consider adding a debugging mode that shows more detailed communication information
- [ ] Add a security configuration flag in example code to make it easy to toggle security on/off 