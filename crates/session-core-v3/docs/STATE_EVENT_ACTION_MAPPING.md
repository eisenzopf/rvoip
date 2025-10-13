# Session-Core-v2 State-Event-Action Mapping Reference

This document provides a detailed mapping showing which events can be received in each state and what actions are performed. This is essential for understanding the state machine behavior and implementing the event system correctly.

## Part 1: State-Centric View (What events can each state handle?)

### 1. **Idle** State
The starting state with no active call.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **MakeCall** | UAC | Initiating | - CreateDialog<br>- CreateMediaSession<br>- GenerateLocalSDP<br>- SendINVITE | None | HasMediaSession: true |
| **IncomingCall** | UAS | Ringing | - CreateMediaSession<br>- StoreRemoteSDP<br>- SendSIPResponse(180, "Ringing") | None | HasRemoteSDP: true<br>HasMediaSession: true |

### 2. **Initiating** State  
Outbound call is being initiated.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog180Ringing** | UAC | Ringing | None | None | None |
| **Dialog183SessionProgress** | UAC | EarlyMedia | - StoreRemoteSDP<br>- NegotiateSDPAsUAC<br>- StartMediaSession | HasRemoteSDP | SDPNegotiated: true<br>MediaSessionReady: true |
| **Dialog200OK** | UAC | Active | - StoreRemoteSDP<br>- NegotiateSDPAsUAC<br>- SendACK<br>- StartMediaFlow | HasRemoteSDP | DialogEstablished: true<br>SDPNegotiated: true<br>MediaSessionReady: true |
| **Dialog4xxFailure** | UAC | Failed(Rejected) | - SendACK | None | None |
| **Dialog5xxFailure** | UAC | Failed(NetworkError) | None | None | None |
| **Dialog6xxFailure** | UAC | Failed(Rejected) | None | None | None |
| **DialogTimeout** | Both | Failed(Timeout) | - SendCANCEL<br>- CleanupResources | None | None |
| **HangupCall** | Both | Cancelled | - SendCANCEL<br>- StartDialogCleanup<br>- StartMediaCleanup | None | None |

### 3. **Ringing** State
Call is ringing (UAC waiting for answer, UAS notifying user).

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **AcceptCall** | UAS | Active | - NegotiateSDPAsUAS<br>- GenerateLocalSDP<br>- Send200OK | HasMediaSession | SDPNegotiated: true |
| **RejectCall** | UAS | Terminated | - SendSIPResponse(486, "Busy Here") | None | None |
| **Dialog200OK** | UAC | Active | - StoreRemoteSDP<br>- NegotiateSDPAsUAC<br>- SendACK<br>- StartMediaFlow | HasRemoteSDP | DialogEstablished: true<br>SDPNegotiated: true<br>MediaSessionReady: true |
| **Dialog183SessionProgress** | UAC | EarlyMedia | - StoreRemoteSDP<br>- StartEarlyMedia | None | None |
| **HangupCall** | Both | Cancelled | - SendCANCEL<br>- StartDialogCleanup<br>- StartMediaCleanup | None | None |

### 4. **EarlyMedia** State
Early media is being received/sent before call establishment.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog200OK** | UAC | Active | - SendACK<br>- StoreNegotiatedConfig | None | None |
| **HangupCall** | Both | Terminating | - SendBYE<br>- StartDialogCleanup<br>- StartMediaCleanup | None | None |

### 5. **Active** State
Call is established with bidirectional media.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **HoldCall** | Both | OnHold | - UpdateMediaDirection(sendonly)<br>- SendReINVITE | None | None |
| **MuteCall** | Both | Muted | - MuteLocalAudio | None | None |
| **BlindTransfer** | Both | Transferring | - SendREFER | None | None |
| **StartAttendedTransfer** | Both | ConsultationCall | - HoldCurrentCall<br>- CreateConsultationCall | None | None |
| **CreateConference** | Both | ConferenceHost | - CreateAudioMixer<br>- RedirectToMixer(self) | None | HasMixer: true<br>IsConferenceHost: true |
| **JoinConference** | Both | InConference | - ConnectToMixer | None | None |
| **SendDTMF** | Both | Active | - SendDTMFTone | None | None |
| **StartRecording** | Both | Active | - StartRecordingMedia | None | None |
| **StopRecording** | Both | Active | - StopRecordingMedia | None | None |
| **HangupCall** | Both | Terminating | - SendBYE<br>- StopMediaFlow | None | None |
| **DialogBYE** | Both | Terminating | - SendSIPResponse(200, "OK")<br>- StopMediaFlow | None | None |
| **DialogError** | Both | Failed(ProtocolError) | - StartEmergencyCleanup | None | None |
| **MediaError** | Both | Active | - AttemptMediaRecovery | None | None |

