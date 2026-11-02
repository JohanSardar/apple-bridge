use ab_domain::ErrorCode;
use ab_wire::{
    crypto,
    framing::{self, BlobChunk, Frame, Protocol},
    gojson::EscapeMode,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::Value;

type Result = std::result::Result<(), Box<dyn std::error::Error>>;
fn fixture(raw: &str) -> std::result::Result<Value, serde_json::Error> {
    serde_json::from_str(raw)
}
fn text<'a>(v: &'a Value, key: &str) -> std::result::Result<&'a str, Box<dyn std::error::Error>> {
    v[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}").into())
}

#[test]
fn ipc_golden_frames_and_authentication() -> Result {
    let golden = fixture(include_str!("../../../../contracts/golden/ipc.json"))?;
    for frame in golden["frames"].as_array().ok_or("frames")? {
        let raw = text(frame, "json")?;
        let encoded = Frame::from_json(Protocol::Ipc, raw.as_bytes())?.encode_ipc()?;
        assert_eq!(hex::encode(&encoded), text(frame, "framed_hex")?);
        let (decoded, consumed) = framing::decode_ipc(&encoded)?;
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.encode_json()?, raw.as_bytes());
    }
    assert_eq!(
        hex::encode(crypto::ipc_proof(
            b"synthetic-ipc-secret",
            "helper",
            "nonce-a",
            "nonce-b"
        )?),
        text(&golden, "hmac_sha256")?
    );
    Ok(())
}
#[test]
fn feed_golden_snapshots_batches_and_partial() -> Result {
    let golden = fixture(include_str!("../../../../contracts/golden/feed.json"))?;
    for raw in golden["frames"].as_array().ok_or("frames")? {
        let raw = raw.as_str().ok_or("frame")?;
        assert_eq!(
            Frame::from_json(Protocol::Feed, raw.as_bytes())?.encode_json()?,
            raw.as_bytes()
        );
    }
    Ok(())
}
#[test]
fn relay_and_delegation_golden_signatures() -> Result {
    let relay = fixture(include_str!("../../../../contracts/golden/relay.json"))?;
    let proof: framing::RelayProof = serde_json::from_str(text(&relay, "proof_json")?)?;
    assert_eq!(
        proof.signature_input("tunnel"),
        text(&relay, "proof_message")?.as_bytes()
    );
    let key = SigningKey::from_bytes(&[42; 32]);
    assert_eq!(
        URL_SAFE_NO_PAD.encode(key.sign(&proof.signature_input("tunnel")).to_bytes()),
        proof.signature
    );
    let raw = framing::legacy_json(&proof, EscapeMode::Html)?;
    assert_eq!(raw, text(&relay, "proof_json")?.as_bytes());
    assert_eq!(
        URL_SAFE_NO_PAD.encode(&raw),
        text(&relay, "X-Bridge-Proof")?
    );
    let delegation = fixture(include_str!("../../../../contracts/golden/delegation.json"))?;
    let raw = text(&delegation, "raw")?.as_bytes();
    let typed: crypto::Delegation = serde_json::from_slice(raw)?;
    let header = typed.sign(&key)?;
    assert_eq!(header, text(&delegation, "header")?);
    assert_eq!(
        crypto::verify_delegation(&header, &key.verifying_key())?,
        raw
    );
    assert!(crypto::verify_delegation(&format!("{header}x"), &key.verifying_key()).is_err());
    Ok(())
}
#[test]
fn iphone_golden_packet_both_directions_and_tampering() -> Result {
    let golden = fixture(include_str!("../../../../contracts/golden/iphone.json"))?;
    let packet: crypto::Packet = serde_json::from_value(golden["packet"].clone())?;
    let key = SigningKey::from_bytes(&[42; 32]);
    let plaintext = text(&golden, "plaintext")?.as_bytes();
    let sealed = crypto::seal(
        crypto::public_key([17; 32]).to_bytes(),
        "device-fixture",
        plaintext,
        [23; 32],
        [31; 12],
        &key,
    )?;
    assert_eq!(serde_json::to_value(&sealed)?, golden["packet"]);
    assert_eq!(
        crypto::open([17; 32], "device-fixture", &packet, &key.verifying_key())?,
        plaintext
    );
    assert!(crypto::open([17; 32], "wrong-device", &packet, &key.verifying_key()).is_err());
    let wrong = SigningKey::from_bytes(&[7; 32]);
    assert!(crypto::open([17; 32], "device-fixture", &packet, &wrong.verifying_key()).is_err());
    Ok(())
}
#[test]
fn reject_oversized_frames_and_malformed_blob_headers() -> Result {
    assert_eq!(
        framing::decode_ipc(&u32::MAX.to_be_bytes()).err(),
        Some(ErrorCode::Capacity)
    );
    assert!(framing::decode_ipc(&[0; 4]).is_err());
    for kind in ["Ping", "QueryResult"] {
        let json = format!(
            "{{\"t\":\"{kind}\",\"result_json\":\"{}\"}}",
            "a".repeat(framing::MAX_FRAME)
        );
        assert_eq!(
            Frame::from_json(Protocol::Ipc, json.as_bytes()).is_ok(),
            kind == "QueryResult"
        );
        assert!(Frame::from_json(Protocol::Feed, json.as_bytes()).is_err());
    }
    let response = format!(
        "{{\"t\":\"CommandResponse\",\"response\":{{\"result\":\"{}\"}}}}",
        "a".repeat(framing::MAX_FRAME)
    );
    let response = Frame::from_json(Protocol::Feed, response.as_bytes())?;
    assert!(response.encode_json()?.len() > framing::MAX_FRAME);
    let chunk = BlobChunk {
        digest: [42; 32],
        offset: 0x0102_0304_0506_0708,
        bytes: b"fixture".to_vec(),
    };
    let mut encoded = chunk.encode()?;
    assert_eq!(&encoded[..32], &[42; 32]);
    assert_eq!(&encoded[32..40], &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(&encoded[40..44], &[0, 0, 0, 7]);
    assert_eq!(BlobChunk::decode(&encoded)?, chunk);
    encoded[44] = 1;
    assert_eq!(BlobChunk::decode(&encoded).err(), Some(ErrorCode::Invalid));
    encoded[44] = 0;
    encoded[40] = 1;
    assert_eq!(BlobChunk::decode(&encoded).err(), Some(ErrorCode::Corrupt));
    Ok(())
}

#[test]
fn blob_golden_header_and_payload() -> Result {
    let golden = fixture(include_str!("../../../../contracts/golden/blob.json"))?;
    for row in golden["chunks"].as_array().ok_or("chunks")? {
        let chunk = BlobChunk {
            digest: hex::decode(text(row, "digest")?)?
                .try_into()
                .map_err(|_| "digest")?,
            offset: row["offset"].as_i64().ok_or("offset")?,
            bytes: text(row, "payload")?.as_bytes().to_vec(),
        };
        let encoded = chunk.encode()?;
        assert_eq!(hex::encode(&encoded), text(row, "hex")?);
        assert_eq!(BlobChunk::decode(&encoded)?, chunk);
    }
    Ok(())
}

#[test]
fn oauth_and_passkey_legacy_json_documents() -> Result {
    let oauth = fixture(include_str!("../../../../contracts/golden/oauth.json"))?;
    for exchange in oauth["exchanges"].as_array().ok_or("exchanges")? {
        let raw = text(exchange, "response")?.trim_end_matches('\n');
        assert_eq!(
            ab_wire::gojson::legacy_raw(raw.as_bytes(), EscapeMode::Html)?,
            raw.as_bytes()
        );
    }
    let passkey = fixture(include_str!("../../../../contracts/golden/passkey.json"))?;
    for (name, value) in passkey.as_object().ok_or("passkey")? {
        if value.is_object() {
            let bytes = ab_wire::gojson::legacy_json(value, EscapeMode::Html)?;
            let decoded: Value = serde_json::from_slice(&bytes)?;
            assert_eq!(&decoded, value, "{name}");
        }
    }
    Ok(())
}

#[test]
fn mcp_legacy_json_and_sse_payloads() -> Result {
    for raw in [
        include_str!("../../../../contracts/golden/mcp-json.json"),
        include_str!("../../../../contracts/golden/mcp-sse.json"),
    ] {
        let transcript = fixture(raw)?;
        for exchange in transcript["exchanges"].as_array().ok_or("exchanges")? {
            let response = text(exchange, "response")?;
            if response.is_empty() {
                continue;
            }
            let payload = if response.starts_with("event:") {
                response
                    .lines()
                    .find_map(|l| l.strip_prefix("data: "))
                    .ok_or("data")?
            } else {
                response.trim_end_matches('\n')
            };
            assert_eq!(
                ab_wire::gojson::legacy_raw(payload.as_bytes(), EscapeMode::Html)?,
                payload.as_bytes()
            );
        }
    }
    Ok(())
}

#[test]
fn typed_message_encoders_equal_go_for_every_message_kind() -> Result {
    let golden = fixture(include_str!("../../../../contracts/golden/messages.json"))?;
    for row in golden["ipc"].as_array().ok_or("ipc")? {
        let raw = text(row, "json")?;
        let frame: ab_wire::messages::IpcFrame = serde_json::from_str(raw)?;
        assert_eq!(
            hex::encode(frame.encode()?),
            text(row, "framed_hex")?,
            "{}",
            frame.t
        );
    }
    for row in golden["feed"].as_array().ok_or("feed")? {
        let raw = row.as_str().ok_or("frame")?;
        let frame: ab_wire::messages::FeedFrame = serde_json::from_str(raw)?;
        assert_eq!(frame.encode()?, raw.as_bytes(), "{}", frame.t);
    }
    let ipc = fixture(include_str!("../../../../contracts/golden/ipc.json"))?;
    for row in ipc["frames"].as_array().ok_or("ipc")? {
        let raw = text(row, "json")?;
        let mut frame: ab_wire::messages::IpcFrame = serde_json::from_str(raw)?;
        if frame.t == "Capture" {
            frame.capture = Some(serde_json::from_str(raw)?);
        }
        assert_eq!(hex::encode(frame.encode()?), text(row, "framed_hex")?);
    }
    let feed = fixture(include_str!("../../../../contracts/golden/feed.json"))?;
    for row in feed["frames"].as_array().ok_or("feed")? {
        let raw = row.as_str().ok_or("frame")?;
        let mut frame: ab_wire::messages::FeedFrame = serde_json::from_str(raw)?;
        if frame.t == "Batch" {
            frame.batch = Some(serde_json::from_str(raw)?);
        }
        assert_eq!(frame.encode()?, raw.as_bytes());
    }
    Ok(())
}

#[test]
fn empty_optional_slices_follow_go_omitempty() -> Result {
    use ab_wire::messages::{FeedFrame, IpcFrame};
    let base = IpcFrame {
        t: "Ping".into(),
        ..IpcFrame::default()
    };
    let empty = IpcFrame {
        candidate_ids: Some(vec![]),
        ..base.clone()
    };
    assert_eq!(empty.encode()?, base.encode()?);
    let base = FeedFrame {
        t: "Heartbeat".into(),
        ..FeedFrame::default()
    };
    let empty = FeedFrame {
        records: Some(vec![]),
        ..base.clone()
    };
    assert_eq!(empty.encode()?, base.encode()?);
    Ok(())
}
