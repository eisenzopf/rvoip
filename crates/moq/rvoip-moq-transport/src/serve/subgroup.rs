// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A stream is a stream of objects with a header, split into a [Writer] and [Reader] handle.
//!
//! A [Writer] writes an ordered stream of objects.
//! Each object can have a sequence number, allowing the reader to detect gaps objects.
//!
//! A [Reader] reads an ordered stream of objects.
//! The reader can be cloned, in which case each reader receives a copy of each object. (fanout)
//!
//! The stream is closed with [ServeError::Closed] when all writers or readers are dropped.
use std::{cmp, ops::Deref, sync::Arc};

use bytes::Bytes;

use crate::data::{self, ObjectStatus};
use crate::watch::State;

use super::{ServeError, Track};

pub struct Subgroups {
    pub track: Arc<Track>,
}

impl Subgroups {
    pub fn produce(self) -> (SubgroupsWriter, SubgroupsReader) {
        let (writer, reader) = State::default().split();

        let writer = SubgroupsWriter::new(writer, self.track.clone());
        let reader = SubgroupsReader::new(reader, self.track);

        (writer, reader)
    }
}

impl Deref for Subgroups {
    type Target = Track;

    fn deref(&self) -> &Self::Target {
        &self.track
    }
}

// State shared between the writer and reader.
struct SubgroupsState {
    latest_subgroup_reader: Option<SubgroupReader>,
    epoch: u64, // Updated each time latest changes
    closed: Result<(), ServeError>,
}

impl Default for SubgroupsState {
    fn default() -> Self {
        Self {
            latest_subgroup_reader: None,
            epoch: 0,
            closed: Ok(()),
        }
    }
}

pub struct SubgroupsWriter {
    pub info: Arc<Track>,
    state: State<SubgroupsState>,
    next_subgroup_id: Option<u64>, // None is an overflow tombstone.
    next_group_id: Option<u64>,    // None is an overflow tombstone.
    last_group_id: u64,            // Not in the state to avoid a lock
}

impl SubgroupsWriter {
    fn new(state: State<SubgroupsState>, track: Arc<Track>) -> Self {
        Self {
            info: track,
            state,
            next_subgroup_id: Some(0),
            next_group_id: Some(0),
            last_group_id: 0,
        }
    }

    // Helper to increment the group by one.
    pub fn append(&mut self, priority: u8) -> Result<SubgroupWriter, ServeError> {
        self.append_with_end_of_group(priority, false)
    }

    /// Append a subgroup for an original publisher and explicitly state
    /// whether it completes the Group.
    ///
    /// [`Self::append`] remains conservative for legacy callers and defaults
    /// this assertion to false.
    pub fn append_with_end_of_group(
        &mut self,
        priority: u8,
        end_of_group: bool,
    ) -> Result<SubgroupWriter, ServeError> {
        let group_id;
        let subgroup_id;

        // TODO: refactor here... For now, every subgroup is mapped to a new group...
        let start_new_group = true;

        if start_new_group {
            group_id = self.next_group_id.ok_or(ServeError::Done)?;
            subgroup_id = 0;
        } else {
            group_id = self.last_group_id;
            subgroup_id = self.next_subgroup_id.ok_or(ServeError::Done)?;
        }

        self.create(Subgroup::new(group_id, subgroup_id, priority).with_end_of_group(end_of_group))
    }

    /// Create a new subgroup with the given parameters, inserting it into the track.
    pub fn create(&mut self, subgroup: Subgroup) -> Result<SubgroupWriter, ServeError> {
        let subgroup = SubgroupInfo {
            track: self.info.clone(),
            group_id: subgroup.group_id,
            subgroup_id: subgroup.subgroup_id,
            priority: subgroup.priority,
            first_object: subgroup.first_object,
            end_of_group: subgroup.end_of_group,
        };
        let (writer, reader) = subgroup.produce();

        let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;

        if let Some(latest) = &state.latest_subgroup_reader {
            // TODO: Check this logic again
            if writer.group_id.cmp(&latest.group_id) == cmp::Ordering::Equal {
                match writer.subgroup_id.cmp(&latest.subgroup_id) {
                    cmp::Ordering::Less => return Ok(writer), // dropped immediately, lul
                    cmp::Ordering::Equal => return Err(ServeError::Duplicate),
                    cmp::Ordering::Greater => state.latest_subgroup_reader = Some(reader),
                }
            } else if writer.group_id.cmp(&latest.group_id) == cmp::Ordering::Greater {
                state.latest_subgroup_reader = Some(reader);
            } else {
                return Ok(writer); // drop here as well
            }
        } else {
            state.latest_subgroup_reader = Some(reader);
        }

        let latest = state.latest_subgroup_reader.as_ref().unwrap();
        self.next_subgroup_id = latest.subgroup_id.checked_add(1);
        self.next_group_id = latest.group_id.checked_add(1);
        self.last_group_id = latest.group_id;
        state.epoch += 1;

        Ok(writer)
    }

