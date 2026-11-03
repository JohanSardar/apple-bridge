 
 

use crate::constants::CREDENTIAL_ID_MAX_LENGTH;
use crate::error::WebauthnError;
use crate::proto::*;
use nom::bytes::complete::{tag, take};
use nom::combinator::cond;
use nom::combinator::{map_opt, verify};
use nom::error::ParseError;
use nom::number::complete::{be_u16, be_u32, be_u64};
use serde::Deserialize;

 
pub type UserId = Vec<u8>;

 
pub type Challenge = Vec<u8>;

 
pub type ChallengeRef<'a> = &'a [u8];

impl PartialEq<Credential> for Credential {
    fn eq(&self, c: &Credential) -> bool {
        self.cred_id == c.cred_id
    }
}

#[allow(clippy::too_many_arguments)]
impl Credential {
    pub(crate) fn new(
        acd: &AttestedCredentialData,
        auth_data: &AuthenticatorData<Registration>,
        ck: COSEKey,
        registration_policy: UserVerificationPolicy,
        attestation: ParsedAttestation,
        req_extn: &RequestRegistrationExtensions,
        client_extn: &RegistrationExtensionsClientOutputs,
        attestation_format: AttestationFormat,
        transports: &Option<Vec<AuthenticatorTransport>>,
    ) -> Self {
        let cred_protect = match (
            auth_data.extensions.cred_protect.as_ref(),
            req_extn.cred_protect.is_some(),
        ) {
            (Some(credprotect), false) => ExtnState::Unsolicited(credprotect.0),
            (Some(credprotect), true) => ExtnState::Set(credprotect.0),
            (None, true) => ExtnState::Ignored,
            (None, false) => ExtnState::NotRequested,
        };

        let hmac_create_secret = match (
            auth_data.extensions.hmac_secret.as_ref(),
            req_extn.hmac_create_secret.is_some(),
        ) {
            (Some(hmac_create_secret), false) => ExtnState::Unsolicited(*hmac_create_secret),
            (Some(hmac_create_secret), true) => ExtnState::Set(*hmac_create_secret),
            (None, true) => ExtnState::Ignored,
            (None, false) => ExtnState::NotRequested,
        };

        let appid = ExtnState::NotRequested;
        










        let cred_props = match (
            client_extn.cred_props.as_ref(),
            req_extn.cred_props.is_some(),
        ) {
            (Some(b), _) => ExtnState::Unsigned(b.clone()),
            (None, true) => ExtnState::Ignored,
            (None, false) => ExtnState::NotRequested,
        };

        let extensions = RegisteredExtensions {
            cred_protect,
            hmac_create_secret,
            appid,
            cred_props,
        };

        trace!(?extensions);
        let counter = auth_data.counter;
        let user_verified = auth_data.user_verified;
        let backup_eligible = auth_data.backup_eligible;
        let backup_state = auth_data.backup_state;

        let transports = if attestation_format == AttestationFormat::Packed
            || attestation_format == AttestationFormat::Tpm
        {
            transports.clone()
        } else {
            None
        };

        Credential {
            cred_id: acd.credential_id.clone(),
            cred: ck,
            counter,
            transports,
            user_verified,
            backup_eligible,
            backup_state,
            registration_policy,
            extensions,
            attestation,
            attestation_format,
        }
    }
}

pub(crate) fn process_authentication_extensions(
    auth_extn: &AuthenticationSignedExtensions,
) -> AuthenticationExtensions {
    trace!(?auth_extn);
    AuthenticationExtensions {}
}























impl TryFrom<i128> for ECDSACurve {
    type Error = WebauthnError;
    fn try_from(u: i128) -> Result<Self, Self::Error> {
        match u {
            1 => Ok(ECDSACurve::SECP256R1),
            2 => Ok(ECDSACurve::SECP384R1),
            3 => Ok(ECDSACurve::SECP521R1),
            _ => Err(WebauthnError::COSEKeyECDSAInvalidCurve),
        }
    }
}

impl TryFrom<i128> for EDDSACurve {
    type Error = WebauthnError;
    fn try_from(u: i128) -> Result<Self, Self::Error> {
        match u {
            6 => Ok(EDDSACurve::ED25519),
            7 => Ok(EDDSACurve::ED448),
            _ => Err(WebauthnError::COSEKeyEDDSAInvalidCurve),
        }
    }
}

