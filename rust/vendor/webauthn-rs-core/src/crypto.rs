 
 
 
 

#![allow(non_camel_case_types)]

use super::error::*;
use crate::proto::*;
use crypto_glue::{
    ecdsa_p256::{
        self, EcdsaP256PublicKey, EcdsaP256PublicSec1Point, EcdsaP256Signature,
        EcdsaP256VerifyingKey,
    },
    ecdsa_p384::{
        self, EcdsaP384PublicKey, EcdsaP384PublicSec1Point, EcdsaP384Signature,
        EcdsaP384VerifyingKey,
    },
    ecdsa_p521::{
        self,
        EcdsaP521PublicKey,
         
        EcdsaP521PublicSec1Point,
    },
    s256,
    traits::{Digest, OwnedToRef, Verifier},
    x509::{self, Certificate, GeneralName, ObjectIdentifier, OtherName, SubjectAltName},
};

 
pub fn verify_signature(
    certificate: &Certificate,
    signature: &[u8],
    verification_data: &[u8],
) -> Result<bool, WebauthnError> {
    let valid = x509::x509_verify_signature(verification_data, signature, certificate)
        .inspect_err(|err| {
            error!(?err, "x509 Verification Error");
        })
        .is_ok();

    Ok(valid)
}

pub(crate) struct TpmSanData<'a> {
    pub manufacturer: &'a str,
    pub _model: &'a str,
    pub _version: &'a str,
}

#[derive(Default)]
struct TpmSanDataBuilder<'a> {
    manufacturer: Option<&'a str>,
    model: Option<&'a str>,
    version: Option<&'a str>,
}

impl<'a> TpmSanDataBuilder<'a> {
    pub(crate) fn new() -> Self {
        Default::default()
    }

    pub(crate) fn manufacturer(mut self, value: &'a str) -> Self {
        self.manufacturer = Some(value);
        self
    }

    pub(crate) fn model(mut self, value: &'a str) -> Self {
        self.model = Some(value);
        self
    }

    pub(crate) fn version(mut self, value: &'a str) -> Self {
        self.version = Some(value);
        self
    }

    pub(crate) fn build(self) -> WebauthnResult<TpmSanData<'a>> {
        self.manufacturer
            .zip(self.model)
            .zip(self.version)
            .map(|((manufacturer, model), version)| TpmSanData {
                manufacturer,
                _model: model,
                _version: version,
            })
            .ok_or(WebauthnError::AttestationCertificateRequirementsNotMet)
    }
}

 
 
 

pub(crate) const TCG_AT_TPM_MANUFACTURER_RAW: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.23.133.2.1");
pub(crate) const TCG_AT_TPM_MODEL_RAW: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.23.133.2.2");
pub(crate) const TCG_AT_TPM_VERSION_RAW: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.23.133.2.3");

impl<'a> TryFrom<&'a SubjectAltName> for TpmSanData<'a> {
    type Error = WebauthnError;

    fn try_from(x509_name: &'a SubjectAltName) -> Result<Self, Self::Error> {
        x509_name
            .0
            .iter()
            .try_fold(TpmSanDataBuilder::new(), |builder, general_name| {
                let next = match general_name {
                    GeneralName::OtherName(OtherName { type_id, value }) => {
                        if *type_id == TCG_AT_TPM_MANUFACTURER_RAW {
                            let attr_value = str::from_utf8(value.value())?;
                            builder.manufacturer(attr_value)
                        } else if *type_id == TCG_AT_TPM_MODEL_RAW {
                            let attr_value = str::from_utf8(value.value())?;
                            builder.model(attr_value)
                        } else if *type_id == TCG_AT_TPM_VERSION_RAW {
                            let attr_value = str::from_utf8(value.value())?;
                            builder.version(attr_value)
                        } else {
                            builder
                        }
                    }
                    _ => builder,
                };
                Ok(next)
            })
            .map_err(|_: std::str::Utf8Error| WebauthnError::ParseNOMFailure)
            .and_then(TpmSanDataBuilder::build)
    }
}

