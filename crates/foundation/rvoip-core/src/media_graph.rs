//! Bounded, one-source-to-many real-time media routing.
//!
//! A `MediaStream::frames_in()` receiver is intentionally single-take. The
//! media graph owns that receiver once and exposes dynamic sink routes so a
//! call peer, recorder, UCTP publisher, and MOQT publisher can observe the
//! same source without racing for frames.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rvoip_media_core::codec::transcoding::Transcoder;
use rvoip_media_core::processing::format::FormatConverter;
use tokio::sync::{mpsc, oneshot, Notify, RwLock};
use tokio::task::AbortHandle;
use tracing::{debug, warn};

use crate::bridge::{codec_to_pt, frame_pump::DEFAULT_TELEPHONE_EVENT_PT};
use crate::capability::CodecInfo;
use crate::error::{Result, RvoipError};
use crate::ids::MediaRouteId;
use crate::stream::MediaFrame;

#[derive(Clone, Debug)]
pub struct MediaGraphPolicy {
    pub sink_queue_frames: usize,
    pub eviction_window: Duration,
    pub eviction_drop_ratio: f32,
    pub minimum_eviction_samples: usize,
}

impl Default for MediaGraphPolicy {
    fn default() -> Self {
        Self {
            sink_queue_frames: 10,
            eviction_window: Duration::from_secs(10),
            eviction_drop_ratio: 0.25,
            minimum_eviction_samples: 50,
        }
    }
}

enum Command {
    Add {
        route_id: MediaRouteId,
        codec: CodecInfo,
        target: mpsc::Sender<MediaFrame>,
    },
    Remove(MediaRouteId),
    Update {
        route_id: MediaRouteId,
        source_pt: u8,
        target_pt: u8,
        ack: oneshot::Sender<()>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct MediaGraphHandle {
    commands: mpsc::UnboundedSender<Command>,
    abort: AbortHandle,
}

impl MediaGraphHandle {
    pub fn add_sink(
        &self,
        codec: CodecInfo,
        target: mpsc::Sender<MediaFrame>,
    ) -> Result<MediaRouteId> {
        codec_to_pt(&codec.name).ok_or_else(|| RvoipError::UnsupportedCodec(codec.name.clone()))?;
        let route_id = MediaRouteId::new();
        self.commands
            .send(Command::Add {
                route_id: route_id.clone(),
                codec,
                target,
            })
            .map_err(|_| RvoipError::InvalidState("media graph is closed"))?;
        Ok(route_id)
    }

    pub fn remove_sink(&self, route_id: MediaRouteId) -> bool {
        self.commands.send(Command::Remove(route_id)).is_ok()
    }

    pub async fn update_route(
        &self,
        route_id: MediaRouteId,
        source_pt: u8,
        target_pt: u8,
    ) -> Result<()> {
        let (ack, done) = oneshot::channel();
        self.commands
            .send(Command::Update {
                route_id,
                source_pt,
                target_pt,
                ack,
            })
            .map_err(|_| RvoipError::InvalidState("media graph is closed"))?;
        tokio::time::timeout(Duration::from_secs(1), done)
            .await
            .map_err(|_| RvoipError::InvalidState("media graph update timed out"))?
            .map_err(|_| RvoipError::InvalidState("media graph update was cancelled"))
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }

    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }
}

struct SinkQueueState {
    frames: VecDeque<MediaFrame>,
    closed: bool,
}

struct SinkQueue {
    capacity: usize,
    state: Mutex<SinkQueueState>,
    notify: Notify,
}

impl SinkQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            state: Mutex::new(SinkQueueState {
                frames: VecDeque::with_capacity(capacity.max(1)),
                closed: false,
            }),
            notify: Notify::new(),
        }
    }

    /// Enqueue without awaiting a slow sink. Returns true when the oldest
    /// queued frame had to be discarded.
    fn offer(&self, frame: MediaFrame) -> bool {
        let dropped = {
            let mut state = self.state.lock().expect("media sink queue poisoned");
            if state.closed {
                return true;
            }
            let dropped = if state.frames.len() >= self.capacity {
                state.frames.pop_front();
                true
            } else {
                false
            };
            state.frames.push_back(frame);
            dropped
        };
        self.notify.notify_one();
        dropped
    }

    async fn receive(&self) -> Option<MediaFrame> {
        loop {
            let state = {
                let mut state = self.state.lock().expect("media sink queue poisoned");
                if let Some(frame) = state.frames.pop_front() {
                    return Some(frame);
                }
                state.closed
            };
            if state {
                return None;
            }
            self.notify.notified().await;
        }
    }

    fn close(&self) {
        {
            let mut state = self.state.lock().expect("media sink queue poisoned");
            state.closed = true;
            state.frames.clear();
        }
        self.notify.notify_waiters();
    }
}

