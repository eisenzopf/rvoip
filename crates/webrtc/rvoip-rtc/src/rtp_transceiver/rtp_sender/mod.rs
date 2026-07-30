//! RTP sender module providing WebRTC RTPSender functionality.
//!
//! This module implements the RTPSender interface which controls how media tracks
//! are encoded and transmitted to remote peers.
//!
//! # Overview
//!
//! An RTP sender manages the transmission of a single media track, providing control over:
//! - Codec selection and parameters
//! - Encoding parameters (bitrate, resolution, framerate)
//! - Simulcast and layered encoding configurations
//! - Direct RTP/RTCP packet transmission
//!
//! # Examples
//!
//! ## Getting the sender's track
//!
//! ```no_run
//! # use rtc::peer_connection::RTCPeerConnectionBuilder;
//! # use rtc::rtp_transceiver::RTCRtpSenderId;
//! # fn example(sender_id: RTCRtpSenderId) -> Result<(), Box<dyn std::error::Error>> {
//! let mut peer_connection = RTCPeerConnectionBuilder::new().build()?;
//!
//! // Get sender and access its track
//! if let Some(mut sender) = peer_connection.rtp_sender(sender_id) {
//!     let track = sender.track();
//!     println!("Track ID: {}", track.track_id());
//!     println!("Track kind: {:?}", track.kind());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Getting and modifying send parameters
//!
//! ```no_run
//! # use rtc::peer_connection::RTCPeerConnectionBuilder;
//! # use rtc::rtp_transceiver::RTCRtpSenderId;
//! # fn example(sender_id: RTCRtpSenderId) -> Result<(), Box<dyn std::error::Error>> {
//! let mut peer_connection = RTCPeerConnectionBuilder::new().build()?;
//!
//! if let Some(mut sender) = peer_connection.rtp_sender(sender_id) {
//!     // Get current parameters
//!     let mut params = sender.get_parameters().clone();
//!     
//!     // Modify encoding parameters
//!     for encoding in &mut params.encodings {
//!         encoding.max_bitrate = 1_000_000; // 1 Mbps
//!         encoding.max_framerate = Some(30.0);
//!     }
//!     
//!     // Apply the changes
//!     sender.set_parameters(params, None)?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Replacing a track
//!
//! ```no_run
//! # use rtc::peer_connection::RTCPeerConnectionBuilder;
//! # use rtc::media_stream::MediaStreamTrack;
//! # use rtc::rtp_transceiver::RTCRtpSenderId;
//! # fn example(
//! #     sender_id: RTCRtpSenderId,
//! #     new_track: MediaStreamTrack
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let mut peer_connection = RTCPeerConnectionBuilder::new().build()?;
//!
//! if let Some(mut sender) = peer_connection.rtp_sender(sender_id) {
//!     // Replace with new track (same kind required)
//!     sender.replace_track(new_track)?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Writing raw RTP packets
//!
//! ```no_run
//! # use rtc::peer_connection::RTCPeerConnectionBuilder;
//! # use rtc::rtp_transceiver::RTCRtpSenderId;
//! # use rtp::packet::Packet;
//! # use shared::error::Error;
//! # fn example(
//! #     sender_id: RTCRtpSenderId,
//! #     mut rtp_packet: Packet
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let mut peer_connection = RTCPeerConnectionBuilder::new().build()?;
//!
//! if let Some(mut sender) = peer_connection.rtp_sender(sender_id) {
//!     // Write RTP packet directly
//!     // The sender will set the correct payload type and SSRC
//!     rtp_packet.header.ssrc = sender
//!         .track()
//!         .ssrcs()
//!         .last()
//!         .ok_or(Error::ErrSenderWithNoSSRCs)?;
//!     sender.write_rtp(rtp_packet)?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Configuring simulcast with transceiver init
//!
//! ```no_run
//! # use rtc::peer_connection::RTCPeerConnectionBuilder;
//! # use rtc::media_stream::MediaStreamTrack;
//! # use rtc::rtp_transceiver::{RTCRtpTransceiverInit, RTCRtpTransceiverDirection};
//! # use rtc::rtp_transceiver::rtp_sender::{
//! #     RTCRtpEncodingParameters, RTCRtpCodingParameters
//! # };
//! # fn example(video_track: MediaStreamTrack) -> Result<(), Box<dyn std::error::Error>> {
//! let mut peer_connection = RTCPeerConnectionBuilder::new().build()?;
//!
//! // Configure simulcast with three layers
//! let mut init = RTCRtpTransceiverInit {
//!     direction: RTCRtpTransceiverDirection::Sendrecv,
//!     ..Default::default()
//! };
//!
//! // High quality layer
//! init.send_encodings.push(RTCRtpEncodingParameters {
//!     rtp_coding_parameters: RTCRtpCodingParameters {
//!         rid: "h".to_string(),
//!         ..Default::default()
//!     },
//!     max_bitrate: 2_000_000, // 2 Mbps
//!     scale_resolution_down_by: Some(1.0),
//!     ..Default::default()
//! });
//!
//! // Medium quality layer
//! init.send_encodings.push(RTCRtpEncodingParameters {
//!     rtp_coding_parameters: RTCRtpCodingParameters {
//!         rid: "m".to_string(),
//!         ..Default::default()
//!     },
//!     max_bitrate: 1_000_000, // 1 Mbps
//!     scale_resolution_down_by: Some(2.0),
//!     ..Default::default()
//! });
//!
//! // Low quality layer
//! init.send_encodings.push(RTCRtpEncodingParameters {
//!     rtp_coding_parameters: RTCRtpCodingParameters {
//!         rid: "l".to_string(),
//!         ..Default::default()
//!     },
//!     max_bitrate: 500_000, // 500 kbps
//!     scale_resolution_down_by: Some(4.0),
//!     ..Default::default()
//! });
//!
//! peer_connection.add_transceiver_from_track(video_track, Some(init))?;
//! # Ok(())
//! # }
//! ```
//!
//! # Specifications
//!
//! * [W3C RTCRtpSender](https://w3c.github.io/webrtc-pc/#rtcrtpsender-interface)
//! * [MDN RTCRtpSender](https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpSender)

