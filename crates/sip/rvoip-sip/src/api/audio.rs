//! Audio streaming types for rvoip-sip.
//!
//! Provides a split duplex audio stream per session, carrying
//! [`rvoip_media_core::types::AudioFrame`] values produced by the
//! [`rvoip_media_core::MediaSession`] underneath an active call. The caller
//! owns the send/receive loop, enabling flexible bridging (mix-minus,
//! recording, AEC) in higher layers. RTP packetisation, jitter buffering,
//! and SRTP handling stay inside [`rvoip_rtp_core`] — this module surfaces
//! only decoded/encoded frames.

#![deny(missing_docs)]

use crate::errors::{Result, SessionError};
use rvoip_media_core::types::AudioFrame;
use tokio::sync::mpsc;

/// Atomic evidence returned after a response-generation advance.
pub use rvoip_audio_send_queue::GenerationAdvance as AudioGenerationAdvance;
/// Observable counters for the bounded outbound audio queue.
pub use rvoip_audio_send_queue::Metrics as AudioSendMetrics;
/// Typed rejection returned by generation-aware outbound audio submission.
pub use rvoip_audio_send_queue::SendError as AudioSendError;

/// Split duplex audio stream for a single session.
///
/// Obtain one via [`SessionHandle::audio()`][crate::api::handle::SessionHandle::audio].
/// The stream can be split into independent [`AudioSender`] and [`AudioReceiver`] halves
/// for use across separate tasks.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example(handle: rvoip_sip::SessionHandle) -> anyhow::Result<()> {
/// let audio = handle.audio().await?;
/// let (mut tx, mut rx) = audio.split();
///
/// // Spawn audio producer
/// tokio::spawn(async move {
///     loop {
///         let frame = rvoip_media_core::types::AudioFrame::new(
///             vec![0i16; 160], 8000, 1, 0
///         );
///         if tx.send(frame).await.is_err() { break; }
///         tokio::time::sleep(std::time::Duration::from_millis(20)).await;
///     }
/// });
///
/// // Receive audio in current task
/// while let Some(frame) = rx.recv().await {
///     // process frame ...
/// }
/// # Ok(())
/// # }
/// ```
pub struct AudioStream {
    /// Send audio to the remote party
    pub sender: AudioSender,
    /// Receive audio from the remote party
    pub receiver: AudioReceiver,
}

impl AudioStream {
    pub(crate) fn new(sender: AudioSender, receiver: AudioReceiver) -> Self {
        Self { sender, receiver }
    }

    /// Split into independent sender and receiver halves.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(call: rvoip_sip::SessionHandle) -> rvoip_sip::Result<()> {
    /// let stream = call.audio().await?;
    /// let (sender, mut receiver) = stream.split();
    /// # let _ = sender.is_open();
    /// # let _ = receiver.try_recv();
    /// # Ok(())
    /// # }
    /// ```
    pub fn split(self) -> (AudioSender, AudioReceiver) {
        (self.sender, self.receiver)
    }
}

/// Send half of an [`AudioStream`].
///
/// Cheap to clone — both clones share the same underlying channel to the
/// media layer.
#[derive(Clone)]
pub struct AudioSender {
    tx: rvoip_audio_send_queue::Sender<AudioFrame>,
}

impl AudioSender {
    pub(crate) fn channel(
        capacity: usize,
    ) -> (Self, rvoip_audio_send_queue::Receiver<AudioFrame>) {
        let (tx, rx) = rvoip_audio_send_queue::channel(capacity);
        (Self { tx }, rx)
    }

    /// Send an audio frame to the remote party.
    ///
    /// Returns `Err` if the session ended or the bounded outbound queue entered
    /// its terminal overload state.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(sender: rvoip_sip::AudioSender) -> rvoip_sip::Result<()> {
    /// let frame = rvoip_media_core::types::AudioFrame::new(vec![0i16; 160], 8000, 1, 0);
    /// sender.send(frame).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send(&self, frame: AudioFrame) -> Result<()> {
        self.try_send_generation(self.current_generation(), frame)
            .map_err(|error| SessionError::Other(error.to_string()))
    }

    /// Submit a frame for an exact response generation without waiting.
    pub fn try_send_generation(
        &self,
        generation: u64,
        frame: AudioFrame,
    ) -> std::result::Result<(), AudioSendError> {
        self.tx.try_send(generation, frame)
    }

    /// Accept a newer response generation and synchronously flush older frames.
    pub fn advance_generation(&self, generation: u64) -> Option<AudioGenerationAdvance> {
        self.tx.advance_to(generation)
    }

    /// Close the queue and synchronously remove pending frames.
    pub fn close_and_flush(&self) {
        self.tx.close_and_flush();
    }

    /// Return the generation currently accepted by this sender.
    pub fn current_generation(&self) -> u64 {
        self.tx.generation()
    }

    /// Return a point-in-time snapshot of bounded queue counters.
    pub fn metrics(&self) -> AudioSendMetrics {
        self.tx.metrics()
    }

    /// Returns `true` if the underlying session is still active.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(sender: rvoip_sip::AudioSender) {
    /// if sender.is_open() {
    ///     println!("audio send channel is open");
    /// }
    /// # }
    /// ```
    pub fn is_open(&self) -> bool {
        self.tx.is_open()
    }
}

/// Receive half of an [`AudioStream`].
pub struct AudioReceiver {
    rx: mpsc::Receiver<AudioFrame>,
}

impl AudioReceiver {
    pub(crate) fn new(rx: mpsc::Receiver<AudioFrame>) -> Self {
        Self { rx }
    }

    /// Wait for the next audio frame from the remote party.
    ///
    /// Returns `None` when the session ends and no more frames will arrive.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(mut receiver: rvoip_sip::AudioReceiver) {
    /// while let Some(frame) = receiver.recv().await {
    ///     println!("received {} samples", frame.samples.len());
    /// }
    /// # }
    /// ```
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        self.rx.recv().await
    }

    /// Try to receive an audio frame without blocking.
    ///
    /// Returns `None` if no frame is available right now.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # fn example(mut receiver: rvoip_sip::AudioReceiver) {
    /// if let Some(frame) = receiver.try_recv() {
    ///     println!("received {} samples", frame.samples.len());
    /// }
    /// # }
    /// ```
    pub fn try_recv(&mut self) -> Option<AudioFrame> {
        self.rx.try_recv().ok()
    }
}
