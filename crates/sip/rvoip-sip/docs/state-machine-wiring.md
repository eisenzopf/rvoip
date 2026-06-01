# State Machine Wiring Manifest

Generated from `rvoip_sip::state_table::wiring_manifest`. Do not edit by hand; update the Rust manifest and rerun the drift test.

| SIP/API message | Caller/source | infra-common event | rvoip-sip event | YAML event | Actions/call path | Wiring | Notes |
|---|---|---|---|---|---|---|---|
| INVITE outbound | InviteBuilder::send / UnifiedCoordinator::invite | n/a | SendOutboundInvite / MakeCall | SendOutboundInvite / MakeCall | CreateDialog, CreateMediaSession, GenerateLocalSDP, SendINVITEWithOptions | state-table | Initial INVITE owns session lifecycle and media setup. |
| re-INVITE outbound | ReinviteBuilder::send | n/a | SendOutboundReInvite | SendOutboundReInvite | SendReINVITEWithOptions | state-table | In-dialog request, state remains Active. |
| UPDATE outbound | UpdateBuilder::send | n/a | SendOutboundUpdate | SendOutboundUpdate | SendUPDATEWithOptions | state-table | In-dialog request, state remains Active. |
| BYE outbound | ByeBuilder::send / SessionHandle::hangup | n/a | SendOutboundBye / HangupCall | SendOutboundBye / HangupCall | SendBYEWithOptions, SendBYE | state-table | Local hangup uses BYE once the dialog is established. |
| BYE inbound | rvoip-sip-dialog bye_handler | DialogToSessionEvent::ByeReceived | DialogBYE | DialogBYE | CleanupDialog, CleanupMedia | state-table | Dialog-core sends the 200 OK; state machine only cleans up. |
| CANCEL outbound | CancelBuilder::send / pending hangup | n/a | SendOutboundCancel / CancelCall | SendOutboundCancel / CancelCall | SendCANCELWithOptions | state-table | Only legal after an INVITE provisional response. |
| CANCEL inbound | rvoip-sip-dialog protocol_handlers | DialogToSessionEvent::CallCancelled | Dialog487RequestTerminated / DialogCANCEL | Dialog487RequestTerminated / DialogCANCEL | CleanupDialog, CleanupMedia | state-table | Dialog-core owns wire responses; state machine publishes cancellation. |
| REFER outbound | ReferBuilder::send | n/a | SendOutboundRefer | SendOutboundRefer | SendREFERWithOptions | state-table | In-dialog request, state remains Active. |
| NOTIFY outbound | NotifyBuilder::send / transfer progress | n/a | SendOutboundNotify | SendOutboundNotify | SendNOTIFYWithOptions | state-table | In-dialog NOTIFY and REFER progress NOTIFYs. |
| INFO outbound | InfoBuilder::send | n/a | SendOutboundInfo | SendOutboundInfo | SendINFOWithOptions | state-table | In-dialog INFO request. |
| REGISTER | RegisterBuilder::send / RegisterRefreshBuilder::send | DialogToSessionEvent::AuthRequired | StartRegistration / SendOutboundRegister / RefreshRegistration | StartRegistration / SendOutboundRegister / RefreshRegistration | SendREGISTER, SendREGISTERWithOptions, SendREGISTERWithAuth | state-table | Registration owns lifecycle, auth, 423 retry, refresh, and unregister state. |
| MESSAGE out-of-dialog | MessageBuilder::send | n/a | n/a | n/a | DialogAdapter::send_message_oob_with_options | direct | Public OOB MESSAGE bypasses the state table by design. |
| MESSAGE in-dialog inbound | rvoip-sip-dialog event_hub | DialogToSessionEvent::MessageReceived | n/a | n/a | publish Event::MessageReceived | direct | Delivered as an application event; no session lifecycle transition. |
| OPTIONS out-of-dialog | OptionsBuilder::send | n/a | n/a | n/a | DialogAdapter::send_options_oob_with_options | direct | Public OOB OPTIONS bypasses the state table by design. |
| OPTIONS inbound | rvoip-sip-dialog event_hub | DialogToSessionEvent::OptionsReceived | n/a | n/a | publish Event::OptionsReceived | direct | Capability query event; in-dialog mapping is preserved when dialog-core supplies a session id. |
| SUBSCRIBE | SubscribeBuilder::send / SubscribeRefreshBuilder::send | DialogToSessionEvent::NotifyReceived | n/a | n/a | DialogAdapter::send_subscribe_oob_with_options | dialog-managed | Subscription sends are direct-wired; NOTIFY delivery is handled by dialog-core/session events. |
| PUBLISH | rvoip-sip-dialog presence::PublishBuilder | n/a | n/a | n/a | deferred | deferred | Presence publish still has a placeholder dialog-core path and no live rvoip-sip state-table row. |
| Bridge | UnifiedCoordinator::bridge / SipBridgeStrategy | n/a | n/a | n/a | MediaAdapter::bridge_rtp_sessions | direct | Bridge is media/core direct wiring, not a SIP state-table transition. |
| Transport inbound message | rvoip-sip-transport live transports | n/a | n/a | n/a | TransportEvent::MessageReceived | transport-only | Transport preserves typed SIP Message plus raw bytes for dialog transaction handling. |
