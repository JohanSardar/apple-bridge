 
 
 

use crate::crypto::{compute_sha256, only_hash_from_type, verify_signature, TpmSanData};
use crate::error::WebauthnError;
use crate::internals::*;
use crate::proto::*;
use crypto_glue::{
    traits::DecodeDer,
    x509::{
        self, oiddb::rfc4519, BasicConstraints, ExtendedKeyUsage, ObjectIdentifier, SubjectAltName,
        X509Display,
    },
};
use std::time::SystemTime;
use uuid::Uuid;

static OID_JOINT_ISO_ITU_T: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.23.133.8.3");

 
 
pub trait AttestationX509Extension {
     
    type Output: Eq;

     
    const OID: ObjectIdentifier;

     
    fn parse(i: &[u8]) -> der_parser::error::BerResult<'_, (Self::Output, AttestationMetadata)>;

     
     
    const IS_REQUIRED: bool;

     
     
    const VALIDATION_ERROR: WebauthnError;
}

 
pub struct FidoGenCeAaguid;
pub(crate) struct AppleAnonymousNonce;

pub(crate) struct AndroidKeyAttestationExtensionData;

impl AttestationX509Extension for FidoGenCeAaguid {
     
    const OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.4.1.45724.1.1.4");

     
    type Output = Aaguid;

    fn parse(i: &[u8]) -> der_parser::error::BerResult<'_, (Self::Output, AttestationMetadata)> {
        let (rem, aaguid) = der_parser::der::parse_der_octetstring(i)?;
        let aaguid: Aaguid = aaguid
            .as_slice()
            .expect("octet string can be used as a slice")
            .try_into()
            .map_err(|_| der_parser::error::BerError::InvalidLength)?;

        Ok((rem, (aaguid, AttestationMetadata::None)))
    }

    const IS_REQUIRED: bool = false;

    const VALIDATION_ERROR: WebauthnError = WebauthnError::AttestationCertificateAAGUIDMismatch;
}

pub(crate) mod android_key_attestation {
    use der_parser::ber::BerObjectContent;

    use crate::proto::AttestationMetadata;

    #[derive(Clone, PartialEq, Eq)]
    pub struct Data {
        pub attestation_challenge: Vec<u8>,
        pub attest_enforcement: EnforcementType,
        pub km_enforcement: EnforcementType,
        pub software_enforced: AuthorizationList,
        pub tee_enforced: AuthorizationList,
    }

    #[derive(Clone, PartialEq, Eq, Copy)]
    pub struct AuthorizationList {
        pub all_applications: bool,
        pub origin: Option<u32>,
        pub purpose: Option<u32>,
    }

    pub const KM_ORIGIN_GENERATED: u32 = 0;
    pub const KM_PURPOSE_SIGN: u32 = 2;

    #[derive(Clone, Eq)]
    pub enum EnforcementType {
        Software,
        Tee,
        #[allow(unused)]
        Either,
    }

    impl PartialEq for EnforcementType {
        fn eq(&self, other: &Self) -> bool {
            matches!(
                (self, other),
                (Self::Either, _)
                    | (_, Self::Either)
                    | (Self::Software, Self::Software)
                    | (Self::Tee, Self::Tee)
            )
        }
    }