pub(crate) fn only_hash_from_type(
    alg: COSEAlgorithm,
    _input: &[u8],
) -> Result<Vec<u8>, WebauthnError> {
    match alg {
        COSEAlgorithm::INSECURE_RS1 => {
             
            warn!("INSECURE SHA1 USAGE DETECTED");
            Err(WebauthnError::CredentialInsecureCryptography)
        }
        c_alg => {
            debug!(?c_alg, "WebauthnError::COSEKeyInvalidType");
            Err(WebauthnError::COSEKeyInvalidType)
        }
    }
}

impl TryFrom<&serde_cbor_2::Value> for COSEKey {
    type Error = WebauthnError;
    fn try_from(d: &serde_cbor_2::Value) -> Result<COSEKey, Self::Error> {
        let m = cbor_try_map!(d)?;

         
         
         
         
         
         
         
         
         
         
         
         

        let key_type_value = m
            .get(&serde_cbor_2::Value::Integer(1))
            .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
        let key_type = cbor_try_i128!(key_type_value)?;
        

















        let content_type_value = m
            .get(&serde_cbor_2::Value::Integer(3))
            .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
        let content_type = cbor_try_i128!(content_type_value)?;

        let type_ = COSEAlgorithm::try_from(content_type)
            .map_err(|_| WebauthnError::COSEKeyInvalidAlgorithm)?;

         
         
         
         
        if key_type == (COSEKeyTypeId::EC_EC2 as i128)
            && (type_ == COSEAlgorithm::ES256
                || type_ == COSEAlgorithm::ES384
                || type_ == COSEAlgorithm::ES521)
        {
             
             
             

            let curve_type_value = m
                .get(&serde_cbor_2::Value::Integer(-1))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let curve_type = cbor_try_i128!(curve_type_value)?;

            let curve = ECDSACurve::try_from(curve_type)?;

            let x_value = m
                .get(&serde_cbor_2::Value::Integer(-2))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let x = cbor_try_bytes!(x_value)?;

            let y_value = m
                .get(&serde_cbor_2::Value::Integer(-3))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let y = cbor_try_bytes!(y_value)?;

            let coord_len = curve.coordinate_size();
            if x.len() != coord_len || y.len() != coord_len {
                return Err(WebauthnError::COSEKeyECDSAXYInvalid);
            }

             
            let cose_key = COSEKey {
                type_,
                key: COSEKeyType::EC_EC2(COSEEC2Key {
                    curve,
                    x: x.to_vec(),
                    y: y.to_vec(),
                }),
            };

             
             
             
             

            cose_key.validate()?;
             
            Ok(cose_key)
        } else if key_type == (COSEKeyTypeId::EC_RSA as i128) && (type_ == COSEAlgorithm::RS256) {
             

             
             

             
             

            let n_value = m
                .get(&serde_cbor_2::Value::Integer(-1))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let n = cbor_try_bytes!(n_value)?;

            let e_value = m
                .get(&serde_cbor_2::Value::Integer(-2))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let e = cbor_try_bytes!(e_value)?;

            if n.len() != 256 || e.len() != 3 {
                return Err(WebauthnError::COSEKeyRSANEInvalid);
            }

             
            let mut e_temp = [0; 3];
            e_temp.copy_from_slice(e.as_slice());

             
            let cose_key = COSEKey {
                type_,
                key: COSEKeyType::RSA(COSERSAKey {
                    n: n.to_vec(),
                    e: e_temp,
                }),
            };

            cose_key.validate()?;
             
            Ok(cose_key)
        } else if key_type == (COSEKeyTypeId::EC_OKP as i128) && (type_ == COSEAlgorithm::EDDSA) {
             

            let curve_type_value = m
                .get(&serde_cbor_2::Value::Integer(-1))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let curve = cbor_try_i128!(curve_type_value).and_then(EDDSACurve::try_from)?;

            
















            let x_value = m
                .get(&serde_cbor_2::Value::Integer(-2))
                .ok_or(WebauthnError::COSEKeyInvalidCBORValue)?;
            let x = cbor_try_bytes!(x_value)?;

            if x.len() != curve.coordinate_size() {
                return Err(WebauthnError::COSEKeyEDDSAXInvalid);
            }

            let cose_key = COSEKey {
                type_,
                key: COSEKeyType::EC_OKP(COSEOKPKey {
                    curve,
                    x: x.to_vec(),
                }),
            };

             
             
             
             
            cose_key.validate()?;
             
            Ok(cose_key)
        } else {
            debug!(?key_type, ?type_, "WebauthnError::COSEKeyInvalidType");
            Err(WebauthnError::COSEKeyInvalidType)
        }
    }
}