    /// Close the segment with an error.
    pub fn close(self, err: ServeError) -> Result<(), ServeError> {
        let state = self.state.lock();
        state.closed.clone()?;

        let mut state = state.into_mut().ok_or(ServeError::Cancel)?;
        state.closed = Err(err);

        Ok(())
    }
}

impl Deref for SubgroupsWriter {
    type Target = Track;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

#[derive(Clone)]
pub struct SubgroupsReader {
    pub info: Arc<Track>,
    state: State<SubgroupsState>,
    epoch: u64,
}

impl SubgroupsReader {
    fn new(state: State<SubgroupsState>, track_info: Arc<Track>) -> Self {
        Self {
            info: track_info,
            state,
            epoch: 0,
        }
    }

    pub async fn next(&mut self) -> Result<Option<SubgroupReader>, ServeError> {
        loop {
            {
                let state = self.state.lock();

                if self.epoch != state.epoch {
                    self.epoch = state.epoch;
                    return Ok(state.latest_subgroup_reader.clone());
                }

                state.closed.clone()?;
                match state.modified() {
                    Some(notify) => notify,
                    None => return Ok(None),
                }
            }
            .await; // Try again when the state changes
        }
    }

    // Returns the largest group/sequence
    pub fn latest(&self) -> Option<(u64, u64)> {
        let state = self.state.lock();
        state
            .latest_subgroup_reader
            .as_ref()
            .and_then(|group| group.latest().map(|object_id| (group.group_id, object_id)))
    }

    /// Check if the subgroups writer has been closed or dropped.
    pub fn is_closed(&self) -> bool {
        let state = self.state.lock();
        state.closed.is_err() || state.modified().is_none()
    }
}

impl Deref for SubgroupsReader {
    type Target = Track;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

/// Parameters that can be specified by the user
#[derive(Debug, Clone, PartialEq)]
pub struct Subgroup {
    // The sequence number of the group within the track.
    // NOTE: These may be received out of order or with gaps.
    pub group_id: u64,

    // The sequence number of the subgroup within the group.
    // NOTE: These may be received out of order or with gaps.
    pub subgroup_id: u64,

    // The priority of the group within the track.
    pub priority: u8,

    /// Whether this stream begins with the first Object ever published in the
    /// Subgroup. Draft-19 requires original publishers, and relays that retain
    /// that first Object, to preserve this signal.
    pub first_object: bool,

    /// Whether this stream contains the largest Object in its Group. A clean
    /// stream FIN then communicates that larger Object IDs do not exist.
    pub end_of_group: bool,
}

impl Subgroup {
    /// Construct a subgroup for an original publisher.
    ///
    /// The legacy/default behavior begins with the original first Object but
    /// makes no claim that the subgroup completes its Group. Publishers must
    /// opt in to [`Self::with_end_of_group`] only when that fact is known.
    pub const fn new(group_id: u64, subgroup_id: u64, priority: u8) -> Self {
        Self {
            group_id,
            subgroup_id,
            priority,
            first_object: true,
            end_of_group: false,
        }
    }

    /// Override whether this stream begins with the originally published
    /// first Object. Relays use this when their cache starts later.
    pub const fn with_first_object(mut self, first_object: bool) -> Self {
        self.first_object = first_object;
        self
    }

    /// Mark whether this subgroup contains the largest Object in its Group.
    pub const fn with_end_of_group(mut self, end_of_group: bool) -> Self {
        self.end_of_group = end_of_group;
        self
    }

