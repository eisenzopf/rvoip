//! Audio streaming types for session-core
//!
//! Provides a split duplex audio stream per session. The caller owns the
//! send/receive loop, enabling flexible bridging in higher layers.

use rvoip_media_core::types::AudioFrame;
use tokio::sync::mpsc;
use crate::errors::{Result, SessionError};

/// Split duplex audio stream for a single session.
///
/// Obtain one via [`SessionHandle::audio()`][crate::api::handle::SessionHandle::audio].
/// The stream can be split into independent [`AudioSender`] and [`AudioReceiver`] halves
/// for use across separate tasks.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example(handle: rvoip_session_core::SessionHandle) -> anyhow::Result<()> {
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
    tx: mpsc::Sender<AudioFrame>,
}

impl AudioSender {
    pub(crate) fn new(tx: mpsc::Sender<AudioFrame>) -> Self {
        Self { tx }
    }

    /// Send an audio frame to the remote party.
    ///
    /// Returns `Err` only if the session has ended and the channel is closed.
    pub async fn send(&self, frame: AudioFrame) -> Result<()> {
        self.tx.send(frame).await.map_err(|_| {
            SessionError::Other("Audio send channel closed (session ended)".to_string())
        })
    }

    /// Returns `true` if the underlying session is still active.
    pub fn is_open(&self) -> bool {
        !self.tx.is_closed()
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
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        self.rx.recv().await
    }

    /// Try to receive an audio frame without blocking.
    ///
    /// Returns `None` if no frame is available right now.
    pub fn try_recv(&mut self) -> Option<AudioFrame> {
        self.rx.try_recv().ok()
    }
}