### 6. **OnHold** State
Call is on hold.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **ResumeCall** | Both | Resuming | - UpdateMediaDirection(sendrecv)<br>- SendReINVITE | None | None |
| **HangupCall** | Both | Terminating | - SendBYE<br>- StartDialogCleanup<br>- StartMediaCleanup | None | None |

### 7. **Resuming** State
Call is being resumed from hold.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog200OK** | Both | Active | - SendACK<br>- RestoreMediaFlow | None | None |

### 8. **Muted** State
Microphone is muted but call remains active.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **UnmuteCall** | Both | Active | - UnmuteLocalAudio | None | None |
| **HangupCall** | Both | Terminating | - SendBYE<br>- StopMediaFlow | None | None |

### 9. **ConferenceHost** State
Session is hosting a conference.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **AddParticipant** | Both | ConferenceHost | - CreateBridge<br>- RedirectToMixer(target) | HasMixer | None |
| **HoldCall** | Both | ConferenceOnHold | - MuteToMixer<br>- SendReINVITE | None | None |
| **StartRecording** | Both | ConferenceHost | - StartRecordingMixer | None | None |
| **HangupCall** | Both | Terminating | - DestroyMixer<br>- SendBYE | None | None |

### 10. **InConference** State
Session is a participant in a conference.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **LeaveConference** | Both | Active | - DisconnectFromMixer<br>- RestoreDirectMedia | None | None |
| **MuteInConference** | Both | InConference | - MuteToMixer | None | None |
| **SendDTMF** | Both | InConference | - SendDTMFTone | None | None |
| **HangupCall** | Both | Terminating | - DisconnectFromMixer<br>- SendBYE | None | None |

### 11. **ConferenceOnHold** State
Conference participant is on hold.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **ResumeCall** | Both | ConferenceHost | - UnmuteToMixer<br>- SendReINVITE | None | None |

### 12. **Transferring** State
Call is being transferred.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **TransferSuccess** | Both | Terminating | - SendBYE<br>- StartMediaCleanup | None | None |
| **TransferFailed** | Both | Active | None | None | None |

### 13. **ConsultationCall** State
In consultation call for attended transfer.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **CompleteAttendedTransfer** | Both | Transferring | - SendREFERWithReplaces<br>- TerminateConsultationCall | None | None |
| **HangupCall** | Both | Active | - TerminateConsultationCall<br>- ResumeOriginalCall | None | None |

### 14. **Bridged** State
Call is bridged to another session.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **UnbridgeCall** | Both | Active | - RestoreDirectMedia | None | None |
| **HangupCall** | Both | Terminating | - SendBYE<br>- StartDialogCleanup<br>- StartMediaCleanup | None | None |

### 15. **Terminating** State
Call is being terminated.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **InternalCleanupComplete** | Both | Terminated | - ReleaseAllResources | None | None |

### 16. **Terminated** State
Call has ended. This is a final state.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Reset** | Both | Idle | None | None | None |

### 17. **Cancelled** State
Call was cancelled before establishment. This is a final state.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Reset** | Both | Idle | None | None | None |

### 18. **Failed** State
Call failed. This is a final state with specific failure reasons.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Reset** | Both | Idle | None | None | None |

### 19. **Registering** State
SIP registration in progress.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog200OK** | Both | Registered | - StoreRegistration<br>- StartRegistrationRefreshTimer | None | IsRegistered: true |
| **Dialog4xxFailure** | Both | Idle | - HandleRegistrationFailure | None | None |
| **Dialog5xxFailure** | Both | Idle | - HandleRegistrationFailure | None | None |
| **DialogTimeout** | Both | Idle | - HandleRegistrationFailure | None | None |

