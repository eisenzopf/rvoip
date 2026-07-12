use rvoip_core_traits::broadcast::{
    BroadcastProtocolFamily, BroadcastPublisher, BroadcastResource,
};
use rvoip_moq::{
    LocOpusPacketizer, MoqBroadcastPublisher, MoqCompatibility, MoqNamespace, MoqProtocolVersion,
    MoqPublisherConfig, MsfCatalog, LOC_DRAFT, MOQT_DRAFT, MSF_DRAFT,
};

#[tokio::test]
async fn application_contract_uses_only_rvoip_owned_models() {
    let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
    let catalog = MsfCatalog::opus_audio(&namespace, 24_000, Some("en".into()), 0).unwrap();
    catalog.validate().unwrap();
    let _packetizer = LocOpusPacketizer::new();
    assert_eq!(
        MoqCompatibility::PINNED
            .require(MoqProtocolVersion::PINNED)
            .unwrap(),
        MoqProtocolVersion::PINNED
    );

    let publisher = MoqBroadcastPublisher::new(MoqPublisherConfig {
        tenant_id: "tenant".into(),
        broadcast_id: "broadcast".into(),
        bitrate: 24_000,
        language: Some("en".into()),
        queue_frames: 10,
    })
    .unwrap();
    let protocol = publisher.protocol();
    assert_eq!(protocol.family, BroadcastProtocolFamily::Moqt);
    assert_eq!(protocol.transport_version, MOQT_DRAFT);
    assert_eq!(protocol.media_format_version.as_deref(), Some(MSF_DRAFT));
    assert_eq!(protocol.object_format_version.as_deref(), Some(LOC_DRAFT));
    assert!(matches!(
        publisher.endpoint().resource,
        BroadcastResource::Moqt { .. }
    ));
    publisher.close().await.unwrap();
}
