#![allow(clippy::unwrap_used)]

use ab_native::{
    plist,
    rich::{self, MessageReaction},
};

#[test]
fn keyed_archive_truncations_and_invalid_graphs() {
    let data = include_bytes!("../../../../lab/fixtures/messages/rich-link.plist");
    for length in 0..data.len() {
        assert!(plist::archive(&data[..length]).is_err(), "{length}");
    }
    for bytes in [
        include_bytes!("../../../../lab/fixtures/messages/rich-cycle.plist").as_slice(),
        include_bytes!("../../../../lab/fixtures/messages/rich-bad-reference.plist").as_slice(),
    ] {
        assert!(plist::archive(bytes).is_err());
    }
}

#[test]
fn reaction_mappings_and_targets() {
    let kinds = [
        "love",
        "like",
        "dislike",
        "laugh",
        "emphasize",
        "question",
        "emoji",
        "sticker",
    ];
    for (index, kind) in kinds.iter().enumerate() {
        for (base, removed) in [(2000, false), (3000, true)] {
            let reaction = MessageReaction::decode(
                base + i64::try_from(index).unwrap(),
                "p:2/synthetic-parent",
                Some("🧪".into()),
            )
            .unwrap();
            assert_eq!(reaction.message_guid, "synthetic-parent");
            assert_eq!(reaction.part_index, 2);
            assert_eq!(reaction.kind, *kind);
            assert_eq!(reaction.removed, removed);
        }
    }
    for target in ["", "p:-1/x", "p:invalid/x", "p:2/", "bp:"] {
        assert!(MessageReaction::decode(2000, target, None).is_none());
    }
    assert!(MessageReaction::decode(4000, "synthetic", None).is_none());
}

#[test]
fn rich_classification_vectors() {
    for (item, action, member, sender, title, expected) in [
        (1, 0, true, false, "", "member_added"),
        (1, 0, true, true, "", "phone_changed"),
        (99, 0, false, false, "", "unknown"),
        (1, 1, true, false, "", "member_removed"),
        (3, 0, true, false, "", "member_left"),
        (3, 1, true, false, "", "photo_changed"),
        (3, 2, true, false, "", "photo_removed"),
        (2, 0, false, false, "Title", "rename"),
    ] {
        assert_eq!(
            rich::group_event_kind(item, action, member, sender, title),
            expected
        );
    }
    for (bundle, expected) in [
        ("URLBalloonProvider", "link"),
        ("MSMessageExtensionBalloonPlugin:synthetic", "app"),
        ("DigitalTouchBalloonProvider", "digital_touch"),
        ("Handwriting", "handwriting"),
        ("Poll", "poll"),
        ("SafetyMonitor", "check_in"),
        ("PeerPayment", "apple_cash"),
        ("FindMy", "find_my"),
        ("chatbot", "chatbot"),
        ("synthetic.unknown", "unknown"),
    ] {
        assert_eq!(rich::balloon_kind(bundle), expected);
    }
    assert!(rich::sticker(&[0, 1], "synthetic").is_none());
}