//TODO: #[cfg(test)]
//mod rtp_sender_test;

pub(crate) mod internal;
pub(crate) mod rtcp_parameters;
pub(crate) mod rtp_capabilities;
pub(crate) mod rtp_codec;
pub(crate) mod rtp_codec_parameters;
pub(crate) mod rtp_coding_parameters;
pub(crate) mod rtp_encoding_parameters;
pub(crate) mod rtp_header_extension_capability;
pub(crate) mod rtp_header_extension_parameters;
pub(crate) mod rtp_parameters;
pub(crate) mod rtp_receiver_parameters;
pub(crate) mod rtp_send_parameters;
pub(crate) mod set_parameter_options;

use crate::media_stream::MediaStreamId;
use crate::media_stream::track::MediaStreamTrack;
use crate::peer_connection::RTCPeerConnection;
use crate::peer_connection::message::RTCMessage;
use crate::rtp_transceiver::RTCRtpSenderId;
use crate::rtp_transceiver::rtp_sender::rtp_codec::{CodecMatch, codec_parameters_fuzzy_search};
use interceptor::{Interceptor, NoopInterceptor};
use sansio::Protocol;
use shared::error::{Error, Result};

pub use rtcp_parameters::{RTCPFeedback, RTCRtcpParameters};
pub use rtcp_parameters::{
    TYPE_RTCP_FB_ACK, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK,
    TYPE_RTCP_FB_TRANSPORT_CC,
};
pub use rtp_capabilities::RTCRtpCapabilities;
pub use rtp_codec::{RTCRtpCodec, RtpCodecKind};
pub use rtp_codec_parameters::RTCRtpCodecParameters;
pub use rtp_coding_parameters::{RTCRtpCodingParameters, RTCRtpFecParameters, RTCRtpRtxParameters};
pub use rtp_encoding_parameters::RTCRtpEncodingParameters;
pub use rtp_header_extension_capability::RTCRtpHeaderExtensionCapability;
pub use rtp_header_extension_parameters::RTCRtpHeaderExtensionParameters;
pub use rtp_parameters::RTCRtpParameters;
pub use rtp_receiver_parameters::RTCRtpReceiveParameters;
pub use rtp_send_parameters::RTCRtpSendParameters;
pub use set_parameter_options::RTCSetParameterOptions;

/// RTCRtpSender controls the encoding and transmission of media tracks to remote peers.
///
/// This struct provides a handle to the RTP sender within a peer connection,
/// allowing control over parameters, track replacement, and direct RTP/RTCP packet writing.
///
/// ## Specifications
///
/// * [MDN]
/// * [W3C]
///
/// [MDN]: https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpSender
/// [W3C]: https://w3c.github.io/webrtc-pc/#rtcrtpsender-interface
pub struct RTCRtpSender<'a, I = NoopInterceptor>
where
    I: Interceptor,
{
    pub(crate) id: RTCRtpSenderId,
    pub(crate) peer_connection: &'a mut RTCPeerConnection<I>,
}

