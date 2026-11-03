#![allow(clippy::unwrap_used)]
use lab_relay::{
    Ca, Frame, Identity, Link, PairCode, client_config, fingerprint, hello, input1, input2,
    random_nonce, replay_gateway, valid_remote_address,
};
use std::time::Duration;

#[test]
fn remote_addresses_follow_the_gateway_rules() {
    for ok in [
        "https://bridge.example.com",
        "https://bridge.example.com/",
        "https://bridge.example.com:8443",
    ] {
        assert!(valid_remote_address(ok), "{ok}");
    }
    for bad in [
        "http://bridge.example.com",
        "wss://bridge.example.com",
        "https://owner:password@bridge.example.com",
        "https://bridge.example.com/feed",
        "https://bridge.example.com?token=invalid",
        "https://bridge.example.com#fragment",
        "https://bridge.example.com:0",
        "https://bridge.example.com:65536",
    ] {
        assert!(!valid_remote_address(bad), "{bad}");
    }
}
#[test]
fn pair_code_round_trips_and_proof_matches_vector() {
    let secret = data_secret();
    let mut code = PairCode {
        remote: true,
        id: "pair-1".into(),
        address: "https://bridge.example.com".into(),
        fingerprint: "0".repeat(64),
        secret,
        expires_at: "2999-01-01T00:00:00Z".into(),
        ..PairCode::default()
    };
    let decoded = PairCode::decode(&code.encode().unwrap()).unwrap();
    assert_eq!(decoded.id, "pair-1");
    assert!(!decoded.expired());
    code.expire();
    assert!(code.expired());
    let before = code.secret_bytes().unwrap();
    code.flip_secret_bit().unwrap();
    assert_ne!(before, code.secret_bytes().unwrap());
    assert!(PairCode::decode("ab2-AAAA").is_err());
}
fn data_secret() -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    STANDARD.encode([7u8; 20])
}
#[test]
fn transcripts_have_the_published_prefix_and_binding() {
    let identity = Identity::generate().unwrap();
    let first = hello(&identity, true, None);
    let mut second = first_clone(&first);
    second.nonce = random_nonce();
    let one = input2("gateway", &first, &second, None, &[1; 32]).unwrap();
    let other = input2("gateway", &first, &second, None, &[2; 32]).unwrap();
    assert!(one.starts_with(b"apple-bridge-relay/2\0gateway\0relay/2\0tls-exporter,feed/1\0"));
    assert_ne!(one, other);
    let legacy = input1("replica", &first, &second, None).unwrap();
    assert!(legacy.starts_with(b"apple-bridge-relay/1\0replica\0"));
    assert_eq!(fingerprint(&identity.certificate).len(), 64);
}
fn first_clone(frame: &Frame) -> Frame {
    Frame::parse(&frame.text().unwrap()).unwrap()
}
#[test]
fn tls_dial_binds_exporter_and_trusts_only_the_lab_ca() {
    let ca = Ca::new().unwrap();
    let (address, outcome) = replay_gateway(&ca, "{\"t\":\"AuthChallenge\"}".into()).unwrap();
    let config = client_config(ca.roots().unwrap()).unwrap();
    let mut link = Link::dial(&address, &config, None).unwrap();
    assert_ne!(link.binding, [0; 32]);
    link.send_text("{\"t\":\"AuthHello\"}").unwrap();
    assert!(
        link.recv_text(Duration::from_secs(2))
            .unwrap()
            .contains("AuthChallenge")
    );
    link.send_text("{\"t\":\"AuthProof\"}").unwrap();
    assert!(outcome.recv_timeout(Duration::from_secs(3)).unwrap());

    let stranger = Ca::new().unwrap();
    let (address, _outcome) = replay_gateway(&ca, String::new()).unwrap();
    let wrong = client_config(stranger.roots().unwrap()).unwrap();
    assert!(Link::dial(&address, &wrong, None).is_err());
}