### 20. **Registered** State
Successfully registered with SIP server.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **MakeCall** | UAC | Initiating | - CreateDialog<br>- CreateMediaSession<br>- GenerateLocalSDP<br>- SendINVITE<br>- UpdatePresenceBusy | None | HasMediaSession: true |
| **IncomingCall** | UAS | Ringing | - CreateMediaSession<br>- StoreRemoteSDP<br>- SendSIPResponse(180, "Ringing")<br>- NotifyUser | None | HasRemoteSDP: true<br>HasMediaSession: true |
| **RefreshRegistration** | Both | Registering | - SendREGISTER | None | None |
| **Unregister** | Both | Unregistering | - SendUnregister | None | None |
| **Subscribe** | Both | Subscribing | - SendSUBSCRIBE | None | None |
| **PublishPresence** | Both | Publishing | - SendPUBLISH | None | None |
| **SendMessage** | Both | Registered | - SendMESSAGE | None | None |
| **SendOptions** | Both | Registered | - SendOPTIONS | None | None |

### 21. **Unregistering** State
Unregistration in progress.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog200OK** | Both | Idle | - ClearRegistration | None | IsRegistered: false |
| **DialogTimeout** | Both | Idle | - ClearRegistration | None | IsRegistered: false |

### 22. **Subscribing** State
Subscription request in progress.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog200OK** | Both | Subscribed | - StoreSubscription | None | HasActiveSubscription: true |
| **Dialog4xxFailure** | Both | Registered | - HandleSubscriptionFailure | None | None |

### 23. **Subscribed** State
Active subscription for presence/events.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **DialogNOTIFY** | Both | Subscribed | - ProcessNotification<br>- Send200OK | None | None |
| **Unsubscribe** | Both | Registered | - SendUnsubscribe | None | HasActiveSubscription: false |
| **RefreshSubscription** | Both | Subscribing | - SendSUBSCRIBE | None | None |

### 24. **Publishing** State
Publishing presence information.

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **Dialog200OK** | Both | Registered | - StoreETag | None | None |
| **Dialog4xxFailure** | Both | Registered | - HandlePublishFailure | None | None |

### 25. **BridgeInitiating** State
Setting up B2BUA bridge (Gateway).

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **BridgeConnected** | Both | BridgeActive | - ConnectMediaStreams<br>- StartBridgeMonitoring | None | None |
| **BridgeFailed** | Both | Failed | - CleanupBothLegs<br>- SendErrorResponses | None | None |

### 26. **BridgeActive** State
B2BUA bridge established with both legs active (Gateway).

| Event | Role | Next State | Actions | Guards | Conditions Set |
|-------|------|------------|---------|--------|----------------|
| **InboundBYE** | Both | Terminating | - Send200OKToInbound<br>- SendBYEToOutbound | None | None |
| **OutboundBYE** | Both | Terminating | - Send200OKToOutbound<br>- SendBYEToInbound | None | None |
| **TranscodingRequired** | Both | BridgeActive | - SetupTranscoder<br>- UpdateMediaPath | None | TranscodingRequired: true |
| **BridgeFailed** | Both | BridgeInitiating | - AttemptBridgeRecovery<br>- NotifyEndpoints | None | None |

## Part 2: Event-Centric View (Which states can handle each event?)

### User-Initiated Events

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **MakeCall** | Idle, Registered | Initiate outbound call |
| **IncomingCall** | Idle, Registered | Handle incoming call |
| **AcceptCall** | Ringing | Accept incoming call |
| **RejectCall** | Ringing | Reject incoming call |
| **HangupCall** | Initiating, Ringing, EarlyMedia, Active, OnHold, Muted, ConferenceHost, InConference, ConsultationCall, Bridged, Queued, AgentRinging | End the call |
| **HoldCall** | Active, ConferenceHost | Put call on hold |
| **ResumeCall** | OnHold, ConferenceOnHold | Resume from hold |
| **MuteCall** | Active | Mute microphone |
| **UnmuteCall** | Muted | Unmute microphone |
| **BlindTransfer** | Active | Initiate blind transfer |
| **StartAttendedTransfer** | Active | Start attended transfer |