    impl AuthorizationList {
        pub fn parse(i: &[u8]) -> der_parser::error::BerResult<'_, Self> {
            use der_parser::{der::*, error::BerError};
            parse_der_container(|i: &[u8], hdr: Header| {
                if hdr.tag() != Tag::Sequence {
                    return Err(nom::Err::Error(BerError::BerTypeError));
                }

                let mut all_applications = false;
                let mut origin = None;
                let mut purpose = None;

                let mut i = i;
                while let Ok((k, obj)) = parse_der(i) {
                    i = k;
                     
                    if obj.content == BerObjectContent::Optional(None) {
                        continue;
                    }

                    match obj.tag() {
                        Tag(600) => {
                            all_applications = true;
                        }
                        Tag(702) => {
                            if let BerObjectContent::Unknown(o) = obj.content {
                                let (_, val) = parse_der_integer(o.data)?;
                                origin = Some(val.as_u32()?);
                            }
                        }
                        Tag(1) => {
                            if let BerObjectContent::Unknown(o) = obj.content {
                                let (_, val) =
                                    parse_der_container(|i, _| parse_der_integer(i))(o.data)?;
                                purpose = Some(val.as_u32()?);
                            }
                        }
                        _ => continue,
                    };
                }

                let al = AuthorizationList {
                    all_applications,
                    origin,
                    purpose,
                };

                Ok((i, al))
            })(i)
        }
    }

    impl Data {
        pub fn parse(i: &[u8]) -> der_parser::error::BerResult<'_, (Vec<u8>, AttestationMetadata)> {
            use der_parser::{der::*, error::BerError};
            parse_der_container(|i: &[u8], hdr: Header| {
                if hdr.tag() != Tag::Sequence {
                    return Err(nom::Err::Error(BerError::BerTypeError));
                }
                let (i, attestation_version) = parse_der_integer(i)?;
                let _attestation_version = attestation_version.as_i64()?;

                let (i, attest_sec_level) = parse_der_enum(i)?;  
                let attest_sec_level = attest_sec_level.as_u32()?;
                let (i, _) = parse_der_integer(i)?;  
                let (i, km_sec_level) = parse_der_enum(i)?;  
                let km_sec_level = km_sec_level.as_u32()?;

                let (i, attestation_challenge) = parse_der_octetstring(i)?;
                let attestation_challenge = attestation_challenge.as_slice()?.to_vec();

                let (i, _unique_id) = parse_der_octetstring(i)?;

                let (i, software_enforced) = AuthorizationList::parse(i)?;
                let (i, tee_enforced) = AuthorizationList::parse(i)?;

                let attest_enforcement = match attest_sec_level {
                    0 => EnforcementType::Software,
                    1 => EnforcementType::Tee,
                    _ => return Err(der_parser::error::BerError::InvalidTag)?,
                };

                let km_enforcement = match km_sec_level {
                    0 => EnforcementType::Software,
                    1 => EnforcementType::Tee,
                    _ => return Err(der_parser::error::BerError::InvalidTag)?,
                };

                 
                if software_enforced.all_applications || tee_enforced.all_applications {
                    Err(der_parser::error::BerError::InvalidValue {
                        tag: Tag(600),
                        msg: "all_applications must not be set".to_string(),
                    })?;
                }

                 
                let software_set = match (software_enforced.origin, software_enforced.purpose) {
                    (Some(origin), Some(purpose))
                        if origin == KM_ORIGIN_GENERATED && purpose == KM_PURPOSE_SIGN =>
                    {
                        true
                    }
                    (None, None) => false,
                    _ => {
                        return Err(der_parser::error::BerError::InvalidValue {
                            tag: Tag(701),
                            msg: "invalid key master values (software)".to_string(),
                        })?;
                    }
                };

                let tee_set = match (tee_enforced.origin, tee_enforced.purpose) {
                    (Some(origin), Some(purpose))
                        if origin == KM_ORIGIN_GENERATED && purpose == KM_PURPOSE_SIGN =>
                    {
                        true
                    }
                    (None, None) => false,
                    _ => {
                        return Err(der_parser::error::BerError::InvalidValue {
                            tag: Tag(701),
                            msg: "invalid key master values (tee)".to_string(),
                        })?;
                    }
                };

                if !tee_set && !software_set {
                    Err(der_parser::error::BerError::InvalidValue {
                        tag: Tag(701),
                        msg: "both software and tee not set (keymaster values)".to_string(),
                    })?;
                }

                let metadata = AttestationMetadata::AndroidKey {
                    is_km_tee: km_enforcement == EnforcementType::Tee,
                    is_attest_tee: attest_enforcement == EnforcementType::Tee,
                };

                Ok((i, (attestation_challenge, metadata)))
            })(i)
        }
    }
}

impl AttestationX509Extension for AndroidKeyAttestationExtensionData {
     
    const OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.4.1.11129.2.1.17");

     
    type Output = Vec<u8>;

    fn parse(i: &[u8]) -> der_parser::error::BerResult<'_, (Self::Output, AttestationMetadata)> {
        android_key_attestation::Data::parse(i)
    }

    const IS_REQUIRED: bool = true;

    const VALIDATION_ERROR: WebauthnError = WebauthnError::AttestationCertificateNonceMismatch;
}

impl AttestationX509Extension for AppleAnonymousNonce {
    type Output = [u8; 32];

     
    const OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113635.100.8.2");