fn cbor_parser(i: &[u8]) -> nom::IResult<&[u8], serde_cbor_2::Value> {
    let mut deserializer = serde_cbor_2::Deserializer::from_slice(i);
    let v = serde::de::Deserialize::deserialize(&mut deserializer).map_err(|e| {
        error!(?e, "cbor_parser");
        nom::Err::Failure(nom::error::Error::from_error_kind(
            i,
            nom::error::ErrorKind::Fail,
        ))
    })?;

    let len = deserializer.byte_offset();

    Ok((&i[len..], v))
}

fn extensions_parser<T: Ceremony>(i: &[u8]) -> nom::IResult<&[u8], T::SignedExtensions> {
    let (i, v) = cbor_parser(i)?;
    trace!(?v, "OK!");

    let v: T::SignedExtensions = serde_cbor_2::value::from_value(v).map_err(|e| {
        error!(?e, "extensions_parser");
        nom::Err::Failure(nom::error::Error::from_error_kind(
            i,
            nom::error::ErrorKind::Fail,
        ))
    })?;
    Ok((i, v))
}

fn aaguid_parser(i: &[u8]) -> nom::IResult<&[u8], Aaguid> {
    let (i, aaguid) = take(16usize)(i)?;
    Ok((i, aaguid.try_into().expect("took 16 bytes exactly")))
}

fn acd_parser(i: &[u8]) -> nom::IResult<&[u8], AttestedCredentialData> {
    let (i, aaguid) = aaguid_parser(i)?;
    let (i, cred_id_len) = be_u16(i)?;

    if cred_id_len > CREDENTIAL_ID_MAX_LENGTH {
        warn!(
            "cred_id_len ({:?}) exceeds the WebAuthn maximum of {:?} bytes.",
            cred_id_len, CREDENTIAL_ID_MAX_LENGTH
        );
        return Err(nom::Err::Failure(nom::error::Error::new(
            i,
            nom::error::ErrorKind::TooLarge,
        )));
    }

    if usize::from(cred_id_len) > i.len() {
        warn!(
            "cred_id_len ({:?}) is larger than remaining number of bytes to parse ({:?}).",
            cred_id_len,
            i.len()
        );
    }

    let (i, cred_id) = take(cred_id_len as usize)(i)?;
    let (i, cred_pk) = cbor_parser(i)?;

    Ok((
        i,
        AttestedCredentialData {
            aaguid,
            credential_id: cred_id.to_vec(),
            credential_pk: cred_pk,
        },
    ))
}
#[allow(clippy::type_complexity)]
fn authenticator_data_flags(i: &[u8]) -> nom::IResult<&[u8], (bool, bool, bool, bool, bool, bool)> {
     
    let (i, ctrl) = nom::number::complete::u8(i)?;
    let exten_pres = (ctrl & 0b1000_0000) != 0;
    let acd_pres = (ctrl & 0b0100_0000) != 0;
    let res_1 = (ctrl & 0b0010_0000) != 0;
    let bak_st = (ctrl & 0b0001_0000) != 0;
    let bak_el = (ctrl & 0b0000_1000) != 0;
    let u_ver = (ctrl & 0b0000_0100) != 0;
    let res_4 = (ctrl & 0b0000_0010) != 0;
    let u_pres = (ctrl & 0b0000_0001) != 0;

    if res_1 || res_4 {
        warn!(
            "Usage of unknown authenticator data flags detected! {:b}",
            ctrl
        );
    }

    Ok((i, (exten_pres, acd_pres, u_ver, u_pres, bak_el, bak_st)))
}

fn authenticator_data_parser<T: Ceremony>(i: &[u8]) -> nom::IResult<&[u8], AuthenticatorData<T>> {
    let (i, rp_id_hash) = take(32usize)(i)?;
    let (i, data_flags) = authenticator_data_flags(i)?;
    let (i, counter) = be_u32(i)?;
    let (i, acd) = cond(data_flags.1, acd_parser)(i)?;
    let (i, extensions) = cond(data_flags.0, extensions_parser::<T>)(i)?;
    trace!(?extensions);
    let extensions = extensions.unwrap_or_default();

    Ok((
        i,
        AuthenticatorData {
            rp_id_hash: rp_id_hash.try_into().expect("took 32 bytes from input"),
            counter,
            user_verified: data_flags.2,
            user_present: data_flags.3,
            backup_eligible: data_flags.4,
            backup_state: data_flags.5,
            acd,
            extensions,
        },
    ))
}

 
#[derive(Debug, Clone)]
pub struct AuthenticatorData<T: Ceremony> {
     
    pub(crate) rp_id_hash: [u8; 32],
     
    pub counter: u32,
     
    pub user_present: bool,
     
    pub user_verified: bool,
     
     
    pub backup_eligible: bool,
     
     
    pub backup_state: bool,
     
