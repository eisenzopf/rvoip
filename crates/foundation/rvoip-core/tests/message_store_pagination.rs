#[path = "support/message_store_conformance.rs"]
mod message_store_conformance;

use rvoip_core::store::MemoryMessageStore;

#[tokio::test]
async fn memory_message_store_satisfies_pagination_contract() {
    let store = MemoryMessageStore::new();
    message_store_conformance::assert_message_store_pagination(&store).await;
}
