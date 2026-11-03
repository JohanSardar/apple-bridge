use lab_wire::{Frame, RelayProof, RpcMessage, frame_bytes, ipc_proof, receive, sse_messages};
use serde_json::Value;
fn corpus(name: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../contracts/golden")
        .join(name);
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
#[test]
fn ipc_corpus() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus("ipc.json")?;
    for row in corpus["frames"].as_array().ok_or("frames")? {
        let raw = row["json"].as_str().ok_or("json")?;
        let bytes = frame_bytes(raw.as_bytes())?;
        assert_eq!(hex::encode(&bytes), row["framed_hex"]);
        let (decoded, n) = receive(&mut bytes.as_slice())?;
        assert_eq!(n, bytes.len());
        assert_eq!(decoded, serde_json::from_str::<Value>(raw)?);
    }
    assert_eq!(
        ipc_proof(b"synthetic-ipc-secret", "helper", "nonce-a", "nonce-b")?,
        corpus["hmac_sha256"]
    );
    Ok(())
}
#[test]
fn feed_and_message_frames() -> Result<(), Box<dyn std::error::Error>> {
    let feed = corpus("feed.json")?;
    for raw in feed["frames"].as_array().ok_or("frames")? {
        Frame::decode(raw.as_str().ok_or("frame")?.as_bytes())?;
    }
    let messages = corpus("messages.json")?;
    for key in ["ipc", "feed"] {
        if let Some(frames) = messages[key].as_array() {
            for raw in frames {
                let text = raw
                    .as_str()
                    .or_else(|| raw["json"].as_str())
                    .ok_or("message frame")?;
                Frame::decode(text.as_bytes())?;
            }
        }
    }
    Ok(())
}
#[test]
fn relay_proof() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus("relay.json")?;
    let proof: RelayProof = serde_json::from_str(corpus["proof_json"].as_str().ok_or("proof")?)?;
    assert_eq!(proof.message("tunnel"), corpus["proof_message"]);
    assert_eq!(serde_json::to_string(&proof)?, corpus["proof_json"]);
    assert_eq!(proof.header()?, corpus["X-Bridge-Proof"]);
    Ok(())
}
#[test]
fn mcp_transports() -> Result<(), Box<dyn std::error::Error>> {
    for name in ["mcp-json.json", "mcp-sse.json"] {
        let corpus = corpus(name)?;
        for exchange in corpus["exchanges"].as_array().ok_or("exchanges")? {
            let request: RpcMessage =
                serde_json::from_str(exchange["request"].as_str().ok_or("request")?)?;
            assert_eq!(request.jsonrpc, "2.0");
            let response = exchange["response"].as_str().ok_or("response")?;
            if response.is_empty() {
                continue;
            }
            if exchange["content_type"]
                .as_str()
                .unwrap_or_default()
                .contains("event-stream")
            {
                assert!(!sse_messages(response)?.is_empty());
            } else {
                let decoded: RpcMessage = serde_json::from_str(response)?;
                assert_eq!(decoded.jsonrpc, "2.0");
            }
        }
    }
    Ok(())
}
#[test]
fn malformed_frames() {
    for bytes in [
        b"{}".as_slice(),
        b"[]",
        b"{\"t\":2}",
        b"{\"t\":\"\"}",
        b"{\"t\":\"Ping\"} trailing",
    ] {
        assert!(Frame::decode(bytes).is_err());
    }
    assert!(receive(&mut [0, 4, 0, 1].as_slice()).is_err());
    assert!(receive(&mut [0, 0, 0, 1, b'{'].as_slice()).is_err());
    assert!(receive(&mut [0, 0, 0, 0].as_slice()).is_err());
}

#[test]
fn blob_frames() -> Result<(), Box<dyn std::error::Error>> {
    for row in corpus("blob.json")?["chunks"].as_array().ok_or("chunks")? {
        let bytes = hex::decode(row["hex"].as_str().ok_or("hex")?)?;
        let chunk = lab_wire::BlobChunk::decode(&bytes)?;
        assert_eq!(hex::encode(chunk.digest), row["digest"]);
        assert_eq!(chunk.offset, row["offset"].as_u64().ok_or("offset")?);
        assert_eq!(
            chunk.payload,
            row["payload"].as_str().ok_or("payload")?.as_bytes()
        );
        assert_eq!(chunk.encode()?, bytes);
        let mut malformed = bytes.clone();
        malformed[44] = 1;
        assert!(lab_wire::BlobChunk::decode(&malformed).is_err());
        assert!(lab_wire::BlobChunk::decode(&bytes[..bytes.len() - 1]).is_err());
    }
    Ok(())
}
