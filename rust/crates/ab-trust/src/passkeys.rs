use crate::{
    Authority, Error, Principal, fingerprint,
    oauth::{live, read, write},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use webauthn_rs_core::{
    WebauthnCore,
    proto::{
        COSEAlgorithm, COSEKey, Credential, CredentialV3, PublicKeyCredential,
        UserVerificationPolicy,
    },
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Passkey {
    pub id: String,
    pub rp_id: String,
    pub credential: Value,
    pub created_at: String,
}
fn invalid<T>(_: T) -> Error {
    Error::Invalid
}
fn bytes(v: &Value) -> Result<Vec<u8>, Error> {
    STANDARD
        .decode(v.as_str().ok_or(Error::Invalid)?)
        .map_err(invalid)
}
pub fn core(rp: &str, origin: &str) -> Result<WebauthnCore, Error> {
    Ok(WebauthnCore::new_unsafe_experts_only(
        "Bridge",
        rp,
        vec![url::Url::parse(origin).map_err(invalid)?],
        std::time::Duration::from_secs(300),
        Some(false),
        Some(false),
    ))
}
pub fn import(v: &Value) -> Result<Credential, Error> {
    let cose: serde_cbor_2::Value =
        serde_cbor_2::from_slice(&bytes(&v["publicKey"])?).map_err(invalid)?;
    validate_cose(&cose)?;
    let mut c: Credential = CredentialV3 {
        cred_id: bytes(&v["id"])?,
        cred: COSEKey::try_from(&cose).map_err(invalid)?,
        counter: serde_json::from_value(v["authenticator"]["signCount"].clone())?,
        verified: v["flags"]["userVerified"] == true,
        registration_policy: UserVerificationPolicy::Required,
    }
    .into();
    c.backup_eligible = v["flags"]["backupEligible"] == true;
    c.backup_state = v["flags"]["backupState"] == true;
    c.transports = serde_json::from_value(v["transport"].clone()).ok();
    Ok(c)
}
pub fn begin(
    rp: &str,
    origin: &str,
    login: bool,
    keys: &[Passkey],
) -> Result<(Value, Value), Error> {
    let wa = core(rp, origin)?;
    if login {
        let creds = keys
            .iter()
            .map(|k| import(&k.credential))
            .collect::<Result<Vec<_>, _>>()?;
        let builder = wa
            .new_challenge_authenticate_builder(creds, Some(UserVerificationPolicy::Required))
            .map_err(invalid)?;
        let (mut options, state) = wa
            .generate_challenge_authenticate(builder)
            .map_err(invalid)?;
        options.public_key.allow_credentials.clear();
        let mut options = serde_json::to_value(options)?;
        options["publicKey"]
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .remove("allowCredentials");
        Ok((options, serde_json::to_value(state)?))
    } else {
        let builder = wa
            .new_challenge_register_builder(b"apple-bridge-owner", "Bridge owner", "Bridge owner")
            .map_err(invalid)?
            .credential_algorithms(vec![COSEAlgorithm::ES256, COSEAlgorithm::EDDSA])
            .require_resident_key(true)
            .user_verification_policy(UserVerificationPolicy::Required);
        let (options, state) = wa.generate_challenge_register(builder).map_err(invalid)?;
        let mut options = serde_json::to_value(options)?;
        options["publicKey"]
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .remove("attestation");
        Ok((options, serde_json::to_value(state)?))
    }
}
pub fn finish_registration(
    rp: &str,
    origin: &str,
    state: Value,
    response: &Value,
) -> Result<Passkey, Error> {
    let c = core(rp, origin)?
        .register_credential(
            &serde_json::from_value(response.clone())?,
            &serde_json::from_value(state)?,
            None,
        )
        .map_err(invalid)?;
    let object = URL_SAFE_NO_PAD
        .decode(
            response["response"]["attestationObject"]
                .as_str()
                .ok_or(Error::Invalid)?,
        )
        .map_err(invalid)?;
    let object_value: serde_cbor_2::Value = serde_cbor_2::from_slice(&object).map_err(invalid)?;
    let serde_cbor_2::Value::Map(map) = object_value else {
        return Err(Error::Invalid);
    };
    let Some(serde_cbor_2::Value::Bytes(auth)) =
        map.get(&serde_cbor_2::Value::Text("authData".into()))
    else {
        return Err(Error::Invalid);
    };
    let offset = 55 + c.cred_id.len();
    let mut decoder =
        serde_cbor_2::Deserializer::from_slice(auth.get(offset..).ok_or(Error::Invalid)?);
    let cose = serde_cbor_2::Value::deserialize(&mut decoder).map_err(invalid)?;
    let client_data = URL_SAFE_NO_PAD
        .decode(
            response["response"]["clientDataJSON"]
                .as_str()
                .ok_or(Error::Invalid)?,
        )
        .map_err(invalid)?;
    let format = map
        .get(&serde_cbor_2::Value::Text("fmt".into()))
        .and_then(|v| {
            if let serde_cbor_2::Value::Text(s) = v {
                Some(s.as_str())
            } else {
                None
            }
        })
        .ok_or(Error::Invalid)?;
    let credential = json!({"id":STANDARD.encode(&c.cred_id),"publicKey":STANDARD.encode(serde_cbor_2::to_vec(&cose).map_err(invalid)?),"attestationType":format,"attestationFormat":format,"transport":response["response"]["transports"],"flags":{"userPresent":true,"userVerified":c.user_verified,"backupEligible":c.backup_eligible,"backupState":c.backup_state},"authenticator":{"AAGUID":STANDARD.encode(auth.get(37..53).ok_or(Error::Invalid)?),"signCount":c.counter},"attestation":{"clientDataJSON":STANDARD.encode(&client_data),"clientDataHash":STANDARD.encode(Sha256::digest(&client_data)),"object":STANDARD.encode(object)}});
    Ok(Passkey {
        id: URL_SAFE_NO_PAD.encode(c.cred_id),
        rp_id: rp.into(),
        credential,
        created_at: jiff::Timestamp::now().to_string(),
    })
}
fn validate_cose(cose: &serde_cbor_2::Value) -> Result<(), Error> {
    use serde_cbor_2::Value::{Integer, Map};
    let Map(map) = cose else {
        return Err(Error::Invalid);
    };
    let alg = map
        .get(&Integer(3))
        .and_then(|v| if let Integer(n) = v { Some(*n) } else { None })
        .ok_or(Error::Invalid)?;
    let supported = match alg {
        -7 => {
            map.get(&Integer(1)) == Some(&Integer(2)) && map.get(&Integer(-1)) == Some(&Integer(1))
        }
        -8 => {
            map.get(&Integer(1)) == Some(&Integer(1)) && map.get(&Integer(-1)) == Some(&Integer(6))
        }
        _ => false,
    };
    if !supported {
        return Err(Error::UnsupportedPasskey(alg));
    }
    Ok(())
}
pub fn validate_home(home: &std::path::Path) -> Result<(), Error> {
    let raw = match std::fs::read(home.join("credentials.json")) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let data: crate::Credentials = serde_json::from_slice(&raw)?;
    for key in read::<Passkey>(&data, "passkeys")? {
        import(&key.credential)?;
    }
    Ok(())
}
impl Authority {
    pub fn passkeys(&self, rp: &str) -> Result<Vec<Passkey>, Error> {
        Ok(read::<Passkey>(&self.load()?, "passkeys")?
            .into_iter()
            .filter(|k| k.rp_id == rp)
            .collect())
    }
    pub fn save_passkey(
        &self,
        key: Passkey,
        enroll: &str,
        principal: Option<&Principal>,
    ) -> Result<(), Error> {
        import(&key.credential)?;
        self.update(|d| {
            if let Some(p) = principal {
                if !d
                    .principals
                    .iter()
                    .any(|v| v.id == p.id && v.kind == "device" && v.active())
                {
                    return Err(Error::Unauthorized);
                }
                if !read::<crate::devices::Session>(d, "devices")?
                    .iter()
                    .any(|s| s.device_id == p.id && crate::devices::valid(s))
                {
                    return Err(Error::Unauthorized);
                }
            } else {
                let tokens = d
                    .extra
                    .get_mut("web_enroll")
                    .and_then(Value::as_object_mut)
                    .ok_or(Error::Unauthorized)?;
                let hash = fingerprint(enroll.as_bytes());
                if !tokens.get(&hash).and_then(Value::as_str).is_some_and(live) {
                    return Err(Error::Unauthorized);
                }
                tokens.remove(&hash);
            }
            let mut keys: Vec<Passkey> = read(d, "passkeys")?;
            if keys.iter().any(|v| v.id == key.id && v.rp_id == key.rp_id) {
                return Err(Error::Invalid);
            }
            keys.push(key);
            write(d, "passkeys", &keys)
        })
    }
    pub fn finish_passkey_login(
        &self,
        rp: &str,
        origin: &str,
        state: Value,
        response: Value,
    ) -> Result<(), Error> {
        let response: PublicKeyCredential = serde_json::from_value(response)?;
        if response
            .response
            .user_handle
            .as_ref()
            .is_none_or(|h| h.as_slice() != b"apple-bridge-owner")
        {
            return Err(Error::Unauthorized);
        }
        let result = core(rp, origin)?
            .authenticate_credential(&response, &serde_json::from_value(state)?)
            .map_err(invalid)?;
        self.update(|d| {
            let mut keys: Vec<Passkey> = read(d, "passkeys")?;
            let key = keys
                .iter_mut()
                .find(|k| k.rp_id == rp && k.id == URL_SAFE_NO_PAD.encode(result.cred_id()))
                .ok_or(Error::Unauthorized)?;
            let previous = import(&key.credential)?;
            if (result.counter() != 0 || previous.counter != 0)
                && result.counter() <= previous.counter
            {
                return Err(Error::Unauthorized);
            }
            key.credential["authenticator"]["signCount"] = result.counter().into();
            key.credential["flags"]["backupState"] = result.backup_state().into();
            write(d, "passkeys", &keys)
        })
    }
}