fn outbound_payload_type(
    requested_payload_type: crate::rtp_transceiver::PayloadType,
    selected_codec: &RTCRtpCodecParameters,
    codecs: &[RTCRtpCodecParameters],
    encodings: &[RTCRtpEncodingParameters],
) -> crate::rtp_transceiver::PayloadType {
    let Some(requested_codec) = codecs
        .iter()
        .find(|codec| codec.payload_type == requested_payload_type)
    else {
        return selected_codec.payload_type;
    };

    // A negotiated codec is eligible on this track only when a coding on the
    // track represents that exact negotiated payload type. This prevents an
    // arbitrary codec from the m-line from being injected through an SSRC
    // owned by this sender.
    let represented_by_track = encodings.iter().any(|encoding| {
        let (represented_codec, match_type) =
            codec_parameters_fuzzy_search(&encoding.codec, codecs);
        match_type != CodecMatch::None && represented_codec.payload_type == requested_payload_type
    });

    if represented_by_track
        && requested_codec.rtp_codec.clock_rate == selected_codec.rtp_codec.clock_rate
    {
        requested_payload_type
    } else {
        // Preserve the legacy behavior for unnegotiated, unrepresented, or
        // clock-rate-changing payload types.
        selected_codec.payload_type
    }
}

impl<I> RTCRtpSender<'_, I>
where
    I: Interceptor,
{
    /// Returns the media track being sent by this sender.
    pub fn track(&self) -> &MediaStreamTrack {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.
        self.peer_connection.rtp_transceivers[self.id.0]
            .sender()
            .as_ref()
            .unwrap()
            .track()
    }

    /// Returns the RTP capabilities for the specified codec kind.
    ///
    /// # Parameters
    ///
    /// * `kind` - The codec type (audio or video) to query capabilities for
    pub fn get_capabilities(&self, kind: RtpCodecKind) -> Option<RTCRtpCapabilities> {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.
        self.peer_connection.rtp_transceivers[self.id.0]
            .sender()
            .as_ref()
            .unwrap()
            .get_capabilities(kind, &self.peer_connection.media_engine)
    }
    /// Updates the RTP send parameters for this sender.
    ///
    /// This method modifies encoding parameters such as bitrates, frame rates,
    /// and active state for each encoding.
    ///
    /// # Parameters
    ///
    /// * `parameters` - The new send parameters to apply
    /// * `set_parameter_options` - Optional additional configuration options
    ///
    /// # Errors
    ///
    /// Returns an error if the parameters are invalid.
    pub fn set_parameters(
        &mut self,
        parameters: RTCRtpSendParameters,
        set_parameter_options: Option<RTCSetParameterOptions>,
    ) -> Result<()> {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.
        self.peer_connection.rtp_transceivers[self.id.0]
            .sender_mut()
            .as_mut()
            .unwrap()
            .set_parameters(parameters, set_parameter_options)
    }

    /// Returns the sender's current RTP send parameters.
    ///
    /// The returned parameters describe how the track is encoded and transmitted,
    /// including codecs, encodings, and header extensions.
    pub fn get_parameters(&mut self) -> &RTCRtpSendParameters {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.
        self.peer_connection.rtp_transceivers[self.id.0]
            .sender_mut()
            .as_mut()
            .unwrap()
            .get_parameters(&self.peer_connection.media_engine)
    }

    /// Replaces the currently sent track with a new media track.
    ///
    /// The new track must be of the same media kind (audio, video, etc) as the original.
    /// Track replacement can be performed without renegotiation.
    ///
    /// # Parameters
    ///
    /// * `track` - The new track to send
    ///
    /// # Errors
    ///
    /// Returns an error if the track kinds do not match.
    ///
    /// ## Specifications
    ///
    /// * [W3C](https://www.w3.org/TR/webrtc/#dom-rtcrtpsender-replacetrack)
    pub fn replace_track(&mut self, track: MediaStreamTrack) -> Result<()> {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.
        self.peer_connection.rtp_transceivers[self.id.0]
            .sender_mut()
            .as_mut()
            .unwrap()
            .replace_track(track)
    }

    /// Sets the media stream IDs associated with this sender's track.
    ///
    /// # Parameters
    ///
    /// * `streams` - Vector of stream IDs to associate with the track
    pub fn set_streams(&mut self, streams: Vec<MediaStreamId>) {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.
        self.peer_connection.rtp_transceivers[self.id.0]
            .sender_mut()
            .as_mut()
            .unwrap()
            .set_streams(streams);
    }

    /// Writes an RTP packet to the network.
    ///
    /// This method allows direct writing of RTP packets, automatically setting
    /// the correct payload type and SSRC based on the sender's configuration.
    ///
    /// # Parameters
    ///
    /// * `packet` - The RTP packet to send
    ///
    /// # Errors
    ///
    /// Returns an error if no matching encoding is found, or the codec is unsupported,
    /// or internal handle_write returns error
    pub fn write_rtp(&mut self, mut packet: rtp::Packet) -> Result<()> {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.

        //TODO: handle rtp header extension, etc.
        let (sender, media_engine) = (
            self.peer_connection.rtp_transceivers[self.id.0]
                .sender_mut()
                .as_mut()
                .unwrap(),
            &mut self.peer_connection.media_engine,
        );

        if !sender
            .track()
            .ssrcs()
            .any(|ssrc| ssrc == packet.header.ssrc)
        {
            return Err(Error::ErrSenderWithNoSSRCs);
        }

        let parameters = sender.get_parameters(media_engine);
        let (codecs, encodings) = (&parameters.rtp_parameters.codecs, &parameters.encodings);

        //From SSRC, find the encoding
        let encoding = encodings
            .iter()
            .find(|encoding| {
                encoding
                    .rtp_coding_parameters
                    .ssrc
                    .is_some_and(|s| s == packet.header.ssrc)
            })
            .ok_or(Error::ErrRTPSenderNoBaseEncoding)?;
        // From the encoding, fuzzy_search the codec which contains payload_type
        let (codec, match_type) = codec_parameters_fuzzy_search(&encoding.codec, codecs);
        if match_type == CodecMatch::None {
            return Err(Error::ErrRTPTransceiverCodecUnsupported);
        }

        packet.header.payload_type =
            outbound_payload_type(packet.header.payload_type, &codec, codecs, encodings);
        let track_id = sender.track().track_id().to_string();
        self.peer_connection
            .handle_write(RTCMessage::RtpPacket(track_id, packet))
    }

    /// Writes RTCP packets to the network.
    ///
    /// This method allows direct writing of sender-related RTCP reports such as
    /// Sender Reports (SR) or other feedback messages.
    ///
    /// # Parameters
    ///
    /// * `packets` - Vector of RTCP packets to send
    ///
    /// # Errors
    ///
    /// Returns an error if internal handle_write returns error
    pub fn write_rtcp(&mut self, packets: Vec<Box<dyn rtcp::Packet>>) -> Result<()> {
        // peer_connection is mutable borrow, its rtp_transceivers won't be resized and
        // the direction won't be changed too, so, unwrap() here is safe.

        //TODO: handle rtcp sender ssrc, header extension, etc.
        let sender = self.peer_connection.rtp_transceivers[self.id.0]
            .sender_mut()
            .as_mut()
            .unwrap();

        let track_id = sender.track().track_id().to_string();
        self.peer_connection
            .handle_write(RTCMessage::RtcpPacket(track_id, packets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp_transceiver::rtp_sender::rtp_coding_parameters::RTCRtpCodingParameters;

    fn codec(
        payload_type: crate::rtp_transceiver::PayloadType,
        mime_type: &str,
        clock_rate: u32,
    ) -> RTCRtpCodecParameters {
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: mime_type.to_owned(),
                clock_rate,
                channels: 1,
                ..Default::default()
            },
            payload_type,
        }
    }

    fn encoding(ssrc: u32, codec: &RTCRtpCodecParameters) -> RTCRtpEncodingParameters {
        RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec: codec.rtp_codec.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn outbound_payload_type_preserves_represented_same_clock_supplemental_codec() {
        let opus = codec(111, "audio/opus", 48_000);
        let telephone_event = codec(110, "audio/telephone-event", 48_000);
        let codecs = vec![opus.clone(), telephone_event.clone()];
        let encodings = vec![
            encoding(0x0102_0304, &opus),
            encoding(0x0506_0708, &telephone_event),
        ];

        assert_eq!(
            outbound_payload_type(telephone_event.payload_type, &opus, &codecs, &encodings),
            telephone_event.payload_type
        );
    }

    #[test]
    fn outbound_payload_type_rejects_represented_codec_with_different_clock() {
        let opus = codec(111, "audio/opus", 48_000);
        let telephone_event = codec(101, "audio/telephone-event", 8_000);
        let codecs = vec![opus.clone(), telephone_event.clone()];
        let encodings = vec![
            encoding(0x0102_0304, &opus),
            encoding(0x0506_0708, &telephone_event),
        ];

        assert_eq!(
            outbound_payload_type(telephone_event.payload_type, &opus, &codecs, &encodings),
            opus.payload_type
        );
    }

    #[test]
    fn outbound_payload_type_rejects_negotiated_but_unrepresented_codec() {
        let opus = codec(111, "audio/opus", 48_000);
        let supplemental = codec(112, "audio/example", 48_000);
        let codecs = vec![opus.clone(), supplemental.clone()];
        let encodings = vec![encoding(0x0102_0304, &opus)];

        assert_eq!(
            outbound_payload_type(supplemental.payload_type, &opus, &codecs, &encodings),
            opus.payload_type
        );
    }

    #[test]
    fn outbound_payload_type_rejects_unnegotiated_codec() {
        let opus = codec(111, "audio/opus", 48_000);
        let codecs = vec![opus.clone()];
        let encodings = vec![encoding(0x0102_0304, &opus)];

        assert_eq!(
            outbound_payload_type(127, &opus, &codecs, &encodings),
            opus.payload_type
        );
    }
}