    pub acd: Option<AttestedCredentialData>,
     
    pub extensions: T::SignedExtensions,
}

impl<T: Ceremony> TryFrom<&[u8]> for AuthenticatorData<T> {
    type Error = WebauthnError;
    fn try_from(auth_data_bytes: &[u8]) -> Result<Self, Self::Error> {
        authenticator_data_parser(auth_data_bytes)
            .map_err(|e| {
                error!(?e, "try_from authenticator_data_parser");
                WebauthnError::ParseNOMFailure
            })
            .map(|(_, ad)| ad)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttestationObjectInner<'a> {
    pub(crate) fmt: &'a str,
    pub(crate) att_stmt: serde_cbor_2::Value,
    pub(crate) auth_data: &'a [u8],
    pub(crate) ep_att: Option<bool>,
    pub(crate) large_blob_key: Option<&'a [u8]>,
}

 
#[derive(Debug)]
pub struct AttestationObject<T: Ceremony> {
     
    pub(crate) fmt: String,
     
    pub(crate) att_stmt: serde_cbor_2::Value,
     
    pub(crate) auth_data: AuthenticatorData<T>,
     
    pub(crate) auth_data_bytes: Vec<u8>,
     
    pub(crate) _ep_att: Option<bool>,
     
    pub(crate) _large_blob_key: Option<Vec<u8>>,
}

impl<T: Ceremony> TryFrom<&[u8]> for AttestationObject<T> {
    type Error = WebauthnError;

    fn try_from(data: &[u8]) -> Result<AttestationObject<T>, WebauthnError> {
        let v: serde_cbor_2::Value =
            serde_cbor_2::from_slice(data).map_err(WebauthnError::ParseCBORFailure)?;
        trace!(?v, "AttestationObjectInner");

        let aoi: AttestationObjectInner =
            serde_cbor_2::from_slice(data).map_err(WebauthnError::ParseCBORFailure)?;
        let auth_data_bytes: &[u8] = aoi.auth_data;
        let auth_data = AuthenticatorData::try_from(auth_data_bytes)?;

         
        Ok(AttestationObject {
            fmt: aoi.fmt.to_owned(),
            att_stmt: aoi.att_stmt,
            auth_data,
            auth_data_bytes: auth_data_bytes.to_owned(),
            _ep_att: aoi.ep_att,
            _large_blob_key: aoi.large_blob_key.map(|v| v.to_owned()),
        })
    }
}

impl TryFrom<u8> for CredProtectResponse {
    type Error = <CredentialProtectionPolicy as TryFrom<u8>>::Error;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        CredentialProtectionPolicy::try_from(v).map(CredProtectResponse)
    }
}

impl From<CredProtectResponse> for u8 {
    fn from(policy: CredProtectResponse) -> Self {
        policy.0 as u8
    }
}

pub(crate) struct AuthenticatorAttestationResponse<T: Ceremony> {
    pub(crate) attestation_object: AttestationObject<T>,
    pub(crate) client_data_json: CollectedClientData,
    pub(crate) client_data_json_bytes: Vec<u8>,
    pub(crate) transports: Option<Vec<AuthenticatorTransport>>,
}

