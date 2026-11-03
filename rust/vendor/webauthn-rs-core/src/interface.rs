 
 

use crate::attestation::verify_attestation_ca_chain;
use crate::error::*;
use crypto_glue::{
    ecdsa_p256::{EcdsaP256FieldBytes, EcdsaP256PublicKey, EcdsaP256PublicSec1Point},
    traits::{DecodeDer, EncodeDer, FromSec1Point},
    x509,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_with::{
    base64::{Base64, UrlSafe},
    formats::Unpadded,
    serde_as, IfIsHumanReadable, PickFirst,
};
use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;
use uuid::Uuid;
use webauthn_rs_proto::cose::*;
use webauthn_rs_proto::extensions::*;
use webauthn_rs_proto::options::*;

pub use crate::internals::{AttestationObject, Challenge};
pub use webauthn_attestation_ca::*;

 
 
pub type Aaguid = [u8; 16];

 
pub type Counter = u32;

 
 
 
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationState {
    pub(crate) policy: UserVerificationPolicy,
    #[serde_as(as = "IfIsHumanReadable<Vec<Base64<UrlSafe, Unpadded>>>")]
    pub(crate) exclude_credentials: Vec<CredentialID>,
    #[serde_as(as = "IfIsHumanReadable<Base64<UrlSafe, Unpadded>>")]
    pub(crate) challenge: Challenge,
    pub(crate) credential_algorithms: Vec<COSEAlgorithm>,
    pub(crate) require_resident_key: bool,
    pub(crate) authenticator_attachment: Option<AuthenticatorAttachment>,
    pub(crate) extensions: RequestRegistrationExtensions,
    pub(crate) allow_synchronised_authenticators: bool,
}

 
 
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationState {
    pub(crate) credentials: Vec<Credential>,
    pub(crate) policy: UserVerificationPolicy,
    #[serde_as(as = "IfIsHumanReadable<Base64<UrlSafe, Unpadded>>")]
    pub(crate) challenge: Challenge,
    pub(crate) appid: Option<String>,
    pub(crate) allow_backup_eligible_upgrade: bool,
}

impl AuthenticationState {
     
     
     
     
    pub fn set_allowed_credentials(&mut self, credentials: Vec<Credential>) {
        self.credentials = credentials;
    }
}

 
 
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EDDSACurve {
     
     
     
     
     
     
     
     
     
     
     
     
     
    ED25519 = 6,
     
    ED448 = 7,
}

impl EDDSACurve {
     
    pub(crate) fn coordinate_size(&self) -> usize {
        match self {
            Self::ED25519 => 32,
            Self::ED448 => 57,
        }
    }
}

 
 
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ECDSACurve {
     
     
     
     
     
     
     
     
    SECP256R1 = 1,
     
    SECP384R1 = 2,
     
    SECP521R1 = 3,
}

impl ECDSACurve {
     
    pub(crate) fn coordinate_size(&self) -> usize {
        match self {
            Self::SECP256R1 => 32,
            Self::SECP384R1 => 48,
            Self::SECP521R1 => 66,
        }
    }
}

 
 
 
 
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct COSEEC2Key {
     
    pub curve: ECDSACurve,
     
    #[serde_as(as = "PickFirst<(IfIsHumanReadable<Base64<UrlSafe, Unpadded>>, _)>")]
    pub x: Vec<u8>,
     
    #[serde_as(as = "PickFirst<(IfIsHumanReadable<Base64<UrlSafe, Unpadded>>, _)>")]
    pub y: Vec<u8>,
}

impl TryFrom<&EcdsaP256PublicKey> for COSEEC2Key {
    type Error = WebauthnError;

    fn try_from(k: &EcdsaP256PublicKey) -> Result<Self, Self::Error> {
        let encoded_point = EcdsaP256PublicSec1Point::from(k);

        let public_key_x = encoded_point
            .x()
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();

        let public_key_y = encoded_point
            .y()
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();

        Ok(COSEEC2Key {
            curve: ECDSACurve::SECP256R1,
            x: public_key_x,
            y: public_key_y,
        })
    }
}

impl TryFrom<&COSEEC2Key> for EcdsaP256PublicKey {
    type Error = WebauthnError;

    fn try_from(k: &COSEEC2Key) -> Result<Self, Self::Error> {
        let mut field_x = EcdsaP256FieldBytes::default();
        if k.x.len() != field_x.len() {
            return Err(WebauthnError::COSEKeyECDSAXYInvalid);
        }

        let mut field_y = EcdsaP256FieldBytes::default();
        if k.y.len() != field_y.len() {
            return Err(WebauthnError::COSEKeyECDSAXYInvalid);
        }

        field_x.copy_from_slice(&k.x);
        field_y.copy_from_slice(&k.y);

        let ep = EcdsaP256PublicSec1Point::from_affine_coordinates(&field_x, &field_y, false);

        EcdsaP256PublicKey::from_sec1_point(&ep)
            .into_option()
            .ok_or(WebauthnError::COSEKeyECDSAXYInvalid)
    }
}

 
 
 
 
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct COSEOKPKey {
     
    pub curve: EDDSACurve,
     
    #[serde_as(as = "PickFirst<(IfIsHumanReadable<Base64<UrlSafe, Unpadded>>, _)>")]
    pub x: Vec<u8>,
}

 
 
 
 
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct COSERSAKey {
     
    #[serde_as(as = "PickFirst<(IfIsHumanReadable<Base64<UrlSafe, Unpadded>>, _)>")]
    pub n: Vec<u8>,
     
    pub e: [u8; 3],
}

 
 
#[allow(non_camel_case_types)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum COSEKeyType {
     
     
     
     
     
     
     
     
     
     
    EC_OKP(COSEOKPKey),
     
    EC_EC2(COSEEC2Key),
     
     
     
    RSA(COSERSAKey),
}

 
#[allow(non_camel_case_types)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i64)]
pub enum COSEKeyTypeId {
     