    /// Convert a validated draft-19 wire header into serve metadata.
    ///
    /// This is the common relay boundary: it rejects non-subgroup and
    /// unresolved subgroup-ID combinations while preserving both independent
    /// FIRST_OBJECT and END_OF_GROUP assertions.
    pub fn from_header(header: &data::SubgroupHeader) -> Result<Self, ServeError> {
        if !header.header_type.is_subgroup() {
            return Err(ServeError::internal_ctx(
                "cannot create serve subgroup from a non-subgroup header",
            ));
        }

        let subgroup_id = header
            .resolved_subgroup_id()
            .map_err(|err| ServeError::internal_ctx(format!("invalid subgroup id: {err}")))?
            .ok_or_else(|| {
                ServeError::internal_ctx(
                    "FIRST_OBJECT subgroup id was not resolved before creating the subgroup",
                )
            })?;

        Ok(Self {
            group_id: header.group_id,
            subgroup_id,
            priority: header.publisher_priority,
            first_object: header.header_type.is_first_object(),
            end_of_group: header.header_type.contains_end_of_group(),
        })
    }
}

/// Static information about the group
#[derive(Debug, Clone, PartialEq)]
pub struct SubgroupInfo {
    pub track: Arc<Track>,

    // The sequence number of the group within the track.
    // NOTE: These may be received out of order or with gaps.
    pub group_id: u64,

    // The sequence number of the subgroup within the group.
    // NOTE: These may be received out of order or with gaps.
    pub subgroup_id: u64,

    // The priority of the group within the track.
    pub priority: u8,

    /// Whether this stream begins with the original first Object.
    pub first_object: bool,

    /// Whether this stream contains the largest Object in its Group.
    pub end_of_group: bool,
}

impl SubgroupInfo {
    pub fn produce(self) -> (SubgroupWriter, SubgroupReader) {
        let (writer, reader) = State::default().split();
        let info = Arc::new(self);

        let writer = SubgroupWriter::new(writer, info.clone());
        let reader = SubgroupReader::new(reader, info);

        (writer, reader)
    }
}

impl Deref for SubgroupInfo {
    type Target = Track;

    fn deref(&self) -> &Self::Target {
        &self.track
    }
}

struct SubgroupState {
    // The data that has been received thus far.
    objects: Vec<SubgroupObjectReader>,

    // Set when the writer or all readers are dropped.
    closed: Result<(), ServeError>,
}

impl Default for SubgroupState {
    fn default() -> Self {
        Self {
            objects: Vec::new(),
            closed: Ok(()),
        }
    }
}

/// Used to write data to a stream and notify readers.
pub struct SubgroupWriter {
    // Mutable stream state.
    state: State<SubgroupState>,

    // Immutable stream state.
    pub info: Arc<SubgroupInfo>,

    // The next object sequence number to use.
    next_object_id: u64,
}

impl SubgroupWriter {
    fn new(state: State<SubgroupState>, group: Arc<SubgroupInfo>) -> Self {
        Self {
            state,
            info: group,
            next_object_id: 0,
        }
    }

    /// Create the next object ID with the given payload.
    pub fn write(&mut self, payload: bytes::Bytes) -> Result<(), ServeError> {
        let mut object = self.create(payload.len(), None)?;
        object.write(payload)?;
        Ok(())
    }

    /// Write an object over multiple writes.
    ///
    /// BAD STUFF will happen if the size is wrong; this is an advanced feature.
    pub fn create(
        &mut self,
        size: usize,
        extension_headers: Option<crate::data::ExtensionHeaders>,
    ) -> Result<SubgroupObjectWriter, ServeError> {
        let (writer, reader) = SubgroupObject {
            group: self.info.clone(),
            object_id: self.next_object_id,
            status: ObjectStatus::NormalObject,
            size,
            extension_headers: extension_headers.unwrap_or_default(),
        }
        .produce();

        self.next_object_id += 1;

        let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;
        state.objects.push(reader);

        Ok(writer)
    }