impl<T: Ceremony> TryFrom<&AuthenticatorAttestationResponseRaw>
    for AuthenticatorAttestationResponse<T>
{
    type Error = WebauthnError;
    fn try_from(aarr: &AuthenticatorAttestationResponseRaw) -> Result<Self, Self::Error> {
        let ccdj = serde_json::from_slice(aarr.client_data_json.as_ref())
            .map_err(WebauthnError::ParseJSONFailure)?;

        let ao = AttestationObject::try_from(aarr.attestation_object.as_ref())?;

        Ok(AuthenticatorAttestationResponse {
            attestation_object: ao,
            client_data_json: ccdj,
            client_data_json_bytes: aarr.client_data_json.clone(),
            transports: aarr.transports.clone(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct AuthenticatorAssertionResponse<T: Ceremony> {
    pub(crate) authenticator_data: AuthenticatorData<T>,
    pub(crate) authenticator_data_bytes: Vec<u8>,
    pub(crate) client_data: CollectedClientData,
    pub(crate) client_data_bytes: Vec<u8>,
    pub(crate) signature: Vec<u8>,
    pub(crate) _user_handle: Option<Vec<u8>>,
}

impl<T: Ceremony> TryFrom<&AuthenticatorAssertionResponseRaw>
    for AuthenticatorAssertionResponse<T>
{
    type Error = WebauthnError;
    fn try_from(aarr: &AuthenticatorAssertionResponseRaw) -> Result<Self, Self::Error> {
        Ok(AuthenticatorAssertionResponse {
            authenticator_data: AuthenticatorData::try_from(aarr.authenticator_data.as_ref())?,
            authenticator_data_bytes: aarr.authenticator_data.clone(),
            client_data: serde_json::from_slice(aarr.client_data_json.as_ref())
                .map_err(WebauthnError::ParseJSONFailure)?,
            client_data_bytes: aarr.client_data_json.clone(),
            signature: aarr.signature.clone(),
            _user_handle: aarr.user_handle.clone(),
        })
    }
}

 

 
pub const TPM_GENERATED_VALUE: u32 = 0xff544347;

#[derive(Debug, PartialEq, Eq)]
#[repr(u16)]
 
pub enum TpmSt {
     
    RspCommand = 0x00c4,
     
    Null = 0x8000,
     
    NoSessions = 0x8001,
     
    Sessions = 0x8002,
     
    ReservedA = 0x8003,
     
    ReservedB = 0x8004,
     
    AttestNV = 0x8014,
     
    AttestCommandAudit = 0x8015,
     
    AttestSessionAudit = 0x8016,
     
    AttestCertify = 0x8017,
     
    AttestQuote = 0x8018,
     
    AttestTime = 0x8019,
     
    AttestCreation = 0x801a,
     
    ReservedC = 0x801b,
     
    Creation = 0x8021,
     
    Verified = 0x8022,
     
    AuthSecret = 0x8023,
     
    Hashcheck = 0x8024,
     
    AuthSigned = 0x8025,
     
    FUManifest = 0x8029,
}

impl TpmSt {
    fn new(v: u16) -> Option<Self> {
        match v {
            0x00c4 => Some(TpmSt::RspCommand),
            0x8000 => Some(TpmSt::Null),
            0x8001 => Some(TpmSt::NoSessions),
            0x8002 => Some(TpmSt::Sessions),
            0x8003 => Some(TpmSt::ReservedA),
            0x8004 => Some(TpmSt::ReservedB),
            0x8014 => Some(TpmSt::AttestNV),
            0x8015 => Some(TpmSt::AttestCommandAudit),
            0x8016 => Some(TpmSt::AttestSessionAudit),
            0x8017 => Some(TpmSt::AttestCertify),
            0x8018 => Some(TpmSt::AttestQuote),
            0x8019 => Some(TpmSt::AttestTime),
            0x801a => Some(TpmSt::AttestCreation),
            0x801b => Some(TpmSt::ReservedC),
            0x8021 => Some(TpmSt::Creation),
            0x8022 => Some(TpmSt::Verified),
            0x8023 => Some(TpmSt::AuthSecret),
            0x8024 => Some(TpmSt::Hashcheck),
            0x8025 => Some(TpmSt::AuthSigned),
            0x8029 => Some(TpmSt::FUManifest),
            _ => None,
        }
    }
}

#[derive(Debug)]
 
pub struct TpmsClockInfo {
    _clock: u64,
    _reset_count: u32,
    _restart_count: u32,
    _safe: bool,  
}

fn tpmsclockinfo_parser(i: &[u8]) -> nom::IResult<&[u8], TpmsClockInfo> {
    let (i, clock) = be_u64(i)?;
    let (i, reset_count) = be_u32(i)?;
    let (i, restart_count) = be_u32(i)?;
    let (i, safe) = nom::combinator::map(nom::number::complete::u8, |v| v != 0)(i)?;

    Ok((
        i,
        TpmsClockInfo {
            _clock: clock,
            _reset_count: reset_count,
            _restart_count: restart_count,
            _safe: safe,
        },
    ))
}

#[derive(Debug)]
 
pub enum Tpm2bName {
     
    None,
     
    Handle(u32),
     
    Digest(Vec<u8>),
}

#[derive(Debug)]
 
pub enum TpmuAttest {
     
    AttestCertify(Tpm2bName, Tpm2bName),
     
     
     
     
     
     
    #[allow(dead_code)]
     
    Invalid,
}

#[derive(Debug)]
 
pub struct TpmsAttest {
     
    pub magic: u32,
     
    pub type_: TpmSt,
     
    pub qualified_signer: Tpm2bName,
     
    pub extra_data: Option<Vec<u8>>,
     
    pub clock_info: TpmsClockInfo,
     
    pub firmware_version: u64,
     
    pub typeattested: TpmuAttest,
}

fn tpm2b_name(i: &[u8]) -> nom::IResult<&[u8], Tpm2bName> {
    let (i, case) = be_u16(i)?;
    let (i, r) = match case {
        0 => (i, Tpm2bName::None),
        4 => {
            let (i, h) = be_u32(i)?;
            (i, Tpm2bName::Handle(h))
        }
        size => {
            let (i, d) = take(size as usize)(i)?;
            (i, Tpm2bName::Digest(d.to_vec()))
        }
    };

    Ok((i, r))
}

fn tpm2b_data(i: &[u8]) -> nom::IResult<&[u8], Option<Vec<u8>>> {
    let (i, size) = be_u16(i)?;
    match size {
        0 => Ok((i, None)),
        size => {
            let (i, d) = take(size as usize)(i)?;
            Ok((i, Some(d.to_vec())))
        }
    }
}

fn tpmuattestcertify(i: &[u8]) -> nom::IResult<&[u8], TpmuAttest> {
    let (i, name) = tpm2b_name(i)?;
    let (i, qualified_name) = tpm2b_name(i)?;

    Ok((i, TpmuAttest::AttestCertify(name, qualified_name)))
}

fn tpmsattest_parser(i: &[u8]) -> nom::IResult<&[u8], TpmsAttest> {
    let (i, magic) = verify(be_u32, |x| *x == TPM_GENERATED_VALUE)(i)?;
    let (i, type_) = map_opt(be_u16, TpmSt::new)(i)?;
    let (i, qualified_signer) = tpm2b_name(i)?;
    let (i, extra_data) = tpm2b_data(i)?;
    let (i, clock_info) = tpmsclockinfo_parser(i)?;
    let (i, firmware_version) = be_u64(i)?;
    let (i, typeattested) = tpmuattestcertify(i)?;

    Ok((
        i,
        TpmsAttest {
            magic,
            type_,
            qualified_signer,
            extra_data,
            clock_info,
            firmware_version,
            typeattested,
        },
    ))
}

impl TryFrom<&[u8]> for TpmsAttest {
    type Error = WebauthnError;

    fn try_from(data: &[u8]) -> Result<TpmsAttest, WebauthnError> {
        tpmsattest_parser(data)
            .map_err(|e| {
                error!(?e, "try_from tpmsattest_parser");
                WebauthnError::ParseNOMFailure
            })
            .map(|(_, v)| v)
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
 
pub enum TpmAlgId {
     
    Error = 0x0000,
     
    Rsa = 0x0001,
     
    Sha1 = 0x0004,
     
    Hmac = 0x0005,
     
    Aes = 0x0006,
     
     
     
     
    Sha256 = 0x000B,
     
    Sha384 = 0x000C,
     
    Sha512 = 0x000D,
     
    Null = 0x0010,
     
     
     
    RsaSSA = 0x0014,
     
     
    RsaPSS = 0x0016,
     
     
    Ecdsa = 0x0018,
     
     
    Ecdaa = 0x001A,
     
     
     
     
     
     
     
    Ecc = 0x0023,
     
     
     
     
     
     
     
}

impl TpmAlgId {
    fn new(v: u16) -> Option<Self> {
        trace!("TpmAlgId::new ( {:x?} )", v);
        match v {
            0x0000 => Some(TpmAlgId::Error),
            0x0001 => Some(TpmAlgId::Rsa),
            0x0004 => Some(TpmAlgId::Sha1),
            0x0005 => Some(TpmAlgId::Hmac),
            0x0006 => Some(TpmAlgId::Aes),
            0x000B => Some(TpmAlgId::Sha256),
            0x000C => Some(TpmAlgId::Sha384),
            0x000D => Some(TpmAlgId::Sha512),
            0x0010 => Some(TpmAlgId::Null),
            0x0014 => Some(TpmAlgId::RsaSSA),
            0x0016 => Some(TpmAlgId::RsaPSS),
            0x0018 => Some(TpmAlgId::Ecdsa),
            0x001A => Some(TpmAlgId::Ecdaa),
            0x0023 => Some(TpmAlgId::Ecc),
            _ => None,
        }
    }
}

 
 
#[derive(Debug)]
 
pub struct TpmtSymDefObject {
    _algorithm: TpmAlgId,
     
     
     
}

fn parse_tpmtsymdefobject(input: &[u8]) -> nom::IResult<&[u8], Option<TpmtSymDefObject>> {
    let (data, algorithm) = map_opt(be_u16, TpmAlgId::new)(input)?;
    match algorithm {
        TpmAlgId::Null => Ok((data, None)),
        _ => Err(nom::Err::Failure(nom::error::Error::from_error_kind(
            input,
            nom::error::ErrorKind::Fail,
        ))),
    }
}

#[derive(Debug)]
 
pub struct TpmtEccScheme {
    _algorithm: TpmAlgId,
}

fn parse_tpmteccscheme(input: &[u8]) -> nom::IResult<&[u8], Option<TpmtEccScheme>> {
    let (data, algorithm) = map_opt(be_u16, TpmAlgId::new)(input)?;
    match algorithm {
        TpmAlgId::Null => Ok((data, None)),
        _ => Err(nom::Err::Failure(nom::error::Error::from_error_kind(
            input,
            nom::error::ErrorKind::Fail,
        ))),
    }
}

 
 
#[derive(Debug, Clone, Copy)]
#[repr(u16)]
pub(crate) enum TpmiEccCurve {
    None = 0x0000,
    NistP192 = 0x0001,
    NistP224 = 0x0002,
    NistP256 = 0x0003,
    NistP384 = 0x0004,
    NistP521 = 0x0005,
    BnP256 = 0x0010,
    BnP638 = 0x0011,
    Sm2P256 = 0x0020,
}

impl TpmiEccCurve {
    fn new(v: u16) -> Option<Self> {
        trace!("TpmiEccCurve::new ( {:x?} )", v);
        match v {
            0x0000 => Some(TpmiEccCurve::None),
            0x0001 => Some(TpmiEccCurve::NistP192),
            0x0002 => Some(TpmiEccCurve::NistP224),
            0x0003 => Some(TpmiEccCurve::NistP256),
            0x0004 => Some(TpmiEccCurve::NistP384),
            0x0005 => Some(TpmiEccCurve::NistP521),
            0x0010 => Some(TpmiEccCurve::BnP256),
            0x0011 => Some(TpmiEccCurve::BnP638),
            0x0020 => Some(TpmiEccCurve::Sm2P256),
            _ => None,
        }
    }
}

#[derive(Debug)]
 
pub struct TpmtKdfScheme {
    _algorithm: TpmAlgId,
}

fn parse_tpmtkdfscheme(input: &[u8]) -> nom::IResult<&[u8], Option<TpmtKdfScheme>> {
    let (data, algorithm) = map_opt(be_u16, TpmAlgId::new)(input)?;
    match algorithm {
        TpmAlgId::Null => Ok((data, None)),
        _ => Err(nom::Err::Failure(nom::error::Error::from_error_kind(
            input,
            nom::error::ErrorKind::Fail,
        ))),
    }
}

#[derive(Debug)]
 
pub struct TpmtRsaScheme {
    _algorithm: TpmAlgId,
     
}

fn parse_tpmtrsascheme(input: &[u8]) -> nom::IResult<&[u8], Option<TpmtRsaScheme>> {
    let (data, algorithm) = map_opt(be_u16, TpmAlgId::new)(input)?;
    match algorithm {
        TpmAlgId::Null => Ok((data, None)),
        _ => Err(nom::Err::Failure(nom::error::Error::from_error_kind(
            input,
            nom::error::ErrorKind::Fail,
        ))),
    }
}

#[derive(Debug)]
 
pub(crate) struct TpmsRsaParms {
     
    _symmetric: Option<TpmtSymDefObject>,
     
    _scheme: Option<TpmtRsaScheme>,
     
    _keybits: u16,
     
     
    _exponent: u32,
}

fn tpmsrsaparms_parser(i: &[u8]) -> nom::IResult<&[u8], TpmsRsaParms> {
    let (i, symmetric) = parse_tpmtsymdefobject(i)?;
    let (i, scheme) = parse_tpmtrsascheme(i)?;
    let (i, keybits) = be_u16(i)?;
    let (i, exponent) = be_u32(i)?;
    Ok((
        i,
        TpmsRsaParms {
            _symmetric: symmetric,
            _scheme: scheme,
            _keybits: keybits,
            _exponent: exponent,
        },
    ))
    















}

 
fn tpmseccparms_parser(i: &[u8]) -> nom::IResult<&[u8], TpmsEccParms> {
    trace!(?i);
    let (i, symmetric) = parse_tpmtsymdefobject(i)?;
    let (i, scheme) = parse_tpmteccscheme(i)?;
    let (i, curve_id) = map_opt(be_u16, TpmiEccCurve::new)(i)?;
    let (i, kdf) = parse_tpmtkdfscheme(i)?;

    Ok((
        i,
        TpmsEccParms {
            _symmetric: symmetric,
            _scheme: scheme,
            curve_id,
            _kdf: kdf,
        },
    ))
}

#[derive(Debug)]
pub(crate) struct TpmsEccParms {
    _symmetric: Option<TpmtSymDefObject>,
    _scheme: Option<TpmtEccScheme>,
     
    pub curve_id: TpmiEccCurve,
    _kdf: Option<TpmtKdfScheme>,
}

#[derive(Debug)]
 
pub(crate) enum TpmuPublicParms {
     
     
     
    Rsa(TpmsRsaParms),
    Ecc(TpmsEccParms),
     
}

fn parse_tpmupublicparms(input: &[u8], alg: TpmAlgId) -> nom::IResult<&[u8], TpmuPublicParms> {
    trace!(?input, ?alg, "tpmupublicparms input");
    match alg {
        TpmAlgId::Rsa => {
            tpmsrsaparms_parser(input).map(|(data, inner)| (data, TpmuPublicParms::Rsa(inner)))
        }
        TpmAlgId::Ecc => {
            tpmseccparms_parser(input).map(|(data, inner)| (data, TpmuPublicParms::Ecc(inner)))
        }
        a => {
            debug!(?a, "unsuported alg in parse_tpmupublicparms");
            Err(nom::Err::Failure(nom::error::Error::from_error_kind(
                input,
                nom::error::ErrorKind::Fail,
            )))
        }
    }
}

 
#[derive(Debug)]
pub(crate) struct TpmsEccPoint {
    pub x: Vec<u8>,
    pub y: Vec<u8>,
}

fn tpmseccpoint_parser(i: &[u8]) -> nom::IResult<&[u8], TpmsEccPoint> {
    let (i, size) = be_u16(i)?;
    let (i, x) = match size {
        0 => (i, Vec::new()),
        size => {
            let (i, d) = take(size as usize)(i)?;
            (i, d.to_vec())
        }
    };
    let (i, size) = be_u16(i)?;
    let (i, y) = match size {
        0 => (i, Vec::new()),
        size => {
            let (i, d) = take(size as usize)(i)?;
            (i, d.to_vec())
        }
    };
    Ok((i, TpmsEccPoint { x, y }))
}

#[derive(Debug)]
 
pub(crate) enum TpmuPublicId {
     
     
     
    Rsa(Vec<u8>),
     
    Ecc(TpmsEccPoint),
     
}

fn tpmsrsapublickey_parser(i: &[u8]) -> nom::IResult<&[u8], Vec<u8>> {
    let (i, size) = be_u16(i)?;
    match size {
        0 => Ok((i, Vec::new())),
        size => {
            let (i, d) = take(size as usize)(i)?;
            Ok((i, d.to_vec()))
        }
    }
}

fn parse_tpmupublicid(input: &[u8], alg: TpmAlgId) -> nom::IResult<&[u8], TpmuPublicId> {
     
    match alg {
        TpmAlgId::Rsa => {
            tpmsrsapublickey_parser(input).map(|(data, inner)| (data, TpmuPublicId::Rsa(inner)))
        }
        TpmAlgId::Ecc => {
            tpmseccpoint_parser(input).map(|(data, inner)| (data, TpmuPublicId::Ecc(inner)))
        }
        a => {
            debug!(?a, "unsuported alg in parse_tpmupublicid");
            Err(nom::Err::Failure(nom::error::Error::from_error_kind(
                input,
                nom::error::ErrorKind::Fail,
            )))
        }
    }
}

#[derive(Debug)]
 
pub(crate) struct TpmtPublic {
     
    pub _type: TpmAlgId,
     
    pub name_alg: TpmAlgId,
     
     
    pub _object_attributes: u32,
     
    pub _auth_policy: Option<Vec<u8>>,
     
     
     
    pub parameters: TpmuPublicParms,
     
     
    pub unique: TpmuPublicId,
}

impl TryFrom<&[u8]> for TpmtPublic {
    type Error = WebauthnError;

    fn try_from(data: &[u8]) -> Result<TpmtPublic, WebauthnError> {
        trace!(?data);
        tpmtpublic_parser(data)
            .map_err(|e| {
                error!(?e, "try_from tpmtpublic_parser");
                WebauthnError::ParseNOMFailure
            })
            .map(|(_, v)| v)
    }
}

fn tpm2b_digest(i: &[u8]) -> nom::IResult<&[u8], Option<Vec<u8>>> {
    let (i, size) = be_u16(i)?;
    match size {
        0 => Ok((i, None)),
        size => {
            let (i, d) = take(size as usize)(i)?;
            Ok((i, Some(d.to_vec())))
        }
    }
}

fn tpmtpublic_parser(i: &[u8]) -> nom::IResult<&[u8], TpmtPublic> {
    let (i, type_) = map_opt(be_u16, TpmAlgId::new)(i)?;
    let (i, name_alg) = map_opt(be_u16, TpmAlgId::new)(i)?;
    let (i, _object_attributes) = be_u32(i)?;
    let (i, _auth_policy) = tpm2b_digest(i)?;
    let (i, parameters) = parse_tpmupublicparms(i, type_)?;
    let (i, unique) = parse_tpmupublicid(i, type_)?;

    Ok((
        i,
        TpmtPublic {
            _type: type_,
            name_alg,
            _object_attributes,
            _auth_policy,
            parameters,
            unique,
        },
    ))
}

#[derive(Debug)]
 
pub enum TpmtSignature {
     
     
     
     
     
    RawSignature(Vec<u8>),
}

impl TryFrom<&[u8]> for TpmtSignature {
    type Error = WebauthnError;

    fn try_from(data: &[u8]) -> Result<TpmtSignature, WebauthnError> {
        tpmtsignature_parser(data)
            .map_err(|e| {
                error!(?e, "try_from tpmtsignature_parser");
                WebauthnError::ParseNOMFailure
            })
            .map(|(_, v)| v)
    }
}

fn tpmtsignature_parser(input: &[u8]) -> nom::IResult<&[u8], TpmtSignature> {
    let (_data, algorithm) = nom::combinator::map(be_u16, TpmAlgId::new)(input)?;
    match algorithm {
        None => Ok((&[], TpmtSignature::RawSignature(Vec::from(input)))),
        _ => Err(nom::Err::Failure(nom::error::Error::from_error_kind(
            input,
            nom::error::ErrorKind::Fail,
        ))),
    }
}

 
 
 
#[derive(Debug)]
#[allow(clippy::upper_case_acronyms)]
pub(crate) enum TpmVendor {
    AMD,
    Atmel,
    Broadcom,
    Cisco,
    FlySliceTechnologies,
    FuzhouRockchip,
    Google,
    HPE,
    Huawei,
    IBM,
    Infineon,
    Intel,
    Lenovo,
    Microsoft,
    NationalSemi,
    Nationz,
    NuvotonTechnology,
    Qualcomm,
    Samsung,
    Sinosun,
    SMSC,
    StMicroelectronics,
    TexasInstruments,
    Winbond,
}

 
 
 
 
impl TryFrom<&[u8; 8]> for TpmVendor {
    type Error = WebauthnError;

    fn try_from(v: &[u8; 8]) -> Result<Self, Self::Error> {
        match v {
            b"414d4400" => Ok(Self::AMD),
            b"41544D4C" => Ok(Self::Atmel),
            b"4252434D" => Ok(Self::Broadcom),
            b"4353434F" => Ok(Self::Cisco),
            b"464C5953" => Ok(Self::FlySliceTechnologies),
            b"524F4343" => Ok(Self::FuzhouRockchip),
            b"474F4F47" => Ok(Self::Google),
            b"48504500" => Ok(Self::HPE),
            b"48495349" => Ok(Self::Huawei),
            b"49424D00" => Ok(Self::IBM),
            b"49465800" => Ok(Self::Infineon),
            b"494E5443" => Ok(Self::Intel),
            b"4C454E00" => Ok(Self::Lenovo),
            b"4D534654" => Ok(Self::Microsoft),
            b"4E534D20" => Ok(Self::NationalSemi),
            b"4E545A00" => Ok(Self::Nationz),
            b"4E544300" => Ok(Self::NuvotonTechnology),
            b"51434F4D" => Ok(Self::Qualcomm),
            b"534D534E" => Ok(Self::Samsung),
            b"534E5300" => Ok(Self::Sinosun),
            b"534D5343" => Ok(Self::SMSC),
            b"53544D20" => Ok(Self::StMicroelectronics),
            b"54584E00" => Ok(Self::TexasInstruments),
            b"57454300" => Ok(Self::Winbond),
            _ => Err(WebauthnError::ParseNOMFailure),
        }
    }
}

 
 
pub(crate) fn tpm_device_attribute_parser(i: &[u8]) -> nom::IResult<&[u8], &[u8; 8]> {
    let (i, _) = tag("id:")(i)?;
    let (i, vendor_code) = take(8usize)(i)?;

    Ok((i, vendor_code.try_into().expect("took exactly 8 bytes")))
}

impl TryFrom<&[u8]> for TpmVendor {
    type Error = WebauthnError;
    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        tpm_device_attribute_parser(data)
            .map_err(|e| {
                error!(?e, "try_from tpm_device_attribute_parser");
                WebauthnError::ParseNOMFailure
            })
            .and_then(|(_, v)| TpmVendor::try_from(v))
    }
}

