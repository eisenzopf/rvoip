use crate::data_channel::message::RTCDataChannelMessage;
use crate::peer_connection::event::RTCEventInternal;
use crate::peer_connection::event::RTCPeerConnectionEvent;
use crate::peer_connection::event::data_channel_event::RTCDataChannelEvent;
use crate::peer_connection::message::internal::{
    ApplicationMessage, DTLSMessage, DataChannelEvent, RTCMessageInternal, RTPMessage,
    TaggedRTCMessageInternal, TrackPacket,
};

use crate::media_stream::track::MediaStreamTrackId;
use crate::peer_connection::configuration::media_engine::MediaEngine;
use crate::peer_connection::event::track_event::{RTCTrackEvent, RTCTrackEventInit};
use crate::rtp_transceiver::rtp_receiver::internal::RTCRtpReceiverInternal;
use crate::rtp_transceiver::rtp_sender::{
    RTCRtpCodingParameters, RTCRtpHeaderExtensionCapability, RtpCodecKind,
};
use crate::rtp_transceiver::{
    PayloadType, RTCRtpReceiverId, SSRC, internal::RTCRtpTransceiverInternal,
};
use crate::statistics::accumulator::RTCStatsAccumulator;
use interceptor::{Interceptor, Packet};
use log::{debug, trace, warn};
use shared::TransportContext;
use shared::error::{Error, Result};
use shared::marshal::MarshalSize;
use std::collections::VecDeque;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct EndpointHandlerContext {
    pub(crate) read_outs: VecDeque<TaggedRTCMessageInternal>,
    pub(crate) write_outs: VecDeque<TaggedRTCMessageInternal>,
    pub(crate) event_outs: VecDeque<RTCEventInternal>,
}

/// EndpointHandler implements DataChannel/Media Endpoint handling
/// The transmits queue is now stored in RTCPeerConnection and passed by reference
pub(crate) struct EndpointHandler<'a, I>
where
    I: Interceptor,
{
    ctx: &'a mut EndpointHandlerContext,
    rtp_transceivers: &'a mut Vec<RTCRtpTransceiverInternal<I>>,
    media_engine: &'a MediaEngine,
    interceptor: &'a mut I,
    stats: &'a mut RTCStatsAccumulator,
}

/// Select the receiver that can own an un-signaled SSRC. An explicit MID is
/// authoritative for every media kind. Without a MID, payload type may select
/// a unique audio receiver so codec-distinct supplemental audio (notably RFC
/// 4733) can join its primary stream. The historical sole-transceiver fallback
/// remains available for audio or video when that receiver has no RID codings.
///
/// This deliberately refuses ambiguous ownership instead of routing media to
/// whichever transceiver happens to appear first.
fn unique_receiver_index(mut matches: impl Iterator<Item = usize>) -> Option<usize> {
    let selected = matches.next()?;
    matches.next().is_none().then_some(selected)
}

fn undeclared_receiver_index<I: Interceptor>(
    rtp_transceivers: &[RTCRtpTransceiverInternal<I>],
    payload_type: PayloadType,
    signaled_mid: Option<&str>,
) -> Option<usize> {
    let signaled_mid = signaled_mid.filter(|mid| !mid.is_empty());
    let matches_payload = |transceiver: &RTCRtpTransceiverInternal<I>| {
        transceiver.direction().has_recv()
            && !transceiver.stopped()
            && transceiver.receiver().as_ref().is_some_and(|receiver| {
                receiver
                    .get_codec_preferences()
                    .iter()
                    .any(|codec| codec.payload_type == payload_type)
            })
    };
    if let Some(mid) = signaled_mid {
        return unique_receiver_index(
            rtp_transceivers
                .iter()
                .enumerate()
                .filter(|(_, transceiver)| {
                    transceiver.mid().as_deref() == Some(mid) && matches_payload(transceiver)
                })
                .map(|(index, _)| index),
        );
    }

    if let Some(index) = unique_receiver_index(
        rtp_transceivers
            .iter()
            .enumerate()
            .filter(|(_, transceiver)| {
                transceiver.kind() == RtpCodecKind::Audio && matches_payload(transceiver)
            })
            .map(|(index, _)| index),
    ) {
        return Some(index);
    }

    let [transceiver] = rtp_transceivers else {
        return None;
    };
    (matches_payload(transceiver)
        && transceiver
            .receiver()
            .as_ref()
            .is_some_and(|receiver| receiver.track().codings().is_empty()))
    .then_some(0)
}