### Registration/Authentication Events

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **Register** | Idle | Start SIP registration |
| **RefreshRegistration** | Registered | Refresh registration |
| **Unregister** | Registered | Unregister from server |
| **Subscribe** | Registered | Subscribe to events |
| **Unsubscribe** | Subscribed | End subscription |
| **PublishPresence** | Registered | Publish presence state |
| **SendMessage** | Registered | Send instant message |
| **SendOptions** | Registered | Send OPTIONS request |

### Dialog Events (from dialog-core)

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **DialogInvite** | (Not used in current state table) | Incoming INVITE |
| **Dialog180Ringing** | Initiating | Remote party is ringing |
| **Dialog183SessionProgress** | Initiating, Ringing | Early media available |
| **Dialog200OK** | Initiating, Ringing, EarlyMedia, Resuming, Registering, Subscribing, Publishing, Unregistering | Call answered/operation succeeded |
| **DialogACK** | Active (for re-INVITE) | ACK received |
| **DialogBYE** | Active, OnHold, Muted, ConferenceHost, InConference | Remote hangup |
| **Dialog4xxFailure** | Initiating, Ringing, Registering, Subscribing, Publishing | Client error |
| **Dialog5xxFailure** | Initiating, Ringing, Registering | Server error |
| **DialogTimeout** | Initiating, Ringing, Registering, Unregistering | No response timeout |
| **DialogError** | Active | Protocol error |
| **DialogOPTIONS** | Any | OPTIONS received (keepalive/capabilities) |
| **DialogUPDATE** | Active | UPDATE received (mid-dialog modification) |
| **DialogINFO** | Active | INFO received (application signaling) |
| **DialogNOTIFY** | Subscribed | NOTIFY received (event notification) |
| **DialogMESSAGE** | Any | MESSAGE received (instant message) |
| **DialogREGISTER** | (Server only) | REGISTER received |
| **DialogSUBSCRIBE** | (Server only) | SUBSCRIBE received |
| **DialogPUBLISH** | (Server only) | PUBLISH received |

### Media Events (from media-core)

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **MediaSessionCreated** | (Handled internally) | Media session created |
| **MediaSessionReady** | (Handled internally) | Media ready to flow |
| **MediaFlowEstablished** | Active | RTP flowing |
| **MediaError** | Active | Media problem detected |

### Conference Events

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **CreateConference** | Active | Create conference from call |
| **AddParticipant** | ConferenceHost | Add participant to conference |
| **JoinConference** | Active | Join existing conference |
| **LeaveConference** | InConference | Leave conference |
| **MuteInConference** | InConference | Mute in conference |

### Transfer Events

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **CompleteAttendedTransfer** | ConsultationCall | Complete attended transfer |
| **TransferSuccess** | Transferring | Transfer succeeded |
| **TransferFailed** | Transferring | Transfer failed |

### Gateway/B2BUA Events

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **InitiateBridge** | Active | Start B2BUA bridge |
| **BridgeConnected** | BridgeInitiating | Bridge established |
| **BridgeFailed** | BridgeInitiating, BridgeActive | Bridge failure |
| **InboundBYE** | BridgeActive | Inbound leg hangup |
| **OutboundBYE** | BridgeActive | Outbound leg hangup |
| **TranscodingRequired** | BridgeActive | Need transcoding |

### Other Events

| Event | Valid States | Purpose |
|-------|-------------|---------|
| **SendDTMF** | Active, InConference | Send DTMF tones |
| **StartRecording** | Active, ConferenceHost | Start recording |
| **StopRecording** | Active | Stop recording |
| **InternalCleanupComplete** | Terminating | Cleanup finished |
| **Reset** | Terminated, Cancelled, Failed | Reset to idle |
| **SendUpdate** | Active | Send UPDATE request |
| **SendInfo** | Active | Send INFO request |
| **RefreshSubscription** | Subscribed | Refresh subscription |

## Part 3: Cross-Crate Event Mapping

### DialogToSessionEvent → Internal EventType