impl TryFrom<(COSEAlgorithm, &Certificate)> for COSEKey {
    type Error = WebauthnError;

    fn try_from((alg, certificate): (COSEAlgorithm, &Certificate)) -> Result<COSEKey, Self::Error> {
        let subject_public_key_info = certificate
            .tbs_certificate()
            .subject_public_key_info()
            .owned_to_ref();

        let key = match alg {
            COSEAlgorithm::ES256 => {
                let pub_key = EcdsaP256PublicKey::try_from(subject_public_key_info)
                    .map_err(|_err| WebauthnError::CertificatePublicKeyAlgorthimMismatch)?;

                let point = EcdsaP256PublicSec1Point::from(pub_key);

                let Some(xbn) = point.x().map(|x| x.to_vec()) else {
                    return Err(WebauthnError::EcdsaPointInvalid);
                };

                let Some(ybn) = point.y().map(|y| y.to_vec()) else {
                    return Err(WebauthnError::EcdsaPointInvalid);
                };

                Ok(COSEKeyType::EC_EC2(COSEEC2Key {
                    curve: ECDSACurve::SECP256R1,
                    x: xbn,
                    y: ybn,
                }))
            }

            COSEAlgorithm::ES384 => {
                let pub_key = EcdsaP384PublicKey::try_from(subject_public_key_info)
                    .map_err(|_err| WebauthnError::CertificatePublicKeyAlgorthimMismatch)?;

                let point = EcdsaP384PublicSec1Point::from(pub_key);

                let Some(xbn) = point.x().map(|x| x.to_vec()) else {
                    return Err(WebauthnError::EcdsaPointInvalid);
                };

                let Some(ybn) = point.y().map(|y| y.to_vec()) else {
                    return Err(WebauthnError::EcdsaPointInvalid);
                };

                Ok(COSEKeyType::EC_EC2(COSEEC2Key {
                    curve: ECDSACurve::SECP384R1,
                    x: xbn,
                    y: ybn,
                }))
            }
            COSEAlgorithm::ES521 => {
                let pub_key = EcdsaP521PublicKey::try_from(subject_public_key_info)
                    .map_err(|_err| WebauthnError::CertificatePublicKeyAlgorthimMismatch)?;

                let point = EcdsaP521PublicSec1Point::from(pub_key);

                let Some(xbn) = point.x().map(|x| x.to_vec()) else {
                    return Err(WebauthnError::EcdsaPointInvalid);
                };

                let Some(ybn) = point.y().map(|y| y.to_vec()) else {
                    return Err(WebauthnError::EcdsaPointInvalid);
                };

                Ok(COSEKeyType::EC_EC2(COSEEC2Key {
                    curve: ECDSACurve::SECP521R1,
                    x: xbn,
                    y: ybn,
                }))
            }

            COSEAlgorithm::RS256
            | COSEAlgorithm::RS384
            | COSEAlgorithm::RS512
            | COSEAlgorithm::PS256
            | COSEAlgorithm::PS384
            | COSEAlgorithm::PS512
            | COSEAlgorithm::EDDSA
            | COSEAlgorithm::PinUvProtocol
            | COSEAlgorithm::INSECURE_RS1 => {
                error!(
                    "unsupported X509 to COSE conversion for COSE algorithm type {:?}",
                    alg
                );
                Err(WebauthnError::COSEKeyInvalidType)
            }
        }?;

        Ok(COSEKey { type_: alg, key })
    }
}

enum COSEKeyPublic {
    EcdsaP256(EcdsaP256PublicKey),
    EcdsaP384(EcdsaP384PublicKey),
    EcdsaP521(EcdsaP521PublicKey),
    Ed25519(ed25519_dalek::VerifyingKey),
     
     
}