    EC_Reserved = 0,
     
    EC_OKP = 1,
     
    EC_EC2 = 2,
     
    EC_RSA = 3,
     
    EC_Symmetric = 4,
}

 
 
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct COSEKey {
     
    pub type_: COSEAlgorithm,
     
    pub key: COSEKeyType,
}

impl TryFrom<&EcdsaP256PublicKey> for COSEKey {
    type Error = WebauthnError;

    fn try_from(k: &EcdsaP256PublicKey) -> Result<Self, Self::Error> {
        let ec = COSEEC2Key::try_from(k)?;

        Ok(COSEKey {
            type_: COSEAlgorithm::ES256,
            key: COSEKeyType::EC_EC2(ec),
        })
    }
}

 
pub type CredentialID = Vec<u8>;

 
pub type Credential = CredentialV5;

 
 
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialV5 {
     
    #[serde_as(as = "IfIsHumanReadable<Base64<UrlSafe, Unpadded>>")]
    pub cred_id: CredentialID,
     
    pub cred: COSEKey,
     
    pub counter: Counter,
     
     
     
    pub transports: Option<Vec<AuthenticatorTransport>>,
     
     
     
     
     
     
     
     
     
     
    pub user_verified: bool,
     
     
     
     
     
    pub backup_eligible: bool,
     
     
    pub backup_state: bool,
     
     
     
     
    pub registration_policy: UserVerificationPolicy,
     
     
    pub extensions: RegisteredExtensions,
     
     
    pub attestation: ParsedAttestation,
     
    pub attestation_format: AttestationFormat,
}

impl Credential {
     
     
     
     
     
    pub fn verify_attestation<'a>(
        &'_ self,
        ca_list: &'a AttestationCaList,
    ) -> Result<Option<&'a AttestationCa>, WebauthnError> {
        let current_time = SystemTime::now();
        verify_attestation_ca_chain(&self.attestation.data, ca_list, current_time)
    }
}