    fn parse(i: &[u8]) -> der_parser::error::BerResult<'_, (Self::Output, AttestationMetadata)> {
        use der_parser::{der::*, error::BerError};
        parse_der_container(|i: &[u8], hdr: Header| {
            if hdr.tag() != Tag::Sequence {
                return Err(nom::Err::Error(BerError::BerTypeError));
            }
            let (i, tagged_nonce) = parse_der_tagged_explicit(1, parse_der_octetstring)(i)?;
            let (class, _tag, nonce) = tagged_nonce.as_tagged()?;
            if class != Class::ContextSpecific {
                return Err(nom::Err::Error(BerError::BerTypeError));
            }
            let nonce = nonce
                .as_slice()?
                .try_into()
                .map_err(|_| der_parser::error::BerError::InvalidLength)?;
            Ok((i, (nonce, AttestationMetadata::None)))
        })(i)
    }

    const IS_REQUIRED: bool = true;

    const VALIDATION_ERROR: WebauthnError = WebauthnError::AttestationCertificateNonceMismatch;
}

 
pub fn validate_extension<T>(
    x509: &x509::Certificate,
    data: &<T as AttestationX509Extension>::Output,
) -> Result<AttestationMetadata, WebauthnError>
where
    T: AttestationX509Extension,
{
    x509.tbs_certificate()
        .extensions()
        .as_ref()
        .and_then(|extensions| {
            extensions.iter().find_map(|extension| {
                (extension.extn_id == T::OID).then(|| {
                    T::parse(extension.extn_value.as_bytes())
                        .map_err(|_| WebauthnError::AttestationStatementX5CInvalid)
                        .and_then(|(_, (output, metadata))| {
                            if &output == data {
                                Ok(metadata)
                            } else {
                                Err(T::VALIDATION_ERROR)
                            }
                        })
                })
            })
        })
        .unwrap_or({
            if T::IS_REQUIRED {
                Err(WebauthnError::AttestationStatementMissingExtension)
            } else {
                Ok(AttestationMetadata::None)
            }
        })
}

 
 
pub(crate) fn verify_packed_attestation(
    acd: &AttestedCredentialData,
    att_obj: &AttestationObject<Registration>,
    client_data_hash: &[u8],
) -> Result<(ParsedAttestationData, AttestationMetadata), WebauthnError> {
    let att_stmt = &att_obj.att_stmt;
    let auth_data_bytes = &att_obj.auth_data_bytes;

     
    let att_stmt_map =
        cbor_try_map!(att_stmt).map_err(|_| WebauthnError::AttestationStatementMapInvalid)?;

    let x5c_key = &serde_cbor_2::Value::Text("x5c".to_string());
    let ecdaa_key_id_key = &serde_cbor_2::Value::Text("ecdaaKeyId".to_string());

    let alg_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("alg".to_string()))
        .ok_or(WebauthnError::AttestationStatementAlgMissing)?;

    let alg = cbor_try_i128!(alg_value)
        .map_err(|_| WebauthnError::AttestationStatementAlgInvalid)
        .and_then(|v| {
            COSEAlgorithm::try_from(v).map_err(|_| WebauthnError::COSEKeyInvalidAlgorithm)
        })?;

    trace!(x5c = ?att_stmt_map.get(x5c_key));
    trace!(ecdaa = ?att_stmt_map.get(ecdaa_key_id_key));

    match (
        att_stmt_map.get(x5c_key),
        att_stmt_map.get(ecdaa_key_id_key),
    ) {
        (Some(x5c), _) => {
             

             
             
             
             
            let x5c_array_ref =
                cbor_try_array!(x5c).map_err(|_| WebauthnError::AttestationStatementX5CInvalid)?;

            let arr_x509: Result<Vec<_>, _> = x5c_array_ref
                .iter()
                .map(|values| {
                    cbor_try_bytes!(values)
                        .map_err(|_| WebauthnError::AttestationStatementX5CInvalid)
                        .and_then(|b| {
                            x509::Certificate::from_der(b)
                                .map_err(|_| WebauthnError::X509DerInvalid)
                        })
                })
                .collect();

            let arr_x509 = arr_x509?;

             
            let attestn_cert = arr_x509
                .first()
                .ok_or(WebauthnError::AttestationStatementX5CInvalid)?;

            trace!(?attestn_cert);

             
             
             

            let verification_data: Vec<u8> = auth_data_bytes
                .iter()
                .chain(client_data_hash.iter())
                .copied()
                .collect();

            let is_valid_signature = att_stmt_map
                .get(&serde_cbor_2::Value::Text("sig".to_string()))
                .ok_or(WebauthnError::AttestationStatementSigMissing)
                .and_then(|s| cbor_try_bytes!(s))
                .and_then(|sig| verify_signature(attestn_cert, sig, &verification_data))?;

            if !is_valid_signature {
                trace!("packed x509 signature invalid");
                return Err(WebauthnError::AttestationStatementSigInvalid);
            }

             
             
             

            assert_packed_attest_req(attestn_cert)?;

             
             
             

            validate_extension::<FidoGenCeAaguid>(attestn_cert, &acd.aaguid)?;

             
             

             
             

            Ok((
                ParsedAttestationData::Basic(arr_x509),
                AttestationMetadata::Packed {
                    aaguid: Uuid::from_bytes(acd.aaguid),
                },
            ))
        }
        (None, Some(_ecdaa_key_id)) => {
             
             
            debug!("_ecdaa_key_id");
            Err(WebauthnError::AttestationNotSupported)
        }
        (None, None) => {
             
            let credential_public_key = COSEKey::try_from(&acd.credential_pk)?;

             
            if alg != credential_public_key.type_ {
                return Err(WebauthnError::AttestationStatementAlgMismatch);
            }

             
            let verification_data: Vec<u8> = auth_data_bytes
                .iter()
                .chain(client_data_hash.iter())
                .copied()
                .collect();

            let is_valid_signature = att_stmt_map
                .get(&serde_cbor_2::Value::Text("sig".to_string()))
                .ok_or(WebauthnError::AttestationStatementSigMissing)
                .and_then(|s| cbor_try_bytes!(s))
                .and_then(|sig| credential_public_key.verify_signature(sig, &verification_data))?;

            if !is_valid_signature {
                trace!("Invalid Self Attestation Signature");
                return Err(WebauthnError::AttestationStatementSigInvalid);
            }

             
            Ok((ParsedAttestationData::Self_, AttestationMetadata::None))
        }
    }
}

 
 
 
 