    /// Close the stream with an error.
    pub fn close(self, err: ServeError) -> Result<(), ServeError> {
        let state = self.state.lock();
        state.closed.clone()?;

        let mut state = state.into_mut().ok_or(ServeError::Cancel)?;
        state.closed = Err(err);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.state.lock().objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Deref for SubgroupWriter {
    type Target = SubgroupInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

/// Notified when a stream has new data available.
#[derive(Clone)]
pub struct SubgroupReader {
    // Modify the stream state.
    state: State<SubgroupState>,

    // Immutable stream state.
    pub info: Arc<SubgroupInfo>,

    // The number of chunks that we've read.
    // NOTE: Cloned readers inherit this index, but then run in parallel.
    read_index: usize,
}

impl SubgroupReader {
    fn new(state: State<SubgroupState>, subgroup: Arc<SubgroupInfo>) -> Self {
        Self {
            state,
            info: subgroup,
            read_index: 0,
        }
    }

    pub fn latest(&self) -> Option<u64> {
        let state = self.state.lock();
        state.objects.last().map(|o| o.object_id)
    }

    pub async fn read_next(&mut self) -> Result<Option<Bytes>, ServeError> {
        let object = self.next().await?;
        match object {
            Some(mut object) => Ok(Some(object.read_all().await?)),
            None => Ok(None),
        }
    }

    pub async fn next(&mut self) -> Result<Option<SubgroupObjectReader>, ServeError> {
        loop {
            {
                let state = self.state.lock();

                if self.read_index < state.objects.len() {
                    let object = state.objects[self.read_index].clone();
                    self.read_index += 1;
                    return Ok(Some(object));
                }

                state.closed.clone()?;
                match state.modified() {
                    Some(notify) => notify,
                    None => return Ok(None),
                }
            }
            .await; // Try again when the state changes
        }
    }

    pub fn pos(&self) -> usize {
        self.read_index
    }

    pub fn len(&self) -> usize {
        self.state.lock().objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Deref for SubgroupReader {
    type Target = SubgroupInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

/// A subset of Object, since we use the group's info.
#[derive(Clone, PartialEq, Debug)]
pub struct SubgroupObject {
    pub group: Arc<SubgroupInfo>,

    pub object_id: u64,

    // The size of the object.
    pub size: usize,

    // Object status
    pub status: ObjectStatus,

    // Extension headers (for draft-14 compliance, particularly immutable extensions)
    pub extension_headers: crate::data::ExtensionHeaders,
}

impl SubgroupObject {
    pub fn produce(self) -> (SubgroupObjectWriter, SubgroupObjectReader) {
        let (writer, reader) = State::default().split();
        let info = Arc::new(self);

        let writer = SubgroupObjectWriter::new(writer, info.clone());
        let reader = SubgroupObjectReader::new(reader, info);

        (writer, reader)
    }
}

impl Deref for SubgroupObject {
    type Target = SubgroupInfo;

    fn deref(&self) -> &Self::Target {
        &self.group
    }
}

struct SubgroupObjectState {
    // The data that has been received thus far.
    chunks: Vec<Bytes>,

    // Set when the writer is dropped.
    closed: Result<(), ServeError>,
}

impl Default for SubgroupObjectState {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            closed: Ok(()),
        }
    }
}

/// Used to write data to a segment and notify readers.
pub struct SubgroupObjectWriter {
    // Mutable segment state.
    state: State<SubgroupObjectState>,

    // Immutable segment state.
    pub info: Arc<SubgroupObject>,

    // The amount of promised data that has yet to be written.
    remain: usize,
}

impl SubgroupObjectWriter {
    /// Create a new segment with the given info.
    fn new(state: State<SubgroupObjectState>, object: Arc<SubgroupObject>) -> Self {
        Self {
            state,
            remain: object.size,
            info: object,
        }
    }

    /// Write a new chunk of bytes.
    pub fn write(&mut self, chunk: Bytes) -> Result<(), ServeError> {
        if chunk.len() > self.remain {
            return Err(ServeError::Size);
        }
        self.remain -= chunk.len();

        let mut state = self.state.lock_mut().ok_or(ServeError::Cancel)?;
        state.chunks.push(chunk);

        Ok(())
    }

    /// Close the segment with an error.
    pub fn close(self, err: ServeError) -> Result<(), ServeError> {
        if self.remain != 0 {
            return Err(ServeError::Size);
        }

        let state = self.state.lock();
        state.closed.clone()?;

        let mut state = state.into_mut().ok_or(ServeError::Cancel)?;
        state.closed = Err(err);

        Ok(())
    }
}

impl Drop for SubgroupObjectWriter {
    fn drop(&mut self) {
        if self.remain == 0 {
            return;
        }

        if let Some(mut state) = self.state.lock_mut() {
            state.closed = Err(ServeError::Size);
        }
    }
}

impl Deref for SubgroupObjectWriter {
    type Target = SubgroupObject;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

/// Notified when a segment has new data available.
#[derive(Clone)]
pub struct SubgroupObjectReader {
    // Modify the segment state.
    state: State<SubgroupObjectState>,

    // Immutable segment state.
    pub info: Arc<SubgroupObject>,

    // The number of chunks that we've read.
    // NOTE: Cloned readers inherit this index, but then run in parallel.
    index: usize,
}

impl SubgroupObjectReader {
    fn new(state: State<SubgroupObjectState>, object: Arc<SubgroupObject>) -> Self {
        Self {
            state,
            info: object,
            index: 0,
        }
    }

    /// Block until the next chunk of bytes is available.
    pub async fn read(&mut self) -> Result<Option<Bytes>, ServeError> {
        loop {
            {
                let state = self.state.lock();

                if self.index < state.chunks.len() {
                    let chunk = state.chunks[self.index].clone();
                    self.index += 1;
                    return Ok(Some(chunk));
                }

                state.closed.clone()?;
                match state.modified() {
                    Some(notify) => notify,
                    None => return Ok(None), // No more changes will come
                }
            }
            .await; // Try again when the state changes
        }
    }

    pub async fn read_all(&mut self) -> Result<Bytes, ServeError> {
        let mut chunks = Vec::new();
        while let Some(chunk) = self.read().await? {
            chunks.push(chunk);
        }

        Ok(Bytes::from(chunks.concat()))
    }
}

impl Deref for SubgroupObjectReader {
    type Target = SubgroupObject;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding::TrackNamespace;
    use crate::data::{StreamHeaderType, SubgroupHeader, SubgroupIdMode};

    #[test]
    fn legacy_construction_uses_safe_draft19_defaults() {
        let (track, _reader) =
            Track::new(TrackNamespace::from_utf8_path("live"), "audio").produce();
        let mut groups = track.subgroups().unwrap();
        let relayed = groups
            .create(Subgroup::new(1, 2, 3).with_first_object(false))
            .unwrap();
        assert!(!relayed.first_object);
        assert!(!relayed.end_of_group);

        let original = groups.append(3).unwrap();
        assert!(original.first_object);
        assert!(!original.end_of_group);

        let complete = groups.append_with_end_of_group(3, true).unwrap();
        assert!(complete.first_object);
        assert!(complete.end_of_group);
    }

    #[test]
    fn all_legal_first_and_end_flag_combinations_survive_the_model() {
        for first_object in [false, true] {
            for end_of_group in [false, true] {
                let header_type = StreamHeaderType::subgroup(
                    true,
                    SubgroupIdMode::Explicit,
                    end_of_group,
                    false,
                    first_object,
                );
                let subgroup = Subgroup::from_header(&SubgroupHeader {
                    header_type,
                    track_alias: 1,
                    group_id: 2,
                    subgroup_id: Some(3),
                    publisher_priority: 4,
                })
                .unwrap();

                assert_eq!(subgroup.first_object, first_object);
                assert_eq!(subgroup.end_of_group, end_of_group);
            }
        }
    }

    #[test]
    fn relay_preserves_first_object_and_end_of_group_metadata() {
        let header_type =
            StreamHeaderType::subgroup(true, SubgroupIdMode::Explicit, true, false, true);
        let subgroup = Subgroup::from_header(&SubgroupHeader {
            header_type,
            track_alias: 1,
            group_id: 2,
            subgroup_id: Some(3),
            publisher_priority: 4,
        })
        .unwrap();

        let (track, _reader) =
            Track::new(TrackNamespace::from_utf8_path("relay"), "audio").produce();
        let mut groups = track.subgroups().unwrap();
        let relayed = groups.create(subgroup).unwrap();
        assert!(relayed.first_object);
        assert!(relayed.end_of_group);
    }

    #[test]
    fn wire_conversion_rejects_non_subgroup_and_unresolved_id_headers() {
        let non_subgroup = SubgroupHeader {
            header_type: StreamHeaderType::Fetch,
            track_alias: 1,
            group_id: 2,
            subgroup_id: None,
            publisher_priority: 3,
        };
        assert!(Subgroup::from_header(&non_subgroup).is_err());

        let unresolved_type =
            StreamHeaderType::subgroup(true, SubgroupIdMode::FirstObject, false, false, true);
        let unresolved = SubgroupHeader {
            header_type: unresolved_type,
            track_alias: 1,
            group_id: 2,
            subgroup_id: None,
            publisher_priority: 3,
        };
        assert!(Subgroup::from_header(&unresolved).is_err());
    }

    #[test]
    fn maximum_group_and_subgroup_ids_tombstone_append_cursors_without_panicking() {
        let (track, _reader) =
            Track::new(TrackNamespace::from_utf8_path("live"), "audio").produce();
        let mut groups = track.subgroups().unwrap();
        let maximum = groups.create(Subgroup::new(u64::MAX, u64::MAX, 0)).unwrap();
        assert_eq!(maximum.group_id, u64::MAX);
        assert_eq!(maximum.subgroup_id, u64::MAX);
        drop(maximum);

        assert!(matches!(groups.append(0), Err(ServeError::Done)));
    }
}