| Cross-Crate Event | Internal Event | Notes |
|-------------------|----------------|-------|
| DialogCreated | (Special handling - stores dialog mapping) | Maps dialog to session |
| IncomingCall | IncomingCall | New incoming call |
| CallEstablished | Dialog200OK | Call answered |
| CallRinging | Dialog180Ringing | Remote ringing |
| SessionProgress | Dialog183SessionProgress | Early media |
| CallTerminated | DialogBYE | Remote hangup |
| CallFailed | Dialog4xxFailure/5xxFailure | Call failed |
| DialogError | DialogError | Protocol error |
| RegistrationSuccess | Dialog200OK | Registration succeeded |
| RegistrationFailed | Dialog4xxFailure | Registration failed |
| SubscriptionActive | Dialog200OK | Subscription accepted |
| NotificationReceived | DialogNOTIFY | NOTIFY received |
| MessageReceived | DialogMESSAGE | MESSAGE received |
| OptionsReceived | DialogOPTIONS | OPTIONS received |
| UpdateReceived | DialogUPDATE | UPDATE received |
| InfoReceived | DialogINFO | INFO received |

### MediaToSessionEvent → Internal EventType  

| Cross-Crate Event | Internal Event | Notes |
|-------------------|----------------|-------|
| MediaStreamStarted | MediaSessionReady | Media ready |
| MediaFlowEstablished | MediaFlowEstablished | RTP flowing |
| MediaStreamStopped | MediaError | Media stopped |
| MediaError | MediaError | Media problem |
| MediaQualityDegraded | (Not yet mapped) | Should trigger quality event |
| DTMFReceived | (Not yet mapped) | Should trigger DTMF event |
| RTPTimeout | (Not yet mapped) | Should trigger timeout |

## Part 4: Action Categories

### Dialog Actions
- **CreateDialog**: Create SIP dialog
- **SendINVITE**: Send initial INVITE
- **Send200OK**: Send 200 OK response
- **SendACK**: Send ACK
- **SendBYE**: Send BYE to end call
- **SendCANCEL**: Cancel pending INVITE
- **SendReINVITE**: Send re-INVITE (hold/resume)
- **SendREFER**: Send REFER for transfer
- **SendSIPResponse**: Send generic SIP response
- **SendREGISTER**: Send REGISTER request
- **SendUnregister**: Send REGISTER with expires=0
- **SendSUBSCRIBE**: Send SUBSCRIBE request
- **SendUnsubscribe**: Send SUBSCRIBE with expires=0
- **SendNOTIFY**: Send NOTIFY
- **SendPUBLISH**: Send PUBLISH request
- **SendMESSAGE**: Send MESSAGE request
- **SendOPTIONS**: Send OPTIONS request
- **SendUPDATE**: Send UPDATE request
- **SendINFO**: Send INFO request

### Media Actions
- **CreateMediaSession**: Create media session
- **StartMediaSession**: Start media flow
- **StopMediaSession**: Stop media flow
- **StartMediaFlow**: Enable RTP
- **StopMediaFlow**: Disable RTP
- **GenerateLocalSDP**: Create local SDP
- **StoreRemoteSDP**: Store remote SDP
- **NegotiateSDPAsUAC**: Negotiate as caller
- **NegotiateSDPAsUAS**: Negotiate as callee
- **UpdateMediaDirection**: Change sendrecv/sendonly/recvonly
- **MuteLocalAudio**: Mute microphone
- **UnmuteLocalAudio**: Unmute microphone

### Conference Actions
- **CreateAudioMixer**: Create conference mixer
- **RedirectToMixer**: Route audio to mixer
- **ConnectToMixer**: Join conference
- **DisconnectFromMixer**: Leave conference
- **DestroyMixer**: Destroy conference
- **MuteToMixer**: Mute in conference
- **UnmuteToMixer**: Unmute in conference

### Resource Management Actions
- **StartDialogCleanup**: Clean dialog resources
- **StartMediaCleanup**: Clean media resources
- **CleanupResources**: General cleanup
- **ReleaseAllResources**: Final cleanup
- **StartEmergencyCleanup**: Error recovery

### State Management Actions
- **SetCondition**: Update condition flags
- **StoreNegotiatedConfig**: Save negotiated params
- **TriggerCallEstablished**: Notify call active
- **TriggerCallTerminated**: Notify call ended
- **StoreRegistration**: Save registration details
- **ClearRegistration**: Remove registration
- **StoreSubscription**: Save subscription details
- **StoreETag**: Save entity tag for PUBLISH