pub fn assert_packed_attest_req(pubk: &x509::Certificate) -> Result<(), WebauthnError> {
     
    let x509_cert = pubk.tbs_certificate();

     
     
    if x509_cert.version() != x509::Version::V3 {
        trace!("X509 Version != v3");
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
     
     
     
     
     
     
     
     
     
    let subject = x509_cert.subject();

    let mut subject_c_present = false;
    let mut subject_o_present = false;
    let mut subject_cn_present = false;
    let mut subject_ou_valid = false;

    for ava in subject.iter() {
        if ava.oid == rfc4519::C {
            subject_c_present = true
        } else if ava.oid == rfc4519::O {
            subject_o_present = true
        } else if ava.oid == rfc4519::CN {
            subject_cn_present = true
        } else if ava.oid == rfc4519::OU {
            let Ok(ava_value_str) = str::from_utf8(ava.value.value()) else {
                 
                trace!("ou invalid");
                return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
            };

            if ava_value_str == "Authenticator Attestation" {
                subject_ou_valid = true
            } else {
                trace!("ou != Authenticator Attestation");
                return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
            }
        }
    }

    if !(subject_c_present && subject_o_present && subject_cn_present && subject_ou_valid) {
        trace!("Invalid subject details");
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
     
     
     
     
     
     
     
     
    let fido_gen_ce_aaguid_critical = x509_cert.extensions().as_ref().and_then(|extensions| {
        extensions.iter().find_map(|extension| {
            (extension.extn_id == FidoGenCeAaguid::OID).then_some(extension.critical)
        })
    });

    if fido_gen_ce_aaguid_critical == Some(true) {
        trace!("fido ce aaguid critical");
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
    let basic_constraints = x509_cert
        .get_extension::<BasicConstraints>()
        .map_err(|_| {
            trace!("error reading extensions");
            WebauthnError::AttestationCertificateRequirementsNotMet
        })?
        .map(|(_crit, extn)| extn)
        .ok_or_else(|| {
            trace!("missing basic constraints");
            WebauthnError::AttestationCertificateRequirementsNotMet
        })?;

    if basic_constraints.ca {
        trace!("attestation cert must not be a ca!!!");
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
     
     
     

    Ok(())
}

 
 
pub(crate) fn verify_fidou2f_attestation(
    acd: &AttestedCredentialData,
    att_obj: &AttestationObject<Registration>,
    client_data_hash: &[u8],
) -> Result<ParsedAttestationData, WebauthnError> {
    let att_stmt = &att_obj.att_stmt;

     
     
     
     
     

     
     

     
    let att_stmt_map =
        cbor_try_map!(att_stmt).map_err(|_| WebauthnError::AttestationStatementMapInvalid)?;
    let x5c = att_stmt_map
        .get(&serde_cbor_2::Value::Text("x5c".to_string()))
        .ok_or(WebauthnError::AttestationStatementX5CMissing)?;

    let sig_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("sig".to_string()))
        .ok_or(WebauthnError::AttestationStatementSigMissing)?;

    let sig =
        cbor_try_bytes!(sig_value).map_err(|_| WebauthnError::AttestationStatementSigMissing)?;

     
    let att_cert_array =
        cbor_try_array!(x5c).map_err(|_| WebauthnError::AttestationStatementX5CInvalid)?;

     
    if att_cert_array.len() != 1 {
        return Err(WebauthnError::AttestationStatementX5CInvalid);
    }

    let arr_x509 = att_cert_array
        .iter()
        .map(|att_cert_bytes| {
            cbor_try_bytes!(att_cert_bytes).and_then(|att_cert| {
                x509::Certificate::from_der(att_cert.as_slice())
                    .map_err(|_| WebauthnError::X509DerInvalid)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

     
    let cerificate_public_key = arr_x509
        .first()
        .ok_or(WebauthnError::AttestationStatementX5CInvalid)?;

     
    let credential_public_key = COSEKey::try_from(&acd.credential_pk)?;

    match &credential_public_key {
        COSEKey {
            type_: COSEAlgorithm::ES256,
            ..
        } => {
             
        }
        _ => {
            return Err(WebauthnError::AttestationStatementX5CInvalid);
        }
    };

     
     
     

     

    let public_key_u2f = credential_public_key.get_alg_key_ecc_x962_raw()?;

     
    let r: [u8; 1] = [0x00];
    let verification_data: Vec<u8> = r
        .iter()
        .chain(att_obj.auth_data.rp_id_hash.iter())
        .chain(client_data_hash.iter())
        .chain(acd.credential_id.iter())
        .chain(public_key_u2f.iter())
        .copied()
        .collect();

     
    let verified = verify_signature(cerificate_public_key, sig, &verification_data)?;

    if !verified {
        error!("signature verification failed!");
        return Err(WebauthnError::AttestationStatementSigInvalid);
    }

    let attestation = ParsedAttestationData::Basic(arr_x509);
     

     

    Ok(attestation)
}

 
pub(crate) fn verify_tpm_attestation(
    acd: &AttestedCredentialData,
    att_obj: &AttestationObject<Registration>,
    client_data_hash: &[u8],
) -> Result<(ParsedAttestationData, AttestationMetadata), WebauthnError> {
    debug!("begin verify_tpm_attest");

    let att_stmt = &att_obj.att_stmt;
    let auth_data_bytes = &att_obj.auth_data_bytes;

     
     
    let att_stmt_map =
        cbor_try_map!(att_stmt).map_err(|_| WebauthnError::AttestationStatementMapInvalid)?;

     
    let ver_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("ver".to_string()))
        .ok_or(WebauthnError::AttestationStatementVerMissing)?;

    let ver =
        cbor_try_string!(ver_value).map_err(|_| WebauthnError::AttestationStatementVerInvalid)?;

    if ver != "2.0" {
        return Err(WebauthnError::AttestationStatementVerUnsupported);
    }

     
     
    let alg_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("alg".to_string()))
        .ok_or(WebauthnError::AttestationStatementAlgMissing)?;

    let alg = cbor_try_i128!(alg_value)
        .map_err(|_| WebauthnError::AttestationStatementAlgInvalid)
        .and_then(|v| {
            COSEAlgorithm::try_from(v).map_err(|_| WebauthnError::COSEKeyInvalidAlgorithm)
        })?;

     

     
     
    let certinfo_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("certInfo".to_string()))
        .ok_or(WebauthnError::AttestationStatementCertInfoMissing)?;

    let certinfo_bytes = cbor_try_bytes!(certinfo_value)
        .map_err(|_| WebauthnError::AttestationStatementCertInfoMissing)?;

    let certinfo = TpmsAttest::try_from(certinfo_bytes.as_slice())?;

     

     
     
    let pubarea_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("pubArea".to_string()))
        .ok_or(WebauthnError::AttestationStatementPubAreaMissing)?;

    let pubarea_bytes = cbor_try_bytes!(pubarea_value)
        .map_err(|_| WebauthnError::AttestationStatementPubAreaMissing)?;

    let pubarea = TpmtPublic::try_from(pubarea_bytes.as_slice())?;

     

     
     
    let sig_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("sig".to_string()))
        .ok_or(WebauthnError::AttestationStatementSigMissing)?;

    let sig_bytes =
        cbor_try_bytes!(sig_value).map_err(|_| WebauthnError::AttestationStatementSigMissing)?;

    let sig = TpmtSignature::try_from(sig_bytes.as_slice())?;

     

     
     
    let x5c_value = att_stmt_map
        .get(&serde_cbor_2::Value::Text("x5c".to_string()))
        .ok_or(WebauthnError::AttestationStatementX5CMissing)?;

    let x5c_array_ref =
        cbor_try_array!(x5c_value).map_err(|_| WebauthnError::AttestationStatementX5CInvalid)?;

    let arr_x509: Result<Vec<_>, _> = x5c_array_ref
        .iter()
        .map(|values| {
            cbor_try_bytes!(values)
                .map_err(|_| WebauthnError::AttestationStatementX5CInvalid)
                .and_then(|b| {
                    x509::Certificate::from_der(b).map_err(|_| WebauthnError::X509DerInvalid)
                })
        })
        .collect();

    let arr_x509 = arr_x509?;

     
    let aik_cert = arr_x509
        .first()
        .ok_or(WebauthnError::AttestationStatementX5CInvalid)?;

     
     
    let credential_public_key = COSEKey::try_from(&acd.credential_pk)?;

     
    match (
        &credential_public_key.key,
        &pubarea.parameters,
        &pubarea.unique,
    ) {
        (
            COSEKeyType::RSA(cose_rsa),
            TpmuPublicParms::Rsa(_tpm_parms),
            TpmuPublicId::Rsa(tpm_modulus),
        ) => {
             
             
             

             
            if cose_rsa.n.as_slice() != tpm_modulus.as_slice() {
                return Err(WebauthnError::AttestationTpmPubAreaMismatch);
            }
        }
        (
            COSEKeyType::EC_EC2(COSEEC2Key { curve, x, y }),
            TpmuPublicParms::Ecc(ecc_parms),
            TpmuPublicId::Ecc(ecc_points),
        ) => {
            match (curve, ecc_parms.curve_id) {
                (ECDSACurve::SECP256R1, TpmiEccCurve::NistP256)
                | (ECDSACurve::SECP384R1, TpmiEccCurve::NistP384)
                | (ECDSACurve::SECP521R1, TpmiEccCurve::NistP521) => {
                     
                }
                c_mismatch => {
                    debug!(?c_mismatch, "TpmiEccCurve ID mismatch");
                    return Err(WebauthnError::AttestationTpmPubAreaMismatch);
                }
            }

            if x.as_slice() != ecc_points.x || y.as_slice() != ecc_points.y {
                debug!("Invalid X or Y coords in TpmuPublicId");
                return Err(WebauthnError::AttestationTpmPubAreaMismatch);
            }
        }
        ex => {
            debug!(?ex, "Unrecognised combination");
            return Err(WebauthnError::AttestationTpmPubAreaMismatch);
        }
    }

     
    let verification_data: Vec<u8> = auth_data_bytes
        .iter()
        .chain(client_data_hash.iter())
        .copied()
        .collect();

     
     

     
     

     
    if certinfo.type_ != TpmSt::AttestCertify {
        return Err(WebauthnError::AttestationTpmStInvalid);
    }

    let extra_data_hash = match certinfo.extra_data {
        Some(h) => h,
        None => return Err(WebauthnError::AttestationTpmExtraDataInvalid),
    };

     
     
    let hash_verification_data = only_hash_from_type(alg, verification_data.as_slice())?;

    if hash_verification_data != extra_data_hash {
        return Err(WebauthnError::AttestationTpmExtraDataMismatch);
    }

     

     
     
     
     
     
    match certinfo.typeattested {
        TpmuAttest::AttestCertify(name, _qname) => {
            let name = match name {
                Tpm2bName::Digest(name) => name,
                _ => return Err(WebauthnError::AttestationTpmPubAreaHashInvalid),
            };
             
             
             
            let hname = match pubarea.name_alg {
                TpmAlgId::Sha256 => {
                    let mut v = vec![0, 11];
                    let r = compute_sha256(pubarea_bytes);
                    v.append(&mut r.to_vec());
                    v
                }
                _ => return Err(WebauthnError::AttestationTpmPubAreaHashUnknown),
            };
            if hname != name {
                return Err(WebauthnError::AttestationTpmPubAreaHashInvalid);
            }
        }
        _ => return Err(WebauthnError::AttestationTpmAttestCertifyInvalid),
    }

     
     

     
     
     
     
     

     
     

    let sig_valid = match sig {
        TpmtSignature::RawSignature(dsig) => {
             
             
            verify_signature(aik_cert, &dsig, certinfo_bytes)?
        }
    };

     

    if !sig_valid {
        return Err(WebauthnError::AttestationStatementSigInvalid);
    }

     
     
    assert_tpm_attest_req(aik_cert)?;

     
     

    validate_extension::<FidoGenCeAaguid>(aik_cert, &acd.aaguid)?;

     
     
    Ok((
        ParsedAttestationData::AttCa(arr_x509),
        AttestationMetadata::Tpm {
            aaguid: Uuid::from_bytes(acd.aaguid),
            firmware_version: certinfo.firmware_version,
        },
    ))
}

pub(crate) fn assert_tpm_attest_req(x509: &x509::Certificate) -> Result<(), WebauthnError> {
    let x509_cert = x509.tbs_certificate();

     

     
    if x509_cert.version() != x509::Version::V3 {
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
    if !x509_cert.subject().is_empty() {
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
     

     
     
     
     
     
     
     
     
     
     
    let subject_alt_name = x509_cert
        .get_extension::<SubjectAltName>()
        .map_err(|_| WebauthnError::AttestationCertificateRequirementsNotMet)?
        .and_then(|(critical, extn)| critical.then_some(extn))
        .ok_or(WebauthnError::AttestationCertificateRequirementsNotMet)?;

    let _tpm_san_data = TpmSanData::try_from(&subject_alt_name)
        .and_then(|san_data| {
            tpm_device_attribute_parser(san_data.manufacturer.as_bytes())
                .map_err(|_| WebauthnError::ParseNOMFailure)
        })
        .and_then(|(_, manufacturer_bytes)| TpmVendor::try_from(manufacturer_bytes))?;

     
    let extended_key_usage = x509_cert
        .get_extension::<ExtendedKeyUsage>()
        .map_err(|_| WebauthnError::AttestationCertificateRequirementsNotMet)?
        .and_then(|(critical, extn)| critical.then_some(extn))
        .ok_or(WebauthnError::AttestationCertificateRequirementsNotMet)?;

    if !extended_key_usage.0.contains(&OID_JOINT_ISO_ITU_T) {
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
    let basic_constraints = x509_cert
        .get_extension::<BasicConstraints>()
        .map_err(|_| WebauthnError::AttestationCertificateRequirementsNotMet)?
        .and_then(|(critical, extn)| critical.then_some(extn))
        .ok_or(WebauthnError::AttestationCertificateRequirementsNotMet)?;

    if basic_constraints.ca {
        return Err(WebauthnError::AttestationCertificateRequirementsNotMet);
    }

     
     
     

    Ok(())
}

pub(crate) fn verify_apple_anonymous_attestation(
    acd: &AttestedCredentialData,
    att_obj: &AttestationObject<Registration>,
    client_data_hash: &[u8],
) -> Result<(ParsedAttestationData, AttestationMetadata), WebauthnError> {
    let att_stmt = &att_obj.att_stmt;
    let auth_data_bytes = &att_obj.auth_data_bytes;

     
    let att_stmt_map =
        cbor_try_map!(att_stmt).map_err(|_| WebauthnError::AttestationStatementMapInvalid)?;

    let x5c_key = &serde_cbor_2::Value::Text("x5c".to_string());

    let x5c_value = att_stmt_map
        .get(x5c_key)
        .ok_or(WebauthnError::AttestationStatementX5CMissing)?;

    let x5c_array_ref =
        cbor_try_array!(x5c_value).map_err(|_| WebauthnError::AttestationStatementX5CInvalid)?;

    let credential_public_key = COSEKey::try_from(&acd.credential_pk)?;
    let alg = credential_public_key.type_;

    let arr_x509: Result<Vec<_>, _> = x5c_array_ref
        .iter()
        .map(|values| {
            cbor_try_bytes!(values)
                .map_err(|_| WebauthnError::AttestationStatementX5CInvalid)
                .and_then(|b| {
                    x509::Certificate::from_der(b).map_err(|_| WebauthnError::X509DerInvalid)
                })
        })
        .collect();

    let arr_x509 = arr_x509?;

     
    let attestn_cert = arr_x509
        .first()
        .ok_or(WebauthnError::AttestationStatementX5CInvalid)?;

     
    let nonce_to_hash: Vec<u8> = auth_data_bytes
        .iter()
        .chain(client_data_hash.iter())
        .copied()
        .collect();

     
    let nonce = compute_sha256(&nonce_to_hash);

     

    validate_extension::<AppleAnonymousNonce>(attestn_cert, &nonce)?;

     
    let subject_public_key = COSEKey::try_from((alg, attestn_cert))?;

    if credential_public_key != subject_public_key {
        return Err(WebauthnError::AttestationCredentialSubjectKeyMismatch);
    }

     
    Ok((
        ParsedAttestationData::AnonCa(arr_x509),
        AttestationMetadata::None,
    ))
}

 
pub(crate) fn verify_android_key_attestation(
    acd: &AttestedCredentialData,
    att_obj: &AttestationObject<Registration>,
    client_data_hash: &[u8],
) -> Result<(ParsedAttestationData, AttestationMetadata), WebauthnError> {
    let att_stmt = &att_obj.att_stmt;
    let auth_data_bytes = &att_obj.auth_data_bytes;

     
    let att_stmt_map =
        cbor_try_map!(att_stmt).map_err(|_| WebauthnError::AttestationStatementMapInvalid)?;

    let alg = {
        let alg_value = att_stmt_map
            .get(&serde_cbor_2::Value::Text("alg".to_string()))
            .ok_or(WebauthnError::AttestationStatementAlgMissing)?;

        cbor_try_i128!(alg_value)
            .map_err(|_| WebauthnError::AttestationStatementAlgInvalid)
            .and_then(|v| {
                COSEAlgorithm::try_from(v).map_err(|_| WebauthnError::COSEKeyInvalidAlgorithm)
            })?
    };

    let sig = {
        let sig_value = att_stmt_map
            .get(&serde_cbor_2::Value::Text("sig".to_string()))
            .ok_or(WebauthnError::AttestationStatementSigMissing)?;

        cbor_try_bytes!(sig_value).map_err(|_| WebauthnError::AttestationStatementSigMissing)?
    };

    let arr_x509 = {
        let x5c_key = &serde_cbor_2::Value::Text("x5c".to_string());

        let x5c_value = att_stmt_map
            .get(x5c_key)
            .ok_or(WebauthnError::AttestationStatementX5CMissing)?;

        let x5c_array_ref = cbor_try_array!(x5c_value)
            .map_err(|_| WebauthnError::AttestationStatementX5CInvalid)?;

        let arr_x509: Result<Vec<_>, _> = x5c_array_ref
            .iter()
            .map(|values| {
                cbor_try_bytes!(values)
                    .map_err(|_| WebauthnError::AttestationStatementX5CInvalid)
                    .and_then(|b| {
                        x509::Certificate::from_der(b).map_err(|_| WebauthnError::X509DerInvalid)
                    })
            })
            .collect();

        arr_x509?
    };

     
    let attestn_cert = arr_x509
        .first()
        .ok_or(WebauthnError::AttestationStatementX5CInvalid)?;

     
    let data_to_verify: Vec<u8> = auth_data_bytes
        .iter()
        .chain(client_data_hash.iter())
        .copied()
        .collect();

     

    let verified = verify_signature(attestn_cert, sig, &data_to_verify)?;

    if !verified {
        error!("signature verification failed!");
        return Err(WebauthnError::AttestationStatementSigInvalid);
    }

     
    let credential_public_key = COSEKey::try_from(&acd.credential_pk)?;
    let subject_public_key = COSEKey::try_from((credential_public_key.type_, attestn_cert))?;

    if credential_public_key != subject_public_key {
        return Err(WebauthnError::AttestationCredentialSubjectKeyMismatch);
    }

    if alg != credential_public_key.type_ {
        error!("Public key algorthim mismatch!");
        return Err(WebauthnError::AttestationStatementSigInvalid);
    }

     
     
     

     
     

    let meta = validate_extension::<AndroidKeyAttestationExtensionData>(
        attestn_cert,
        &client_data_hash.to_vec(),
    )?;

     
     
     
     

     
    Ok((ParsedAttestationData::Basic(arr_x509), meta))
}

 
pub fn verify_attestation_ca_chain<'a>(
    att_data: &'_ ParsedAttestationData,
    ca_list: &'a AttestationCaList,
    current_time: SystemTime,
) -> Result<Option<&'a AttestationCa>, WebauthnError> {
     
    if ca_list.cas().is_empty() {
        return Err(WebauthnError::AttestationCertificateTrustStoreEmpty);
    }

     
    let fullchain = match att_data {
        ParsedAttestationData::Basic(chain) => chain,
        ParsedAttestationData::AttCa(chain) => chain,
        ParsedAttestationData::AnonCa(chain) => chain,
        ParsedAttestationData::Self_ | ParsedAttestationData::None => {
             
            debug!("No attestation present");
            return Ok(None);
        }
        ParsedAttestationData::ECDAA | ParsedAttestationData::Uncertain => {
            debug!("attestation is an unsupported format");
            return Err(WebauthnError::AttestationNotVerifiable);
        }
    };

    for crt in fullchain {
        debug!(cert = %X509Display::from(crt));
    }

    let (leaf, chain) = fullchain
        .split_first()
        .ok_or(WebauthnError::AttestationLeafCertMissing)?;

     
    let cg_ca_list = ca_list
        .cas()
        .values()
        .map(|att_ca| att_ca.ca().clone())
        .collect::<Vec<_>>();

    for ca in cg_ca_list.iter() {
        debug!(ca = %X509Display::from(ca));
    }

    let mut store = x509::X509Store::new(&cg_ca_list);
     
    store.leaf_basic_constraints_required(false);

    store
        .verify(leaf, chain, current_time)
        .map_err(WebauthnError::AttestationChainNotTrusted)
        .map(|valid_ca| {
            ca_list
                .cas()
                .values()
                .find(|att_ca| att_ca.ca() == valid_ca)
        })
}