struct SinkRuntime {
    target_pt: u8,
    queue: Arc<SinkQueue>,
    task: AbortHandle,
    history: VecDeque<(Instant, bool)>,
}

impl SinkRuntime {
    fn record_offer(&mut self, now: Instant, dropped: bool, policy: &MediaGraphPolicy) -> bool {
        self.history.push_back((now, dropped));
        while self
            .history
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > policy.eviction_window)
        {
            self.history.pop_front();
        }
        if self.history.len() < policy.minimum_eviction_samples {
            return false;
        }
        let drops = self.history.iter().filter(|(_, dropped)| *dropped).count();
        drops as f32 / self.history.len() as f32 > policy.eviction_drop_ratio
    }
}

impl Drop for SinkRuntime {
    fn drop(&mut self) {
        self.queue.close();
        self.task.abort();
    }
}

struct CodecGroup {
    target_pt: u8,
    transcoder: Option<Transcoder>,
    sinks: HashSet<MediaRouteId>,
}

impl CodecGroup {
    fn new(source_pt: u8, target_pt: u8) -> Self {
        Self {
            target_pt,
            transcoder: make_transcoder(source_pt, target_pt),
            sinks: HashSet::new(),
        }
    }
}

fn make_transcoder(source_pt: u8, target_pt: u8) -> Option<Transcoder> {
    (source_pt != target_pt).then(|| Transcoder::new(Arc::new(RwLock::new(FormatConverter::new()))))
}

/// Start a media graph task that owns `source` for its lifetime.
pub fn start_media_graph(
    mut source: mpsc::Receiver<MediaFrame>,
    source_codec: CodecInfo,
    policy: MediaGraphPolicy,
) -> Result<MediaGraphHandle> {
    let mut source_pt = codec_to_pt(&source_codec.name)
        .ok_or_else(|| RvoipError::UnsupportedCodec(source_codec.name.clone()))?;
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();
    let (sink_event_tx, mut sink_event_rx) = mpsc::unbounded_channel::<MediaRouteId>();

    let task = tokio::spawn(async move {
        let mut sinks: HashMap<MediaRouteId, SinkRuntime> = HashMap::new();
        let mut groups: HashMap<u8, CodecGroup> = HashMap::new();

        loop {
            tokio::select! {
                biased;
                command = command_rx.recv() => {
                    let Some(command) = command else { break; };
                    match command {
                        Command::Add { route_id, codec, target } => {
                            let Some(target_pt) = codec_to_pt(&codec.name) else { continue; };
                            let queue = Arc::new(SinkQueue::new(policy.sink_queue_frames));
                            let queue_for_task = Arc::clone(&queue);
                            let route_for_task = route_id.clone();
                            let event_tx = sink_event_tx.clone();
                            let task = tokio::spawn(async move {
                                while let Some(frame) = queue_for_task.receive().await {
                                    if target.send(frame).await.is_err() {
                                        let _ = event_tx.send(route_for_task.clone());
                                        return;
                                    }
                                }
                            });
                            groups.entry(target_pt)
                                .or_insert_with(|| CodecGroup::new(source_pt, target_pt))
                                .sinks.insert(route_id.clone());
                            sinks.insert(route_id, SinkRuntime {
                                target_pt,
                                queue,
                                task: task.abort_handle(),
                                history: VecDeque::new(),
                            });
                            metrics::gauge!("rvoip_media_graph_sinks").set(sinks.len() as f64);
                        }
                        Command::Remove(route_id) => remove_sink(&route_id, &mut sinks, &mut groups),
                        Command::Update { route_id, source_pt: new_source_pt, target_pt, ack } => {
                            source_pt = new_source_pt;
                            if let Some(sink) = sinks.get_mut(&route_id) {
                                if let Some(group) = groups.get_mut(&sink.target_pt) {
                                    group.sinks.remove(&route_id);
                                }
                                sink.target_pt = target_pt;
                                groups.entry(target_pt)
                                    .or_insert_with(|| CodecGroup::new(source_pt, target_pt))
                                    .sinks.insert(route_id);
                                groups.retain(|_, group| !group.sinks.is_empty());
                                for group in groups.values_mut() {
                                    group.transcoder = make_transcoder(source_pt, group.target_pt);
                                }
                            }
                            let _ = ack.send(());
                        }
                        Command::Shutdown => break,
                    }
                }
                closed_route = sink_event_rx.recv() => {
                    if let Some(route_id) = closed_route {
                        remove_sink(&route_id, &mut sinks, &mut groups);
                    }
                }
                frame = source.recv() => {
                    let Some(frame) = frame else { break; };
                    let now = Instant::now();
                    let mut evict = Vec::new();
                    for group in groups.values_mut() {
                        let mut routed = frame.clone();
                        if frame.payload_type != Some(DEFAULT_TELEPHONE_EVENT_PT) {
                            if let Some(transcoder) = group.transcoder.as_mut() {
                                match transcoder.transcode(&frame.payload, source_pt, group.target_pt).await {
                                    Ok(payload) => {
                                        routed.payload = payload.into();
                                        routed.payload_type = Some(group.target_pt);
                                    }
                                    Err(error) => {
                                        warn!(%error, source_pt, target_pt = group.target_pt, "media graph transcode failed");
                                        metrics::counter!("rvoip_media_graph_transcode_errors_total").increment(1);
                                        continue;
                                    }
                                }
                            }
                        }
                        for route_id in &group.sinks {
                            let Some(sink) = sinks.get_mut(route_id) else { continue; };
                            let dropped = sink.queue.offer(routed.clone());
                            metrics::counter!("rvoip_media_graph_frames_total").increment(1);
                            if dropped {
                                metrics::counter!("rvoip_media_graph_drops_total", "reason" => "queue-full").increment(1);
                            }
                            if sink.record_offer(now, dropped, &policy) {
                                evict.push(route_id.clone());
                            }
                        }
                    }
                    for route_id in evict {
                        metrics::counter!("rvoip_media_graph_evictions_total", "reason" => "slow-consumer").increment(1);
                        remove_sink(&route_id, &mut sinks, &mut groups);
                    }
                }
            }
        }

        for (_, sink) in sinks.drain() {
            sink.queue.close();
        }
        metrics::gauge!("rvoip_media_graph_sinks").set(0.0);
        debug!("rvoip media graph stopped");
    });

    Ok(MediaGraphHandle {
        commands: command_tx,
        abort: task.abort_handle(),
    })
}