impl From<CredentialV3> for Credential {
    fn from(other: CredentialV3) -> Credential {
        let CredentialV3 {
            cred_id,
            cred,
            counter,
            verified,
            registration_policy,
        } = other;

         
        Credential {
            cred_id,
            cred,
            counter,
            transports: None,
            user_verified: verified,
            backup_eligible: false,
            backup_state: false,
            registration_policy,
            extensions: RegisteredExtensions::none(),
            attestation: ParsedAttestation {
                data: ParsedAttestationData::None,
                metadata: AttestationMetadata::None,
            },
            attestation_format: AttestationFormat::None,
        }
    }
}

 
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialV3 {
     
    #[serde_as(as = "PickFirst<(IfIsHumanReadable<Base64<UrlSafe, Unpadded>>, _)>")]
    pub cred_id: Vec<u8>,
     
    pub cred: COSEKey,
     
    pub counter: u32,
     
     
     
     
     
     
     
     
     
     
    pub verified: bool,
     
     
     
     
    pub registration_policy: UserVerificationPolicy,
}

 
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub enum SerialisableAttestationData {
     
    Basic(#[serde_as(as = "IfIsHumanReadable<Vec<Base64<UrlSafe, Unpadded>>>")] Vec<Vec<u8>>),
     
    Self_,
     
    AttCa(#[serde_as(as = "IfIsHumanReadable<Vec<Base64<UrlSafe, Unpadded>>>")] Vec<Vec<u8>>),
     
    AnonCa(#[serde_as(as = "IfIsHumanReadable<Vec<Base64<UrlSafe, Unpadded>>>")] Vec<Vec<u8>>),
     
    ECDAA,
     
    None,
     
    Uncertain,
}

impl fmt::Debug for SerialisableAttestationData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerialisableAttestationData::Basic(_) => {
                write!(f, "SerialisableAttestationData::Basic")
            }
            SerialisableAttestationData::Self_ => write!(f, "SerialisableAttestationData::Self_"),
            SerialisableAttestationData::AttCa(_) => {
                write!(f, "SerialisableAttestationData::AttCa")
            }
            SerialisableAttestationData::AnonCa(_) => {
                write!(f, "SerialisableAttestationData::AnonCa")
            }
            SerialisableAttestationData::ECDAA => write!(f, "SerialisableAttestationData::ECDAA"),
            SerialisableAttestationData::None => write!(f, "SerialisableAttestationData::None"),
            SerialisableAttestationData::Uncertain => {
                write!(f, "SerialisableAttestationData::Uncertain")
            }
        }
    }
}

 
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedAttestation {
     
    pub data: ParsedAttestationData,
     
    pub metadata: AttestationMetadata,
}

impl Default for ParsedAttestation {
    fn default() -> Self {
        ParsedAttestation {
            data: ParsedAttestationData::None,
            metadata: AttestationMetadata::None,
        }
    }
}

 
 
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AttestationMetadata {
     
    None,
     
    Packed {
         
         
        aaguid: Uuid,
    },
     
    Tpm {
         
         
        aaguid: Uuid,
         
         
         
        firmware_version: u64,
    },
     
    AndroidKey {
         
        is_km_tee: bool,
         
        is_attest_tee: bool,
    },
     
     
    AndroidSafetyNet {
         
        apk_package_name: String,
         
        #[serde_as(as = "IfIsHumanReadable<Vec<Base64<UrlSafe, Unpadded>>>")]
        apk_certificate_digest_sha256: Vec<Vec<u8>>,
         
        cts_profile_match: bool,
         
        basic_integrity: bool,
         
        evaluation_type: Option<String>,
    },
}

 
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    try_from = "SerialisableAttestationData",
    into = "SerialisableAttestationData"
)]
pub enum ParsedAttestationData {
     
     
    Basic(Vec<x509::Certificate>),
     
     
    Self_,
     
     
    AttCa(Vec<x509::Certificate>),
     
     
    AnonCa(Vec<x509::Certificate>),
     
    ECDAA,
     
     
    None,
     
     
    Uncertain,
}

#[allow(clippy::from_over_into)]
impl Into<SerialisableAttestationData> for ParsedAttestationData {
    fn into(self) -> SerialisableAttestationData {
        match self {
            ParsedAttestationData::Basic(chain) => SerialisableAttestationData::Basic(
                chain
                    .into_iter()
                    .map(|c| c.to_der().expect("Invalid DER"))
                    .collect(),
            ),
            ParsedAttestationData::Self_ => SerialisableAttestationData::Self_,
            ParsedAttestationData::AttCa(chain) => SerialisableAttestationData::AttCa(
                 
                chain
                    .into_iter()
                    .map(|c| c.to_der().expect("Invalid DER"))
                    .collect(),
            ),
            ParsedAttestationData::AnonCa(chain) => SerialisableAttestationData::AnonCa(
                 
                chain
                    .into_iter()
                    .map(|c| c.to_der().expect("Invalid DER"))
                    .collect(),
            ),
            ParsedAttestationData::ECDAA => SerialisableAttestationData::ECDAA,
            ParsedAttestationData::None => SerialisableAttestationData::None,
            ParsedAttestationData::Uncertain => SerialisableAttestationData::Uncertain,
        }
    }
}

