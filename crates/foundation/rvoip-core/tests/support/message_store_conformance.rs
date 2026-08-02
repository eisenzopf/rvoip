use bytes::Bytes;
use chrono::{Duration, TimeZone, Utc};
use rvoip_core::connection::Direction;
use rvoip_core::error::RvoipError;
use rvoip_core::ids::{ConversationId, MessageId, ParticipantId};
use rvoip_core::message::{ContentType, Message, MessageOrigin, MessageRecipients};
use rvoip_core::store::{MessageFilter, MessageStore, PageCursor};
use std::collections::HashSet;

fn message(
    conversation_id: ConversationId,
    id: &str,
    from: ParticipantId,
    content_type: ContentType,
    second: i64,
) -> Message {
    Message {
        id: MessageId::from_string(id),
        conversation_id,
        origin: MessageOrigin::System,
        from_participant: from,
        to: MessageRecipients::All,
        direction: Direction::Inbound,
        content_type,
        body: Bytes::from_static(b"pagination-conformance"),
        attachments: Vec::new(),
        in_reply_to: None,
        timestamp: Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .expect("valid fixture timestamp")
            + Duration::seconds(second),
    }
}

async fn traverse<S: MessageStore + ?Sized>(
    store: &S,
    conversation_id: &ConversationId,
    filter: MessageFilter,
) -> Vec<String> {
    let mut cursor: Option<PageCursor> = None;
    let mut ids = Vec::new();
    loop {
        let page = store
            .list(conversation_id, filter.clone(), cursor)
            .await
            .expect("conforming store lists a valid page");
        ids.extend(page.messages.into_iter().map(|item| item.id.to_string()));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    ids
}

/// Shared pagination contract for memory and future durable MessageStore
/// implementations. A backend test can call this function with its own store.
pub async fn assert_message_store_pagination<S: MessageStore + ?Sized>(store: &S) {
    let wanted = ParticipantId::from_string("participant-wanted");
    let other = ParticipantId::from_string("participant-other");

    // Exercise zero, one, and many leading raw-log entries that do not match.
    for prefix_misses in 0..=3 {
        let conversation_id =
            ConversationId::from_string(format!("pagination-prefix-{prefix_misses}"));
        for index in 0..prefix_misses {
            store
                .put(message(
                    conversation_id.clone(),
                    &format!("prefix-{prefix_misses}-miss-{index}"),
                    other.clone(),
                    ContentType::Binary,
                    index as i64,
                ))
                .await
                .expect("insert prefix miss");
        }
        for index in 0..5 {
            store
                .put(message(
                    conversation_id.clone(),
                    &format!("prefix-{prefix_misses}-match-{index}"),
                    wanted.clone(),
                    ContentType::Text,
                    (prefix_misses + index) as i64,
                ))
                .await
                .expect("insert match");
        }
        let filter = MessageFilter {
            from_participant: Some(wanted.clone()),
            page_size: Some(2),
            ..MessageFilter::default()
        };
        let first = traverse(store, &conversation_id, filter.clone()).await;
        let second = traverse(store, &conversation_id, filter).await;
        assert_eq!(first, second, "immutable traversal must be deterministic");
        assert_eq!(first.len(), 5);
        assert_eq!(first.iter().collect::<HashSet<_>>().len(), first.len());
    }

    // Sparse matches exercise page sizes one, exact multiple, below the match
    // count, and above it without duplicates, omissions, or an empty tail page.
    let sparse_id = ConversationId::from_string("pagination-sparse");
    for index in 0..12 {
        let is_match = matches!(index, 1 | 4 | 7 | 10);
        store
            .put(message(
                sparse_id.clone(),
                &format!("sparse-{index}"),
                if is_match {
                    wanted.clone()
                } else {
                    other.clone()
                },
                if is_match {
                    ContentType::Json
                } else {
                    ContentType::Binary
                },
                index,
            ))
            .await
            .expect("insert sparse fixture");
    }
    let expected = vec!["sparse-1", "sparse-4", "sparse-7", "sparse-10"];
    for page_size in [1, 2, 3, 4, 10] {
        let filter = MessageFilter {
            from_participant: Some(wanted.clone()),
            content_types: Some(vec![ContentType::Json]),
            page_size: Some(page_size),
            ..MessageFilter::default()
        };
        assert_eq!(traverse(store, &sparse_id, filter).await, expected);
    }

    let exact_filter = MessageFilter {
        from_participant: Some(wanted.clone()),
        page_size: Some(2),
        ..MessageFilter::default()
    };
    let first = store
        .list(&sparse_id, exact_filter.clone(), None)
        .await
        .expect("first exact page");
    let second = store
        .list(&sparse_id, exact_filter, first.next)
        .await
        .expect("second exact page");
    assert_eq!(second.messages.len(), 2);
    assert!(
        second.next.is_none(),
        "exact final page must not advertise an empty tail"
    );

    let zero_result = store
        .list(
            &sparse_id,
            MessageFilter {
                page_size: Some(0),
                ..MessageFilter::default()
            },
            None,
        )
        .await;
    let zero_error = match zero_result {
        Err(error) => error,
        Ok(_) => panic!("zero page size must be invalid"),
    };
    assert!(matches!(zero_error, RvoipError::InvalidState(_)));

    // Appends are visible after the cursor because the memory model has no
    // removals and preserves insertion order.
    let append_id = ConversationId::from_string("pagination-append");
    for index in 0..3 {
        store
            .put(message(
                append_id.clone(),
                &format!("append-{index}"),
                wanted.clone(),
                ContentType::Text,
                index,
            ))
            .await
            .expect("insert append fixture");
    }
    let append_filter = MessageFilter {
        page_size: Some(2),
        ..MessageFilter::default()
    };
    let first = store
        .list(&append_id, append_filter.clone(), None)
        .await
        .expect("first append page");
    store
        .put(message(
            append_id.clone(),
            "append-3",
            wanted,
            ContentType::Text,
            3,
        ))
        .await
        .expect("append after first page");
    let second = store
        .list(&append_id, append_filter, first.next)
        .await
        .expect("second append page");
    assert_eq!(
        second
            .messages
            .iter()
            .map(|item| item.id.to_string())
            .collect::<Vec<_>>(),
        ["append-2", "append-3"]
    );
    assert!(second.next.is_none());
}
