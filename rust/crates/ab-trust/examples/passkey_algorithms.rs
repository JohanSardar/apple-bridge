use ab_trust::{Authority, passkeys};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use ed25519_dalek::{Signer, SigningKey};
use serde_cbor_2::Value as Cbor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::args().nth(1).ok_or("isolated home required")?;
    let fixture: Value = serde_json::from_slice(&std::fs::read("contracts/golden/passkey.json")?)?;
    let authority = Authority::open(&home)?;
    let key = SigningKey::from_bytes(&[42; 32]);
    let cose = Cbor::Map(BTreeMap::from([
        (Cbor::Integer(1), Cbor::Integer(1)),
        (Cbor::Integer(3), Cbor::Integer(-8)),
        (Cbor::Integer(-1), Cbor::Integer(6)),
        (
            Cbor::Integer(-2),
            Cbor::Bytes(key.verifying_key().to_bytes().to_vec()),
        ),
    ]));
    let (options, mut state) = passkeys::begin("localhost", "http://localhost:7780", false, &[])?;
    assert_eq!(
        options["publicKey"]["pubKeyCredParams"],
        json!([{"type":"public-key","alg":-7},{"type":"public-key","alg":-8}])
    );
    state["challenge"] = fixture["enroll_options"]["publicKey"]["challenge"].clone();
    let mut response = fixture["enroll_credential"].clone();
    let mut object: BTreeMap<String, Cbor> = serde_cbor_2::from_slice(
        &URL_SAFE_NO_PAD.decode(
            response["response"]["attestationObject"]
                .as_str()
                .ok_or("attestation")?,
        )?,
    )?;
    let Cbor::Bytes(auth) = object.get_mut("authData").ok_or("authData")? else {
        return Err("authData".into());
    };
    auth.truncate(87);
    auth.extend(serde_cbor_2::to_vec(&cose)?);
    response["response"]["attestationObject"] = URL_SAFE_NO_PAD
        .encode(serde_cbor_2::to_vec(&object)?)
        .into();
    let registered =
        passkeys::finish_registration("localhost", "http://localhost:7780", state, &response)?;
    assert_eq!(
        registered.credential["publicKey"],
        STANDARD.encode(serde_cbor_2::to_vec(&cose)?)
    );
    let enroll = authority.create_web_enroll()?;
    authority.save_passkey(registered.clone(), &enroll, None)?;
    let (_, mut login) =
        passkeys::begin("localhost", "http://localhost:7780", true, &[registered])?;
    login["challenge"] = fixture["login_options"]["publicKey"]["challenge"].clone();
    let mut assertion = fixture["login_credential"].clone();
    let mut signed = URL_SAFE_NO_PAD.decode(
        assertion["response"]["authenticatorData"]
            .as_str()
            .ok_or("auth")?,
    )?;
    signed.extend(Sha256::digest(
        URL_SAFE_NO_PAD.decode(
            assertion["response"]["clientDataJSON"]
                .as_str()
                .ok_or("client")?,
        )?,
    ));
    assertion["response"]["signature"] =
        URL_SAFE_NO_PAD.encode(key.sign(&signed).to_bytes()).into();
    let mut bad = assertion.clone();
    bad["response"]["signature"] = URL_SAFE_NO_PAD.encode([0; 64]).into();
    assert!(
        authority
            .finish_passkey_login("localhost", "http://localhost:7780", login.clone(), bad)
            .is_err()
    );
    authority.finish_passkey_login("localhost", "http://localhost:7780", login, assertion)?;
    let path = Path::new(&home).join("credentials.json");
    let original = std::fs::read(&path)?;
    for (alg, curve) in [(-257, 1), (-35, 2), (-8, 7)] {
        let mut stored: Value = serde_json::from_slice(&original)?;
        let mut unsupported = cose.clone();
        let Cbor::Map(map) = &mut unsupported else {
            return Err("map".into());
        };
        map.insert(Cbor::Integer(3), Cbor::Integer(alg));
        map.insert(Cbor::Integer(-1), Cbor::Integer(curve));
        stored["passkeys"][0]["credential"]["publicKey"] =
            STANDARD.encode(serde_cbor_2::to_vec(&unsupported)?).into();
        let bytes = serde_json::to_vec(&stored)?;
        std::fs::write(&path, &bytes)?;
        assert!(
            passkeys::validate_home(Path::new(&home))
                .err()
                .ok_or("accepted unsupported key")?
                .to_string()
                .contains("re-enrol")
        );
        assert!(Authority::open(&home).is_err());
        assert_eq!(std::fs::read(&path)?, bytes);
    }
    std::fs::write(&path, original)?;
    println!(
        "{}",
        json!({"eddsa_registration":true,"eddsa_login":true,"tampered_signature_rejected":true,"unsupported_startup_rejected_without_mutation":3})
    );
    Ok(())
}