impl TryFrom<SerialisableAttestationData> for ParsedAttestationData {
    type Error = WebauthnError;

    fn try_from(data: SerialisableAttestationData) -> Result<Self, Self::Error> {
        Ok(match data {
            SerialisableAttestationData::Basic(chain) => ParsedAttestationData::Basic(
                chain
                    .into_iter()
                    .map(|c| {
                        x509::Certificate::from_der(c.as_slice())
                            .map_err(|_| WebauthnError::X509DerInvalid)
                    })
                    .collect::<WebauthnResult<_>>()?,
            ),
            SerialisableAttestationData::Self_ => ParsedAttestationData::Self_,
            SerialisableAttestationData::AttCa(chain) => ParsedAttestationData::AttCa(
                 
                chain
                    .into_iter()
                    .map(|c| {
                        x509::Certificate::from_der(c.as_slice())
                            .map_err(|_| WebauthnError::X509DerInvalid)
                    })
                    .collect::<WebauthnResult<_>>()?,
            ),
            SerialisableAttestationData::AnonCa(chain) => ParsedAttestationData::AnonCa(
                 
                chain
                    .into_iter()
                    .map(|c| {
                        x509::Certificate::from_der(c.as_slice())
                            .map_err(|_| WebauthnError::X509DerInvalid)
                    })
                    .collect::<WebauthnResult<_>>()?,
            ),
            SerialisableAttestationData::ECDAA => ParsedAttestationData::ECDAA,
            SerialisableAttestationData::None => ParsedAttestationData::None,
            SerialisableAttestationData::Uncertain => ParsedAttestationData::Uncertain,
        })
    }
}

 
#[derive(Debug)]
pub struct Registration;

 
#[derive(Debug)]
pub struct Authentication;

 
pub trait Ceremony {
     
    type SignedExtensions: DeserializeOwned + std::fmt::Debug + std::default::Default;
}

impl Ceremony for Registration {
    type SignedExtensions = RegistrationSignedExtensions;
}

impl Ceremony for Authentication {
    type SignedExtensions = AuthenticationSignedExtensions;
}

 
 
 
 
#[derive(Debug, Serialize, Clone, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct CredProtectResponse(pub CredentialProtectionPolicy);

 
 
 
 
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistrationSignedExtensions {
     
    #[serde(rename = "credProtect")]
    pub cred_protect: Option<CredProtectResponse>,
     
    #[serde(rename = "hmac-secret")]
    pub hmac_secret: Option<bool>,
     
    #[serde(flatten)]
    pub unknown_keys: BTreeMap<String, serde_cbor_2::Value>,
}

 
 
 
 
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationSignedExtensions {
     
    #[serde(flatten)]
    pub unknown_keys: BTreeMap<String, serde_cbor_2::Value>,
}

 
#[derive(Debug, Clone)]
pub struct AttestedCredentialData {
     
    pub aaguid: Aaguid,
     
    pub credential_id: CredentialID,
     
    pub credential_pk: serde_cbor_2::Value,
}

 
#[serde_as]
#[derive(Debug, Serialize, Clone, Deserialize)]
pub struct AuthenticationResult {
     
    #[serde_as(as = "IfIsHumanReadable<Base64<UrlSafe, Unpadded>>")]
    pub(crate) cred_id: CredentialID,
     
    pub(crate) needs_update: bool,
     
    pub(crate) user_verified: bool,
     
     
    pub(crate) backup_state: bool,
     
     
     
     
    pub(crate) backup_eligible: bool,
     
    pub(crate) counter: Counter,
     
    pub(crate) extensions: AuthenticationExtensions,
}

impl AuthenticationResult {
     
    pub fn cred_id(&self) -> &CredentialID {
        &self.cred_id
    }

     
     
    pub fn needs_update(&self) -> bool {
        self.needs_update
    }

     
    pub fn user_verified(&self) -> bool {
        self.user_verified
    }

     
     
    pub fn backup_state(&self) -> bool {
        self.backup_state
    }

     
     
     
     
    pub fn backup_eligible(&self) -> bool {
        self.backup_eligible
    }

     
    pub fn counter(&self) -> Counter {
        self.counter
    }

     
    pub fn extensions(&self) -> &AuthenticationExtensions {
        &self.extensions
    }
}