fn remove_sink(
    route_id: &MediaRouteId,
    sinks: &mut HashMap<MediaRouteId, SinkRuntime>,
    groups: &mut HashMap<u8, CodecGroup>,
) {
    let Some(sink) = sinks.remove(route_id) else {
        return;
    };
    if let Some(group) = groups.get_mut(&sink.target_pt) {
        group.sinks.remove(route_id);
    }
    groups.retain(|_, group| !group.sinks.is_empty());
    metrics::gauge!("rvoip_media_graph_sinks").set(sinks.len() as f64);
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Utc;

    use super::*;
    use crate::ids::StreamId;
    use crate::stream::StreamKind;

    fn codec(name: &str, clock_rate: u32) -> CodecInfo {
        CodecInfo {
            name: name.into(),
            clock_rate_hz: clock_rate,
            channels: 1,
            fmtp: None,
        }
    }

    fn frame(value: u8) -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::new(),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![value; 160]),
            timestamp_rtp: value as u32 * 160,
            captured_at: Utc::now(),
            payload_type: Some(0),
        }
    }

    #[tokio::test]
    async fn one_source_reaches_multiple_sinks() {
        let (source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (a_tx, mut a_rx) = mpsc::channel(4);
        let (b_tx, mut b_rx) = mpsc::channel(4);
        graph.add_sink(codec("pcmu", 8_000), a_tx).unwrap();
        graph.add_sink(codec("pcmu", 8_000), b_tx).unwrap();
        tokio::task::yield_now().await;

        source_tx.send(frame(7)).await.unwrap();
        assert_eq!(a_rx.recv().await.unwrap().payload[0], 7);
        assert_eq!(b_rx.recv().await.unwrap().payload[0], 7);
        graph.shutdown();
    }

    #[tokio::test]
    async fn removing_one_sink_does_not_stop_others() {
        let (source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (a_tx, mut a_rx) = mpsc::channel(4);
        let (b_tx, mut b_rx) = mpsc::channel(4);
        let a = graph.add_sink(codec("pcmu", 8_000), a_tx).unwrap();
        graph.add_sink(codec("pcmu", 8_000), b_tx).unwrap();
        tokio::task::yield_now().await;
        graph.remove_sink(a);
        tokio::task::yield_now().await;

        source_tx.send(frame(9)).await.unwrap();
        let removed = tokio::time::timeout(Duration::from_millis(50), a_rx.recv()).await;
        assert!(matches!(removed, Ok(None) | Err(_)));
        assert_eq!(b_rx.recv().await.unwrap().payload[0], 9);
        graph.shutdown();
    }
}