impl COSEKey {
    pub(crate) fn get_alg_key_ecc_x962_raw(&self) -> Result<Vec<u8>, WebauthnError> {
         
         
        match &self.key {
            COSEKeyType::EC_EC2(ecpk) => {
                let r: [u8; 1] = [0x04];
                Ok(r.iter()
                    .chain(ecpk.x.iter())
                    .chain(ecpk.y.iter())
                    .copied()
                    .collect())
            }
            _ => {
                debug!("get_alg_key_ecc_x962_raw");
                Err(WebauthnError::COSEKeyInvalidType)
            }
        }
    }

    pub(crate) fn validate(&self) -> Result<(), WebauthnError> {
        self.get_public_key().map(|_| ())
    }

     
    fn get_public_key(&self) -> Result<COSEKeyPublic, WebauthnError> {
        match &self.key {
            COSEKeyType::EC_EC2(ec2k) => match ec2k.curve {
                ECDSACurve::SECP256R1 => {
                    ecdsa_p256::from_coords_raw(ec2k.x.as_ref(), ec2k.y.as_ref())
                        .map(COSEKeyPublic::EcdsaP256)
                        .ok_or(WebauthnError::EcdsaPointInvalid)
                }
                ECDSACurve::SECP384R1 => {
                    ecdsa_p384::from_coords_raw(ec2k.x.as_ref(), ec2k.y.as_ref())
                        .map(COSEKeyPublic::EcdsaP384)
                        .ok_or(WebauthnError::EcdsaPointInvalid)
                }
                ECDSACurve::SECP521R1 => {
                    ecdsa_p521::from_coords_raw(ec2k.x.as_ref(), ec2k.y.as_ref())
                        .map(COSEKeyPublic::EcdsaP521)
                        .ok_or(WebauthnError::EcdsaPointInvalid)
                }
            },
            COSEKeyType::RSA(_) => Err(WebauthnError::COSEKeyInvalidType),
            COSEKeyType::EC_OKP(edk) => {
                if edk.curve != EDDSACurve::ED25519 || self.type_ != COSEAlgorithm::EDDSA {
                    return Err(WebauthnError::COSEKeyInvalidType);
                }
                let x: &[u8; 32] = edk.x.as_slice().try_into().map_err(|_| WebauthnError::COSEKeyEDDSAXInvalid)?;
                ed25519_dalek::VerifyingKey::from_bytes(x)
                    .map(COSEKeyPublic::Ed25519)
                    .map_err(|_| WebauthnError::COSEKeyEDDSAXInvalid)
            }
        }
    }

     
    pub fn verify_signature(
        &self,
        signature: &[u8],
        verification_data: &[u8],
    ) -> Result<bool, WebauthnError> {
        let public_key = self.get_public_key()?;

        match public_key {
            COSEKeyPublic::EcdsaP256(pub_key) => {
                let signature = EcdsaP256Signature::from_der(signature)
                    .map_err(|_err| WebauthnError::SignatureInvalid)?;
                let verifier = EcdsaP256VerifyingKey::from(&pub_key);
                Ok(verifier.verify(verification_data, &signature).is_ok())
            }
            COSEKeyPublic::EcdsaP384(pub_key) => {
                let signature = EcdsaP384Signature::from_der(signature)
                    .map_err(|_err| WebauthnError::SignatureInvalid)?;
                let verifier = EcdsaP384VerifyingKey::from(&pub_key);
                Ok(verifier.verify(verification_data, &signature).is_ok())
            }
            COSEKeyPublic::EcdsaP521(_pub_key) => {
                 
                 
                 
                





                Ok(false)
            }
            COSEKeyPublic::Ed25519(key) => {
                let sig = ed25519_dalek::Signature::from_slice(signature).map_err(|_| WebauthnError::SignatureInvalid)?;
                Ok(key.verify_strict(verification_data, &sig).is_ok())
            }
        }
    }
}

 
pub fn compute_sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = s256::Sha256::new();
    hasher.update(data);
    *hasher.finalize().as_ref()
}