/// Add an un-signaled SSRC without discarding the receiver's SDP-declared
/// primary, RTX, FEC, or earlier supplemental codings. Returns whether a new
/// coding was inserted so retransmitted packets reuse the existing route.
fn append_undeclared_coding(receive_codings: &mut Vec<RTCRtpCodingParameters>, ssrc: SSRC) -> bool {
    if receive_codings
        .iter()
        .any(|coding| coding.ssrc == Some(ssrc))
    {
        return false;
    }
    receive_codings.push(RTCRtpCodingParameters {
        rid: String::new(),
        ssrc: Some(ssrc),
        rtx: None,
        fec: None,
    });
    true
}

impl<'a, I> EndpointHandler<'a, I>
where
    I: Interceptor,
{
    pub(crate) fn new(
        ctx: &'a mut EndpointHandlerContext,
        rtp_transceivers: &'a mut Vec<RTCRtpTransceiverInternal<I>>,
        media_engine: &'a MediaEngine,
        interceptor: &'a mut I,
        stats: &'a mut RTCStatsAccumulator,
    ) -> Self {
        EndpointHandler {
            ctx,
            rtp_transceivers,
            media_engine,
            interceptor,
            stats,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        "EndpointHandler"
    }
}

// Implement Protocol trait for message processing
impl<'a, I> sansio::Protocol<TaggedRTCMessageInternal, TaggedRTCMessageInternal, RTCEventInternal>
    for EndpointHandler<'a, I>
where
    I: Interceptor,
{
    type Rout = TaggedRTCMessageInternal;
    type Wout = TaggedRTCMessageInternal;
    type Eout = RTCEventInternal;
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedRTCMessageInternal) -> Result<()> {
        match msg.message {
            RTCMessageInternal::Dtls(DTLSMessage::DataChannel(message)) => {
                self.handle_dtls_message(msg.now, msg.transport, message)
            }
            RTCMessageInternal::Rtp(RTPMessage::Packet(Packet::Rtp(message))) => {
                self.handle_rtp_message(msg.now, msg.transport, message)
            }
            RTCMessageInternal::Rtp(RTPMessage::Packet(Packet::Rtcp(message))) => {
                self.handle_rtcp_message(msg.now, msg.transport, message)
            }
            _ => {
                warn!("drop unsupported message from {}", msg.transport.peer_addr);
                Ok(())
            }
        }
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        self.ctx.read_outs.pop_front()
    }

    fn handle_write(&mut self, msg: TaggedRTCMessageInternal) -> Result<()> {
        self.ctx.write_outs.push_back(msg);
        Ok(())
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        self.ctx.write_outs.pop_front()
    }

    fn handle_event(&mut self, evt: RTCEventInternal) -> Result<()> {
        self.ctx.event_outs.push_back(evt);
        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        self.ctx.event_outs.pop_front()
    }

    fn handle_timeout(&mut self, _now: Instant) -> Result<()> {
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Instant> {
        None
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<'a, I> EndpointHandler<'a, I>
where
    I: Interceptor,
{
    fn handle_dtls_message(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        message: ApplicationMessage,
    ) -> Result<()> {
        match message.data_channel_event {
            DataChannelEvent::Open => {
                self.handle_datachannel_open(now, transport_context, message.data_channel_id)
            }
            DataChannelEvent::Message(data_channel_message) => self.handle_datachannel_message(
                now,
                transport_context,
                message.data_channel_id,
                data_channel_message,
            ),
            DataChannelEvent::Close => {
                self.handle_datachannel_close(now, transport_context, message.data_channel_id)
            }
        }
    }

    fn handle_rtp_message(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        rtp_packet: rtp::Packet,
    ) -> Result<()> {
        debug!("handle_rtp_message {}", transport_context.peer_addr);

        let ssrc = rtp_packet.header.ssrc;

        if let Some(track_id) = self.find_track_id(ssrc, Some(&rtp_packet.header)) {
            // Track RTP stats if accumulator exists (created when OnOpen event is fired)
            if let Some(stream) = self.stats.inbound_rtp_streams.get_mut(&ssrc) {
                stream.on_rtp_received(
                    rtp_packet.header.marshal_size(),
                    rtp_packet.payload.len(),
                    now,
                );
            }

            self.ctx.read_outs.push_back(TaggedRTCMessageInternal {
                now,
                transport: transport_context,
                message: RTCMessageInternal::Rtp(RTPMessage::TrackPacket(TrackPacket {
                    track_id,
                    packet: Packet::Rtp(rtp_packet),
                })),
            });
        } else {
            debug!("drop rtp packet ssrc = {}", ssrc);
        }
        Ok(())
    }

    fn handle_rtcp_message(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        rtcp_packets: Vec<Box<dyn rtcp::Packet>>,
    ) -> Result<()> {
        debug!("handle_rtcp_message {}", transport_context.peer_addr);

        let rtcp_ssrc = if let Some(rtcp_packet) = rtcp_packets.first() {
            rtcp_packet.destination_ssrc().first().cloned()
        } else {
            None
        };

        if let Some(rtcp_ssrc) = rtcp_ssrc {
            if let Some(track_id) = self.find_track_id(rtcp_ssrc, None) {
                self.ctx.read_outs.push_back(TaggedRTCMessageInternal {
                    now,
                    transport: transport_context,
                    message: RTCMessageInternal::Rtp(RTPMessage::TrackPacket(TrackPacket {
                        track_id,
                        packet: Packet::Rtcp(rtcp_packets),
                    })),
                });
            } else {
                debug!("drop rtcp packet ssrc = {}", rtcp_ssrc);
            }
        } else {
            debug!("drop rtcp packet due to empty ssrc");
        }

        Ok(())
    }

    fn handle_datachannel_open(
        &mut self,
        _now: Instant,
        transport_context: TransportContext,
        data_channel_id: u16,
    ) -> Result<()> {
        debug!("data channel is open for {:?}", transport_context);
        self.ctx
            .event_outs
            .push_back(RTCEventInternal::RTCPeerConnectionEvent(
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnOpen(data_channel_id)),
            ));

        Ok(())
    }

    fn handle_datachannel_close(
        &mut self,
        _now: Instant,
        transport_context: TransportContext,
        data_channel_id: u16,
    ) -> Result<()> {
        debug!("data channel is close for {:?}", transport_context);
        self.ctx
            .event_outs
            .push_back(RTCEventInternal::RTCPeerConnectionEvent(
                RTCPeerConnectionEvent::OnDataChannel(RTCDataChannelEvent::OnClose(
                    data_channel_id,
                )),
            ));

        Ok(())
    }

    fn handle_datachannel_message(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        data_channel_id: u16,
        data_channel_message: RTCDataChannelMessage,
    ) -> Result<()> {
        debug!("data channel recv message for {:?}", transport_context);
        self.ctx.read_outs.push_back(TaggedRTCMessageInternal {
            now,
            transport: transport_context,
            message: RTCMessageInternal::Dtls(DTLSMessage::DataChannel(ApplicationMessage {
                data_channel_id,
                data_channel_event: DataChannelEvent::Message(data_channel_message),
            })),
        });

        Ok(())
    }

    // crosscheck with RTCPeerConnection::start_rtp, since remote tracks(RTCRtpCodingParameters) are added in it
    fn find_track_id(
        &mut self,
        ssrc: SSRC,
        rtp_header: Option<&rtp::Header>,
    ) -> Option<MediaStreamTrackId> {
        if let Some(track_id) = self.find_track_id_by_ssrc(ssrc, rtp_header) {
            Some(track_id)
        } else if let Some(rtp_header) = rtp_header // rid search only for RTP packet
            && let Some(track_id) = self.find_track_id_by_rid(ssrc, rtp_header)
        {
            Some(track_id)
        } else {
            None
        }
    }

    fn find_track_id_by_ssrc(
        &mut self,
        ssrc: SSRC,
        rtp_header: Option<&rtp::Header>,
    ) -> Option<MediaStreamTrackId> {
        if let Some((id, transceiver)) =
            self.rtp_transceivers
                .iter_mut()
                .enumerate()
                .find(|(_, transceiver)| {
                    if let Some(receiver) = transceiver.receiver() {
                        receiver.get_coding_parameters().iter().any(|coding| {
                            coding.ssrc.is_some_and(|coding_ssrc| coding_ssrc == ssrc)
                        })
                    } else {
                        false
                    }
                })
        {
            // Get kind and mid before borrowing receiver mutably
            let kind = transceiver.kind();
            let mid = transceiver.mid().clone().unwrap_or_default();

            if let Some(receiver) = transceiver.receiver_mut()
                && receiver
                    .track()
                    .ssrcs()
                    .any(|track_ssrc| track_ssrc == ssrc)
            {
                let (is_track_codec_empty, track_id) = (
                    receiver
                        .track()
                        .get_codec_by_ssrc(ssrc)
                        .is_some_and(|codec| codec.mime_type.is_empty()),
                    receiver.track().track_id().clone(),
                );

                let track_codec = if is_track_codec_empty
                    && let Some(rtp_header) = rtp_header
                    && let Some(codec) = receiver
                        .get_codec_preferences()
                        .iter()
                        .find(|codec| codec.payload_type == rtp_header.payload_type)
                //TODO: what about RTX/FEC stream?
                {
                    Some(codec.rtp_codec.clone())
                } else {
                    None
                };

                if let Some(codec) = track_codec {
                    // Set valid Codec for track when received the first RTP packet for such ssrc stream
                    // assert not inserting new entry
                    let new_entry = receiver.track_mut().set_codec_by_ssrc(codec, ssrc);
                    assert!(!new_entry);

                    // Get RTX and FEC SSRCs from coding parameters
                    let (rtx_ssrc, fec_ssrc) = receiver
                        .get_coding_parameters()
                        .iter()
                        .find(|c| c.ssrc == Some(ssrc))
                        .map(|c| {
                            (
                                c.rtx.as_ref().map(|r| r.ssrc),
                                c.fec.as_ref().map(|f| f.ssrc),
                            )
                        })
                        .unwrap_or((None, None));

                    // Create inbound stream accumulator before firing OnOpen event
                    self.stats.get_or_create_inbound_rtp_streams(
                        ssrc, kind, &track_id, &mid, rtx_ssrc, fec_ssrc, id,
                    );

                    // Fire RTCTrackEvent::OnOpen event when received the first RTP packet for such ssrc stream
                    self.ctx
                        .event_outs
                        .push_back(RTCEventInternal::RTCPeerConnectionEvent(
                            RTCPeerConnectionEvent::OnTrack(RTCTrackEvent::OnOpen(
                                RTCTrackEventInit {
                                    receiver_id: RTCRtpReceiverId(id),
                                    track_id: receiver.track().track_id().to_owned(),
                                    stream_ids: vec![receiver.track().stream_id().to_owned()],
                                    ssrc,
                                    rid: None,
                                },
                            )),
                        ));
                }

                return Some(track_id);
            }
        }

        trace!(
            "no track id for {:?} for {}",
            ssrc,
            if rtp_header.is_some() {
                "RTP packet, let's try search rid"
            } else {
                "RTCP packet"
            }
        );
        None
    }

    fn find_track_id_by_rid(
        &mut self,
        ssrc: SSRC,
        rtp_header: &rtp::Header,
    ) -> Option<MediaStreamTrackId> {
        let (mid, rid, rrid) = self
            .get_rtp_header_extension_ids(rtp_header)
            .unwrap_or_default();

        // RFC 8834 requires WebRTC receivers to accept SSRCs that were not
        // signaled in SDP. Codec-distinct supplemental audio (notably RFC
        // 4733) has no RID, so bind it to an explicit MID when available or
        // to the unique audio receiver that negotiated its payload type. The
        // same path retains the sole-transceiver fallback for un-signaled
        // video RTP without header extensions.
        if rid.is_empty() && rrid.is_empty() {
            return self
                .handle_undeclared_ssrc(rtp_header, (!mid.is_empty()).then_some(mid.as_str()));
        }
        if mid.is_empty() {
            return None;
        }

        // If rtp header extension has valid mid, find receiver based on mid, instead of rid,
        // since rid is not unique across m= lines
        if let Some((id, transceiver)) =
            self.rtp_transceivers
                .iter_mut()
                .enumerate()
                .find(|(_, transceiver)| {
                    transceiver
                        .mid()
                        .as_deref()
                        .is_some_and(|t_mid| t_mid == mid)
                })
        {
            // Get kind before borrowing receiver mutably
            let kind = transceiver.kind();

            if let Some(receiver) = transceiver.receiver_mut()
                && let Some(codec) = receiver
                    .get_codec_preferences()
                    .iter()
                    .find(|codec| codec.payload_type == rtp_header.payload_type) //TODO: what about RTX/FEC stream?
                    .cloned()
            {
                if !rrid.is_empty() {
                    //TODO: Add support of handling repair rtp stream id (rrid) #12
                } else {
                    if let Some(coding) = receiver.get_coding_parameter_mut_by_rid(rid.as_str()) {
                        coding.ssrc = Some(ssrc);
                    }

                    let parameters = receiver.get_parameters(self.media_engine);
                    RTCRtpReceiverInternal::interceptor_remote_stream_op(
                        self.interceptor,
                        true,
                        rtp_header.ssrc,
                        codec.payload_type,
                        &codec.rtp_codec,
                        &parameters.rtp_parameters.header_extensions,
                    );

                    let new_entry =
                        receiver
                            .track_mut()
                            .set_codec_ssrc_by_rid(codec.rtp_codec, ssrc, &rid);
                    assert!(!new_entry);

                    let track_id = receiver.track().track_id().to_owned();

                    // Get RTX and FEC SSRCs from coding parameters
                    let (rtx_ssrc, fec_ssrc) = receiver
                        .get_coding_parameters()
                        .iter()
                        .find(|c| c.ssrc == Some(ssrc))
                        .map(|c| {
                            (
                                c.rtx.as_ref().map(|r| r.ssrc),
                                c.fec.as_ref().map(|f| f.ssrc),
                            )
                        })
                        .unwrap_or((None, None));

                    // Create inbound stream accumulator before firing OnOpen event
                    self.stats.get_or_create_inbound_rtp_streams(
                        ssrc, kind, &track_id, &mid, rtx_ssrc, fec_ssrc, id,
                    );

                    // Fire RTCTrackEvent::OnOpen event when received the first RTP packet for such ssrc stream
                    self.ctx
                        .event_outs
                        .push_back(RTCEventInternal::RTCPeerConnectionEvent(
                            RTCPeerConnectionEvent::OnTrack(RTCTrackEvent::OnOpen(
                                RTCTrackEventInit {
                                    receiver_id: RTCRtpReceiverId(id),
                                    track_id: track_id.clone(),
                                    stream_ids: vec![receiver.track().stream_id().to_owned()],
                                    ssrc,
                                    rid: Some(rid),
                                },
                            )),
                        ));
                    return Some(track_id);
                }
            }
        }
        None
    }

    fn handle_undeclared_ssrc(
        &mut self,
        rtp_header: &rtp::Header,
        signaled_mid: Option<&str>,
    ) -> Option<MediaStreamTrackId> {
        let receiver_index = undeclared_receiver_index(
            self.rtp_transceivers,
            rtp_header.payload_type,
            signaled_mid,
        )?;
        let transceiver = &mut self.rtp_transceivers[receiver_index];
        let kind = transceiver.kind();
        let mid = transceiver.mid().clone().unwrap_or_default();
        let receiver = transceiver.receiver_mut().as_mut()?;
        let codec = receiver
            .get_codec_preferences()
            .iter()
            .find(|codec| codec.payload_type == rtp_header.payload_type)
            .cloned()?;

        let mut receive_codings = receiver.get_coding_parameters().to_vec();
        if !append_undeclared_coding(&mut receive_codings, rtp_header.ssrc) {
            // The normal SSRC lookup owns established routes. This branch is
            // defensive against a repeated packet racing route publication.
            return Some(receiver.track().track_id().to_owned());
        }
        receiver.set_coding_parameters(receive_codings);

        let parameters = receiver.get_parameters(self.media_engine);
        RTCRtpReceiverInternal::interceptor_remote_stream_op(
            self.interceptor,
            true,
            rtp_header.ssrc,
            codec.payload_type,
            &codec.rtp_codec,
            &parameters.rtp_parameters.header_extensions,
        );

        let new_entry = receiver
            .track_mut()
            .set_codec_by_ssrc(codec.rtp_codec, rtp_header.ssrc);
        if !new_entry {
            return Some(receiver.track().track_id().to_owned());
        }

        let track_id = receiver.track().track_id().to_owned();
        self.stats.get_or_create_inbound_rtp_streams(
            rtp_header.ssrc,
            kind,
            &track_id,
            &mid,
            None,
            None,
            receiver_index,
        );
        self.ctx
            .event_outs
            .push_back(RTCEventInternal::RTCPeerConnectionEvent(
                RTCPeerConnectionEvent::OnTrack(RTCTrackEvent::OnOpen(RTCTrackEventInit {
                    receiver_id: RTCRtpReceiverId(receiver_index),
                    track_id: track_id.clone(),
                    stream_ids: vec![receiver.track().stream_id().to_owned()],
                    ssrc: rtp_header.ssrc,
                    rid: None,
                })),
            ));
        Some(track_id)
    }

    fn get_rtp_header_extension_ids(
        &self,
        rtp_header: &rtp::Header,
    ) -> Option<(String, String, String)> {
        if !rtp_header.extension {
            return None;
        }

        // Get MID extension ID
        let (mid_extension_id, audio_supported, video_supported) = self
            .media_engine
            .get_header_extension_id(RTCRtpHeaderExtensionCapability {
                uri: ::sdp::extmap::SDES_MID_URI.to_owned(),
            });
        if !audio_supported && !video_supported {
            return None;
        }

        // Get RID extension ID
        let (rid_extension_id, audio_supported, video_supported) = self
            .media_engine
            .get_header_extension_id(RTCRtpHeaderExtensionCapability {
                uri: ::sdp::extmap::SDES_RTP_STREAM_ID_URI.to_owned(),
            });
        let rid_supported = audio_supported || video_supported;

        // Get RRID extension ID
        let (rrid_extension_id, rrid_audio_supported, rrid_video_supported) = self
            .media_engine
            .get_header_extension_id(RTCRtpHeaderExtensionCapability {
                uri: ::sdp::extmap::SDES_REPAIR_RTP_STREAM_ID_URI.to_owned(),
            });
        let rrid_supported = rrid_audio_supported || rrid_video_supported;

        let mid = if let Some(payload) = rtp_header.get_extension(mid_extension_id as u8) {
            String::from_utf8(payload.to_vec()).unwrap_or_default()
        } else {
            String::new()
        };

        let rid = if rid_supported
            && let Some(payload) = rtp_header.get_extension(rid_extension_id as u8)
        {
            String::from_utf8(payload.to_vec()).unwrap_or_default()
        } else {
            String::new()
        };

        let rrid = if rrid_supported
            && let Some(payload) = rtp_header.get_extension(rrid_extension_id as u8)
        {
            String::from_utf8(payload.to_vec()).unwrap_or_default()
        } else {
            String::new()
        };

        Some((mid, rid, rrid))
    }
}

#[cfg(test)]
mod undeclared_ssrc_tests {
    use super::{
        RTCRtpCodingParameters, RTCRtpTransceiverInternal, RtpCodecKind, append_undeclared_coding,
        undeclared_receiver_index,
    };
    use crate::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters};
    use crate::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
    use interceptor::NoopInterceptor;

    fn receiver(
        kind: RtpCodecKind,
        mid: &str,
        payload_types: &[u8],
    ) -> RTCRtpTransceiverInternal<NoopInterceptor> {
        let mut transceiver = RTCRtpTransceiverInternal::new(
            kind,
            None,
            RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                ..Default::default()
            },
        );
        transceiver.set_mid(mid.to_owned()).expect("unique MID");
        transceiver
            .receiver_mut()
            .as_mut()
            .expect("receive-capable transceiver")
            .set_codec_preferences(
                payload_types
                    .iter()
                    .copied()
                    .map(|payload_type| RTCRtpCodecParameters {
                        rtp_codec: RTCRtpCodec {
                            mime_type: match kind {
                                RtpCodecKind::Audio if payload_type == 111 => {
                                    "audio/opus".to_owned()
                                }
                                RtpCodecKind::Audio => "audio/telephone-event".to_owned(),
                                RtpCodecKind::Video => "video/VP8".to_owned(),
                                RtpCodecKind::Unspecified => String::new(),
                            },
                            clock_rate: 48_000,
                            channels: 1,
                            ..Default::default()
                        },
                        payload_type,
                    })
                    .collect(),
            );
        transceiver
    }

    #[test]
    fn payload_type_requires_unique_receiver_unless_mid_selects_one() {
        let transceivers = vec![
            receiver(RtpCodecKind::Audio, "audio-0", &[111, 110]),
            receiver(RtpCodecKind::Audio, "audio-1", &[111, 110]),
            receiver(RtpCodecKind::Audio, "audio-2", &[126]),
        ];

        assert_eq!(
            undeclared_receiver_index(&transceivers, 126, None),
            Some(2),
            "a unique negotiated payload type identifies its receiver"
        );
        assert_eq!(
            undeclared_receiver_index(&transceivers, 110, None),
            None,
            "an ambiguous payload type must not use vector order as ownership"
        );
        assert_eq!(
            undeclared_receiver_index(&transceivers, 110, Some("audio-1")),
            Some(1),
            "a negotiated MID disambiguates equal payload types"
        );
        assert_eq!(
            undeclared_receiver_index(&transceivers, 126, Some("audio-1")),
            None,
            "MID selection still requires that receiver to negotiate the payload type"
        );
        assert_eq!(
            undeclared_receiver_index(&transceivers, 110, Some("unknown")),
            None,
            "an unknown MID cannot escape its signaling context"
        );
    }

    #[test]
    fn mid_only_video_routes_to_its_authoritative_receiver() {
        let transceivers = vec![
            receiver(RtpCodecKind::Audio, "audio-0", &[111]),
            receiver(RtpCodecKind::Video, "video-0", &[96]),
        ];

        assert_eq!(
            undeclared_receiver_index(&transceivers, 96, Some("video-0")),
            Some(1),
            "an explicit MID must route an undeclared video SSRC"
        );
        assert_eq!(
            undeclared_receiver_index(&transceivers, 96, Some("audio-0")),
            None,
            "the payload type cannot escape its authoritative MID"
        );
    }

    #[test]
    fn sole_video_without_header_extensions_uses_generic_fallback() {
        let transceivers = vec![receiver(RtpCodecKind::Video, "video-0", &[96])];

        assert_eq!(
            undeclared_receiver_index(&transceivers, 96, None),
            Some(0),
            "a sole video receiver with no RID codings accepts an undeclared SSRC"
        );

        let bundled = vec![
            receiver(RtpCodecKind::Video, "video-0", &[96]),
            receiver(RtpCodecKind::Audio, "audio-0", &[111]),
        ];
        assert_eq!(
            undeclared_receiver_index(&bundled, 96, None),
            None,
            "without MID, a video receiver is not selected from multiple media sections"
        );
    }

    #[test]
    fn supplemental_coding_is_appended_once_without_replacing_primary() {
        let mut codings = vec![RTCRtpCodingParameters {
            rid: String::new(),
            ssrc: Some(1001),
            rtx: None,
            fec: None,
        }];

        assert!(append_undeclared_coding(&mut codings, 2002));
        assert_eq!(codings.len(), 2);
        assert_eq!(codings[0].ssrc, Some(1001));
        assert_eq!(codings[1].ssrc, Some(2002));
        assert!(!append_undeclared_coding(&mut codings, 2002));
        assert!(!append_undeclared_coding(&mut codings, 1001));
        assert_eq!(codings.len(), 2, "repeated packets reuse their coding");
        assert_eq!(codings[0].ssrc, Some(1001));
    }
}
