use ab_wire::framing::RelayProof;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

#[test]
fn frozen_relay_corpus() -> Result<(), Box<dyn std::error::Error>> {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("../../../../contracts/golden/relay.json"))?;
    let text = |key: &str| value[key].as_str().ok_or("missing corpus field");
    let proof: RelayProof = serde_json::from_str(text("proof_json")?)?;
    assert_eq!(proof.encode_header()?, text("X-Bridge-Proof")?);
    assert_eq!(
        proof.signature_input("tunnel"),
        text("proof_message")?.as_bytes()
    );
    let bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(text("public_key")?)?
        .try_into()
        .map_err(|_| "key length")?;
    let key = VerifyingKey::from_bytes(&bytes)?;
    key.verify(
        &proof.signature_input("tunnel"),
        &Signature::from_slice(&URL_SAFE_NO_PAD.decode(&proof.signature)?)?,
    )?;
    Ok(())
}