### Registration/Presence Actions
- **StartRegistrationRefreshTimer**: Schedule re-registration
- **HandleRegistrationFailure**: Process registration error
- **UpdatePresenceBusy**: Set presence to busy
- **UpdatePresenceAvailable**: Set presence to available
- **ProcessNotification**: Handle NOTIFY content
- **ProcessMessage**: Handle MESSAGE content
- **GenerateCapabilities**: Create OPTIONS response

### Call Center Actions
- **AddToQueue**: Add call to queue
- **RemoveFromQueue**: Remove from queue
- **RouteToAgent**: Route call to agent
- **ReleaseAgent**: Free agent for next call
- **BridgeToAgent**: Connect customer to agent
- **StartCallRecording**: Begin recording
- **SaveCallNotes**: Save wrap-up notes
- **UpdateAgentStats**: Update metrics
- **PlayQueueMusic**: Play hold music
- **NotifyAgent**: Alert agent of call

### Gateway/B2BUA Actions
- **CreateOutboundLeg**: Create second leg
- **ConnectMediaStreams**: Bridge media
- **SetupTranscoder**: Initialize transcoding
- **UpdateMediaPath**: Modify media route
- **AttemptBridgeRecovery**: Recover failed bridge
- **Send200OKToInbound**: Respond to inbound leg
- **SendBYEToOutbound**: Terminate outbound leg
- **CleanupBothLegs**: Clean both legs

## Key Insights

1. **State Constraints**: Most events are only valid in specific states, preventing invalid operations
2. **Role-Based Transitions**: UAC and UAS have different transitions for the same events
3. **Guard Conditions**: Some transitions require conditions (e.g., HasMediaSession) to proceed
4. **Atomic Actions**: Each transition executes a sequence of actions atomically
5. **Event-Driven Flow**: External events (SIP, media) drive all state changes
6. **Condition Tracking**: The state machine tracks readiness conditions to coordinate async operations

This mapping is essential for:
- Implementing proper event routing
- Debugging state machine behavior
- Understanding call flow scenarios
- Ensuring event handlers trigger correct state transitions

## TODO: Missing Elements to Add

### Missing States
1. **Proceeding** - After receiving 100 Trying, before 180/183
2. **Redirecting** - Handling 3xx redirect responses  
3. **Authenticating** - Handling 401/407 authentication challenges (partially implemented for gateway)
4. **Refreshing** - Session timer refresh in progress
5. **WaitingForPRACK** - Waiting for PRACK for reliable provisional responses (RFC 3262)
6. **Replaced** - Session being replaced (attended transfer completion)
7. **Forking** - Handling multiple provisional responses (partially implemented for gateway)

### Missing Events

#### Dialog Events (from dialog-core) - Still Missing
- **Dialog100Trying** - Call is being processed
- **Dialog3xxRedirect** - Call redirected to new destination
- **Dialog401Unauthorized** - Authentication required
- **Dialog407ProxyAuthRequired** - Proxy authentication required
- **DialogPRACK** - PRACK received (RFC 3262)
- **SessionTimerExpired** - Session timer needs refresh (RFC 4028)
- **AuthenticationChallenge** - Need to authenticate
- **AuthenticationSuccess** - Authentication completed
- **Dialog202Accepted** - Asynchronous request accepted
- **DialogForked** - Multiple provisional responses received

#### Media Events (from media-core)
- **MediaCodecChanged** - Codec renegotiation occurred
- **MediaPacketLoss** - Significant packet loss detected
- **MediaJitterHigh** - High jitter detected
- **MediaLatencyHigh** - High latency detected
- **ICEStateChanged** - ICE connection state changed
- **ICEGatheringComplete** - ICE candidates gathered
- **DTMFStarted** - DTMF tone started
- **DTMFEnded** - DTMF tone ended
- **SilenceDetected** - Voice activity stopped
- **VoiceDetected** - Voice activity resumed
- **MediaQualityRecovered** - Media quality improved
- **RTPTimeout** - No RTP packets received

#### Session Management Events - Still Missing
- **SessionTimerWarning** - Session about to expire
- **ForkedResponse** - Multiple responses received (forking)
- **RegistrationExpiring** - Registration about to expire
- **SubscriptionExpiring** - Subscription needs refresh
- **AuthenticationRequired** - Need to provide credentials

### Missing Actions

#### Dialog Actions - Still Missing
- **SendPRACK** - Send PRACK for reliable provisional response
- **SendAuthResponse** - Send authentication response
- **ProcessRedirect** - Handle 3xx redirect
- **StartSessionTimer** - Initialize session timer
- **RefreshSession** - Send session refresh
- **CancelSessionTimer** - Stop session timer
- **HandleForkedResponse** - Process multiple provisional responses
- **Send100Trying** - Send 100 Trying response

#### Media Actions
- **RenegotiateCodecs** - Change audio codecs
- **EnableDTMFDetection** - Start DTMF detection
- **DisableDTMFDetection** - Stop DTMF detection
- **EnableVAD** - Enable voice activity detection
- **DisableVAD** - Disable voice activity detection
- **AdjustJitterBuffer** - Modify jitter buffer size
- **EnablePacketLossConcealment** - Enable PLC
- **RequestKeyFrame** - Request video keyframe (future)
- **UpdateICECandidates** - Add new ICE candidates
- **RestartICE** - Restart ICE negotiation

#### Session Management Actions
- **InitiateRegistration** - Start SIP registration
- **HandleAuthentication** - Process auth challenge
- **StoreAuthCredentials** - Save auth info
- **ProcessForkedResponses** - Handle multiple responses
- **SelectBestResponse** - Choose from forked responses

### Missing Cross-Crate Events

These events should be added to DialogToSessionEvent and MediaToSessionEvent enums:

#### DialogToSessionEvent (should be added)
- **RegistrationRequired** - Dialog-core detects registration needed
- **AuthenticationChallenge** - 401/407 received
- **RedirectReceived** - 3xx response received
- **PRACKRequired** - Reliable provisional response needs PRACK
- **SessionTimerExpired** - Session needs refresh
- **ForkingDetected** - Multiple provisional responses

#### MediaToSessionEvent (should be added)  
- **CodecNegotiationFailed** - Incompatible codecs
- **TranscodingRequired** - Need to transcode between codecs
- **MediaRecovered** - Media quality restored
- **VADStateChanged** - Voice activity detection state change
- **JitterBufferUnderrun** - Jitter buffer empty
- **JitterBufferOverrun** - Jitter buffer overflow

### Implementation Priority

1. **High Priority** (Basic SIP compliance):
   - Authentication support (401/407) 
   - Session timers (RFC 4028)
   - 100 Trying handling
   - Cross-crate event mappings for new features

2. **Medium Priority** (Enhanced functionality):
   - Reliable provisional responses (RFC 3262)
   - Media quality monitoring improvements
   - DTMF detection improvements
   - Voice activity detection

3. **Low Priority** (Advanced features):
   - Call forking support (partially done)
   - Full B2BUA state management
   - Advanced media features
   - Performance optimizations

### Notes

- Some events may require new CrossCrateEvent types in infra-common
- Media events need better granularity than current "MediaError"
- Authentication should be coordinated with dialog-core
- Session timers are critical for carrier compliance
- Consider if all states need to handle "Reset" event for error recovery

## Summary of Recent Additions

This document has been updated to include the following new state machine features:

### New States Added (8 total)
- **Registration**: Registering, Registered, Unregistering
- **Presence/Events**: Subscribing, Subscribed, Publishing
- **Gateway/B2BUA**: BridgeInitiating, BridgeActive

### New Events Supported
- **SIP Methods**: REGISTER, OPTIONS, UPDATE, INFO, SUBSCRIBE, NOTIFY, MESSAGE, PUBLISH
- **Registration**: Register, RefreshRegistration, Unregister
- **Presence**: Subscribe, Unsubscribe, PublishPresence, SendMessage
- **Gateway**: InitiateBridge, BridgeConnected, TranscodingRequired

### New Actions Added
- **Dialog**: SendREGISTER, SendSUBSCRIBE, SendNOTIFY, SendMESSAGE, SendOPTIONS, SendUPDATE, SendINFO
- **Registration**: StoreRegistration, StartRegistrationRefreshTimer, HandleRegistrationFailure
- **Gateway**: CreateOutboundLeg, SetupTranscoder, AttemptBridgeRecovery

### Use Case Support
- **SIP Clients**: Full registration, presence, and messaging support
- **Gateways**: B2BUA operations, transcoding, and load balancing

The state machine now provides comprehensive coverage for production SIP deployments across multiple use cases.
