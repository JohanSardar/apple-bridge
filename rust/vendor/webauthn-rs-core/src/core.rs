 
 
 
 
 
 
 
 
 
 
 
 
 
 

#![allow(missing_docs)]

use crate::attestation::{
    verify_android_key_attestation, verify_apple_anonymous_attestation,
    verify_attestation_ca_chain, verify_fidou2f_attestation, verify_packed_attestation,
    verify_tpm_attestation,
};
use crate::constants::CHALLENGE_SIZE_BYTES;
use crate::crypto::compute_sha256;
use crate::error::WebauthnError;
use crate::internals::*;
use crate::proto::origin::origins_match;
use crate::proto::*;
use rand::prelude::*;
use std::time::{Duration, SystemTime};
use url::Url;

 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
#[derive(Debug, Clone)]
pub struct WebauthnCore {
    rp_name: String,
    rp_id: String,
    rp_id_hash: [u8; 32],
    allowed_origins: Vec<Url>,
    authenticator_timeout: Duration,
    require_valid_counter_value: bool,
    #[allow(unused)]
    ignore_unsupported_attestation_formats: bool,
    allow_cross_origin: bool,
    allow_subdomains_origin: bool,
    allow_any_port: bool,
}

 
pub struct ChallengeRegisterBuilder {
    user_unique_id: Vec<u8>,
    user_name: String,
    user_display_name: String,
    attestation: AttestationConveyancePreference,
    policy: UserVerificationPolicy,
    exclude_credentials: Option<Vec<CredentialID>>,
    extensions: Option<RequestRegistrationExtensions>,
    credential_algorithms: Vec<COSEAlgorithm>,
    require_resident_key: bool,
    authenticator_attachment: Option<AuthenticatorAttachment>,
    attestation_formats: Option<Vec<AttestationFormat>>,
    reject_synchronised_authenticators: bool,
    hints: Option<Vec<PublicKeyCredentialHints>>,
}

 
#[derive(Debug)]
pub struct ChallengeAuthenticateBuilder {
    creds: Vec<Credential>,
    policy: UserVerificationPolicy,
    extensions: Option<RequestAuthenticationExtensions>,
    allow_backup_eligible_upgrade: bool,
    hints: Option<Vec<PublicKeyCredentialHints>>,
}

impl ChallengeRegisterBuilder {
     
    pub fn attestation(mut self, value: AttestationConveyancePreference) -> Self {
        self.attestation = value;
        self
    }

     
    pub fn user_verification_policy(mut self, value: UserVerificationPolicy) -> Self {
        self.policy = value;
        self
    }

     
     
     
    pub fn exclude_credentials(mut self, value: Option<Vec<CredentialID>>) -> Self {
        self.exclude_credentials = value;
        self
    }

     
    pub fn extensions(mut self, value: Option<RequestRegistrationExtensions>) -> Self {
        self.extensions = value;
        self
    }

     
    pub fn credential_algorithms(mut self, value: Vec<COSEAlgorithm>) -> Self {
        self.credential_algorithms = value;
        self
    }

     
     
    pub fn require_resident_key(mut self, value: bool) -> Self {
        self.require_resident_key = value;
        self
    }

     
    pub fn authenticator_attachment(mut self, value: Option<AuthenticatorAttachment>) -> Self {
        self.authenticator_attachment = value;
        self
    }

     
    pub fn reject_synchronised_authenticators(mut self, value: bool) -> Self {
        self.reject_synchronised_authenticators = value;
        self
    }

     
    pub fn hints(mut self, hints: Option<Vec<PublicKeyCredentialHints>>) -> Self {
        self.hints = hints;
        self
    }

     
    pub fn attestation_formats(
        mut self,
        attestation_formats: Option<Vec<AttestationFormat>>,
    ) -> Self {
        self.attestation_formats = attestation_formats;
        self
    }
}

impl ChallengeAuthenticateBuilder {
     
    pub fn extensions(mut self, value: Option<RequestAuthenticationExtensions>) -> Self {
        self.extensions = value;
        self
    }

     
     
     
     
    pub fn allow_backup_eligible_upgrade(mut self, value: bool) -> Self {
        self.allow_backup_eligible_upgrade = value;
        self
    }

     
    pub fn hints(mut self, hints: Option<Vec<PublicKeyCredentialHints>>) -> Self {
        self.hints = hints;
        self
    }
}

impl WebauthnCore {
     
     
     
     
     
     
     
     
     
     
     
     
     
     
     
     
    pub fn new_unsafe_experts_only(
        rp_name: &str,
        rp_id: &str,
        allowed_origins: Vec<Url>,
        authenticator_timeout: Duration,
        allow_subdomains_origin: Option<bool>,
        allow_any_port: Option<bool>,
    ) -> Self {
        let rp_id_hash = compute_sha256(rp_id.as_bytes());
        WebauthnCore {
            rp_name: rp_name.to_string(),
            rp_id: rp_id.to_string(),
            rp_id_hash,
            allowed_origins,
            authenticator_timeout,
            require_valid_counter_value: true,
            ignore_unsupported_attestation_formats: true,
            allow_cross_origin: false,
            allow_subdomains_origin: allow_subdomains_origin.unwrap_or(false),
            allow_any_port: allow_any_port.unwrap_or(false),
        }
    }

     
    pub fn get_allowed_origins(&self) -> &[Url] {
        &self.allowed_origins
    }

    fn generate_challenge(&self) -> Challenge {
        let mut rng = rand::rng();
        Challenge::from(rng.random::<[u8; CHALLENGE_SIZE_BYTES]>())
    }

     
     
     
    pub fn new_challenge_register_builder(
        &self,
        user_unique_id: &[u8],
        user_name: &str,
        user_display_name: &str,
    ) -> Result<ChallengeRegisterBuilder, WebauthnError> {
        if user_unique_id.is_empty() || user_display_name.is_empty() || user_name.is_empty() {
            return Err(WebauthnError::InvalidUsername);
        }

        Ok(ChallengeRegisterBuilder {
            user_unique_id: user_unique_id.to_vec(),
            user_name: user_name.to_string(),
            user_display_name: user_display_name.to_string(),
            credential_algorithms: COSEAlgorithm::secure_algs(),
            attestation: Default::default(),
            policy: UserVerificationPolicy::Preferred,
            exclude_credentials: Default::default(),
            extensions: Default::default(),
            require_resident_key: Default::default(),
            authenticator_attachment: Default::default(),
            reject_synchronised_authenticators: Default::default(),
            hints: Default::default(),
            attestation_formats: Default::default(),
        })
    }

     
     
     
     
     
     
     
     
     
     
     
     
    #[allow(clippy::too_many_arguments)]
    pub fn generate_challenge_register(
        &self,
        challenge_builder: ChallengeRegisterBuilder,
    ) -> Result<(CreationChallengeResponse, RegistrationState), WebauthnError> {
        let ChallengeRegisterBuilder {
            user_unique_id,
            user_name,
            user_display_name,
            attestation,
            policy,
            exclude_credentials,
            extensions,
            credential_algorithms,
            require_resident_key,
            authenticator_attachment,
            attestation_formats,
            reject_synchronised_authenticators,
            hints,
        } = challenge_builder;

        let challenge = self.generate_challenge();

        let resident_key = if require_resident_key {
            Some(ResidentKeyRequirement::Required)
        } else {
            Some(ResidentKeyRequirement::Discouraged)
        };

        let timeout_millis =
            u32::try_from(self.authenticator_timeout.as_millis()).expect("Timeout too large");

        let c = CreationChallengeResponse {
            public_key: PublicKeyCredentialCreationOptions {
                rp: RelyingParty {
                    name: self.rp_name.clone(),
                    id: self.rp_id.clone(),
                },
                user: User {
                    id: user_unique_id,
                    name: user_name,
                    display_name: user_display_name,
                },
                challenge: challenge.to_owned(),
                pub_key_cred_params: credential_algorithms
                    .iter()
                    .map(|alg| PubKeyCredParams {
                        type_: "public-key".to_string(),
                        alg: *alg as i64,
                    })
                    .collect(),
                timeout: Some(timeout_millis),
                hints,
                attestation: Some(attestation),
                exclude_credentials: exclude_credentials.as_ref().map(|creds| {
                    creds
                        .iter()
                        .map(|id| PublicKeyCredentialDescriptor {
                            type_: "public-key".to_string(),
                            id: id.clone(),
                            transports: None,
                        })
                        .collect()
                }),
                authenticator_selection: Some(AuthenticatorSelectionCriteria {
                    authenticator_attachment,
                    resident_key,
                    require_resident_key,
                    user_verification: policy,
                }),
                extensions: extensions.clone(),
                attestation_formats,
            },
        };

        let wr = RegistrationState {
            policy,
            exclude_credentials: exclude_credentials.unwrap_or_else(|| Vec::with_capacity(0)),
            challenge,
            credential_algorithms,
             
            require_resident_key,
            authenticator_attachment,
            extensions: extensions.unwrap_or_default(),
            allow_synchronised_authenticators: !reject_synchronised_authenticators,
        };

         
        Ok((c, wr))
    }

     
     
     
     
     
     
     
     
     
     
     
    pub fn register_credential(
        &self,
        reg: &RegisterPublicKeyCredential,
        state: &RegistrationState,
        attestation_cas: Option<&AttestationCaList>,
         
    ) -> Result<Credential, WebauthnError> {
         
        trace!(?state);
        trace!(?reg);

        let RegistrationState {
            policy,
            exclude_credentials,
            challenge,
            credential_algorithms,
            require_resident_key: _,
            authenticator_attachment: _,
            extensions,
            allow_synchronised_authenticators,
        } = state;
        let chal: ChallengeRef = challenge;
        let current_time = SystemTime::now();

         
        let credential = self.register_credential_internal(
            reg,
            *policy,
            chal,
            exclude_credentials,
            credential_algorithms,
            attestation_cas,
            extensions,
            *allow_synchronised_authenticators,
            current_time,
        )?;

         
         
         
         

        








        Ok(credential)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register_credential_internal(
        &self,
        reg: &RegisterPublicKeyCredential,
        policy: UserVerificationPolicy,
        chal: ChallengeRef,
        exclude_credentials: &[CredentialID],
        credential_algorithms: &[COSEAlgorithm],
        attestation_cas: Option<&AttestationCaList>,
        req_extn: &RequestRegistrationExtensions,
        allow_synchronised_authenticators: bool,
        current_time: SystemTime,
    ) -> Result<Credential, WebauthnError> {
         
        if attestation_cas
            .as_ref()
            .map(|l| l.is_empty())
            .unwrap_or(false)
        {
            return Err(WebauthnError::MissingAttestationCaList);
        }

         
         
         

         

         
         
         
         
         
         
         
         
         

        let data = AuthenticatorAttestationResponse::try_from(&reg.response)?;

         

         
        if data.client_data_json.type_ != "webauthn.create" {
            return Err(WebauthnError::InvalidClientDataType);
        }

         
         
        if data.client_data_json.challenge.as_slice() != chal {
            return Err(WebauthnError::MismatchedChallenge);
        }

         
        if !self.allowed_origins.iter().any(|origin| {
            origins_match(
                self.allow_subdomains_origin,
                self.allow_any_port,
                &data.client_data_json.origin,
                origin,
            )
        }) {
            return Err(WebauthnError::InvalidRPOrigin);
        }

         
         
         
        if !self.allow_cross_origin && data.client_data_json.cross_origin.unwrap_or(false) {
            return Err(WebauthnError::CredentialCrossOrigin);
        }

         
         
         
         
         
         
         
         
         

         
         
         
         
         
         
        let client_data_json_hash = compute_sha256(data.client_data_json_bytes.as_slice());

         
         
         
         
         

         
         
         
         
         
        if data.attestation_object.auth_data.rp_id_hash != self.rp_id_hash {
            return Err(WebauthnError::InvalidRPIDHash);
        }

         
        if !data.attestation_object.auth_data.user_present {
            return Err(WebauthnError::UserNotPresent);
        }

         
         

         
         
        if matches!(policy, UserVerificationPolicy::Required)
            && !data.attestation_object.auth_data.user_verified
        {
            return Err(WebauthnError::UserNotVerified);
        }

         
         
         
         
         

         
         
         
         
         
         
         
         

        debug!(
            "extensions: {:?}",
            data.attestation_object.auth_data.extensions
        );

         

         
         
         
         
         
         
         
         
         
         
         
         
         
         
        let attest_format = AttestationFormat::try_from(data.attestation_object.fmt.as_str())
            .map_err(|()| WebauthnError::AttestationNotSupported)?;

         
         
         

        let acd = data
            .attestation_object
            .auth_data
            .acd
            .as_ref()
            .ok_or(WebauthnError::MissingAttestationCredentialData)?;

         
        debug!("attestation is: {:?}", &attest_format);
        debug!("attested credential data is: {:?}", &acd);

        let (attestation_data, attestation_metadata) = match attest_format {
            AttestationFormat::FIDOU2F => (
                verify_fidou2f_attestation(acd, &data.attestation_object, &client_data_json_hash)?,
                AttestationMetadata::None,
            ),
            AttestationFormat::Packed => {
                verify_packed_attestation(acd, &data.attestation_object, &client_data_json_hash)?
            }
             
            AttestationFormat::Tpm => {
                verify_tpm_attestation(acd, &data.attestation_object, &client_data_json_hash)?
            }
             
            AttestationFormat::AppleAnonymous => verify_apple_anonymous_attestation(
                acd,
                &data.attestation_object,
                &client_data_json_hash,
            )?,
             
            AttestationFormat::AndroidKey => verify_android_key_attestation(
                acd,
                &data.attestation_object,
                &client_data_json_hash,
            )?,
            AttestationFormat::AndroidSafetyNet => {
                return Err(WebauthnError::AttestationNotSupported)
            }
            AttestationFormat::None => (ParsedAttestationData::None, AttestationMetadata::None),
        };

        let credential: Credential = Credential::new(
            acd,
            &data.attestation_object.auth_data,
            COSEKey::try_from(&acd.credential_pk)?,
            policy,
            ParsedAttestation {
                data: attestation_data,
                metadata: attestation_metadata,
            },
            req_extn,
            &reg.extensions,
            attest_format,
            &data.transports,
        );

         

         
         
         
         
         
         
         
         
         
         
         
         
         
         

         
         

        let attested_ca_crt = if let Some(ca_list) = attestation_cas {
             
            let ca_crt =
                verify_attestation_ca_chain(&credential.attestation.data, ca_list, current_time)?;

             
             
             
             
            let ca_crt = ca_crt.ok_or_else(|| {
                warn!("device attested with a certificate not present in attestation ca chain");
                WebauthnError::AttestationNotVerifiable
            })?;
            Some(ca_crt)
        } else {
            None
        };

        debug!("attested_ca_crt = {:?}", attested_ca_crt);

         
         
        if let Some(att_ca_crt) = attested_ca_crt {
            if att_ca_crt.blanket_allow() {
                trace!("CA allows all associated keys.");
            } else {
                match &credential.attestation.metadata {
                    AttestationMetadata::Packed { aaguid }
                    | AttestationMetadata::Tpm { aaguid, .. } => {
                         
                        if !att_ca_crt.aaguids().contains_key(aaguid) {
                            return Err(WebauthnError::AttestationUntrustedAaguid);
                        }
                    }
                    _ => {
                         
                        return Err(WebauthnError::AttestationFormatMissingAaguid);
                    }
                }
            }
        };

         
        let alg_valid = credential_algorithms
            .iter()
            .any(|alg| alg == &credential.cred.type_);

        if !alg_valid {
            error!(
                "Authenticator ignored requested algorithm set - {:?} - {:?}",
                credential.cred.type_, credential_algorithms
            );
            return Err(WebauthnError::CredentialAlteredAlgFromRequest);
        }

         
        if !allow_synchronised_authenticators && credential.backup_eligible {
            error!("Credential counter is 0 - may indicate that it is a passkey and not bound to hardware.");
            return Err(WebauthnError::CredentialMayNotBeHardwareBound);
        }

         
         
        if credential.backup_state && !credential.backup_eligible {
            error!("Credential indicates it is backed up, but has not declared valid backup eligibility");
            return Err(WebauthnError::CredentialMayNotBeHardwareBound);
        }

         
        let excluded = exclude_credentials
            .iter()
            .any(|credid| credid.as_slice() == credential.cred_id.as_slice());

        if excluded {
            return Err(WebauthnError::CredentialAlteredAlgFromRequest);
        }

         
         
         
         
         

         
         
         
         
         
         
         

        Ok(credential)
    }

     
    pub(crate) fn verify_credential_internal(
        &self,
        rsp: &PublicKeyCredential,
        policy: UserVerificationPolicy,
        chal: ChallengeRef<'_>,
        cred: &Credential,
        appid: &Option<String>,
        allow_backup_eligible_upgrade: bool,
    ) -> Result<AuthenticatorData<Authentication>, WebauthnError> {
         

         
         
         
         
        let data = AuthenticatorAssertionResponse::try_from(&rsp.response).map_err(|e| {
            debug!("AuthenticatorAssertionResponse::try_from -> {:?}", e);
            e
        })?;

        let c = &data.client_data;

        






         
        if c.type_ != "webauthn.get" {
            return Err(WebauthnError::InvalidClientDataType);
        }

         
         
        if c.challenge.as_slice() != chal {
            return Err(WebauthnError::MismatchedChallenge);
        }

         
        if !self.allowed_origins.iter().any(|origin| {
            origins_match(
                self.allow_subdomains_origin,
                self.allow_any_port,
                &c.origin,
                origin,
            )
        }) {
            return Err(WebauthnError::InvalidRPOrigin);
        }

         
         
         
         

         
         
         
        let has_appid_enabled = rsp.extensions.appid.unwrap_or(false);

        let appid_hash = if has_appid_enabled {
            appid.as_ref().map(|id| compute_sha256(id.as_bytes()))
        } else {
            None
        };

        if !(data.authenticator_data.rp_id_hash == self.rp_id_hash
            || Some(&data.authenticator_data.rp_id_hash) == appid_hash.as_ref())
        {
            return Err(WebauthnError::InvalidRPIDHash);
        }

         
        if !data.authenticator_data.user_present {
            return Err(WebauthnError::UserNotPresent);
        }

         
         
         
         

        match (&policy, &cred.registration_policy) {
             
            (_, UserVerificationPolicy::Required) | (UserVerificationPolicy::Required, _)
                if !data.authenticator_data.user_verified =>
            {
                return Err(WebauthnError::UserNotVerified);
            }

             
             
            (_, UserVerificationPolicy::Preferred)
                if cred.user_verified && !data.authenticator_data.user_verified =>
            {
                debug!("Token registered UV=preferred, enforcing UV policy.");
                return Err(WebauthnError::UserNotVerified);
            }

             
             
             
            _ => {}
        }

         
         
         
         

        if cred.backup_eligible != data.authenticator_data.backup_eligible {
            if allow_backup_eligible_upgrade
                && !cred.backup_eligible
                && data.authenticator_data.backup_eligible
            {
                debug!("Credential backup eligibility has changed!");
            } else {
                error!("Credential backup eligibility has changed!");
                return Err(WebauthnError::CredentialBackupEligibilityInconsistent);
            }
        }

         
         
        if data.authenticator_data.backup_state && !cred.backup_eligible {
            error!("Credential indicates it is backed up, but has not declared valid backup eligibility");
            return Err(WebauthnError::CredentialMayNotBeHardwareBound);
        }

         
         
         
         
         
         
         
         
         
         
         
         
        debug!("extensions: {:?}", data.authenticator_data.extensions);

         
        let client_data_json_hash = compute_sha256(data.client_data_bytes.as_slice());

         
         
         
         

        let verification_data: Vec<u8> = data
            .authenticator_data_bytes
            .iter()
            .chain(client_data_json_hash.iter())
            .copied()
            .collect();

        let verified = cred
            .cred
            .verify_signature(&data.signature, &verification_data)?;

        if !verified {
            return Err(WebauthnError::AuthenticationFailure);
        }

        Ok(data.authenticator_data)
    }

     
     
     
     
     
     
    pub fn new_challenge_authenticate_builder(
        &self,
        creds: Vec<Credential>,
        policy: Option<UserVerificationPolicy>,
    ) -> Result<ChallengeAuthenticateBuilder, WebauthnError> {
        let policy = if let Some(policy) = policy {
            policy
        } else {
            let policy = creds
                .first()
                .map(|cred| cred.registration_policy.to_owned())
                .ok_or(WebauthnError::CredentialNotFound)?;

            for cred in creds.iter() {
                if cred.registration_policy != policy {
                    return Err(WebauthnError::InconsistentUserVerificationPolicy);
                }
            }

            policy
        };

        Ok(ChallengeAuthenticateBuilder {
            creds,
            policy,
            extensions: Default::default(),
            allow_backup_eligible_upgrade: Default::default(),
            hints: Default::default(),
        })
    }

     
     
     
     
     
     
     
     
     
     
    pub fn generate_challenge_authenticate(
        &self,
        challenge_builder: ChallengeAuthenticateBuilder,
    ) -> Result<(RequestChallengeResponse, AuthenticationState), WebauthnError> {
        let ChallengeAuthenticateBuilder {
            creds,
            policy,
            extensions,
            allow_backup_eligible_upgrade,
            hints,
        } = challenge_builder;

        let chal = self.generate_challenge();

         
        let ac = creds
            .iter()
            .map(|cred| AllowCredentials {
                type_: "public-key".to_string(),
                id: cred.cred_id.clone(),
                transports: cred.transports.clone(),
            })
            .collect();

         
        let appid = extensions.as_ref().and_then(|e| e.appid.clone());

        let timeout_millis =
            u32::try_from(self.authenticator_timeout.as_millis()).expect("Timeout too large");

         
         
        let r = RequestChallengeResponse {
            public_key: PublicKeyCredentialRequestOptions {
                challenge: chal.to_owned(),
                timeout: Some(timeout_millis),
                rp_id: self.rp_id.clone(),
                allow_credentials: ac,
                user_verification: policy,
                extensions,
                hints,
            },
            mediation: None,
        };
        let st = AuthenticationState {
            credentials: creds,
            policy,
            challenge: chal,
            appid,
            allow_backup_eligible_upgrade,
        };
        Ok((r, st))
    }

     
     
     
     
     
     
     
     
     
     
     
     
     
     
     
    pub fn authenticate_credential(
        &self,
        rsp: &PublicKeyCredential,
        state: &AuthenticationState,
    ) -> Result<AuthenticationResult, WebauthnError> {
         

         

         
         
         
         
         
         

        let cred = {
             
             
             
             
             
             
             
             

             
             
             
             
             
             

             
             
            let mut found_cred: Option<&Credential> = None;
            for cred in &state.credentials {
                if cred.cred_id.as_slice() == rsp.raw_id.as_slice() {
                    found_cred = Some(cred);
                    break;
                }
            }

            found_cred.ok_or(WebauthnError::CredentialNotFound)?
        };
        self.authenticate_single_credential(rsp, state, cred)
    }

     
     
     
     
     
     
     
     
     
     
     
     
     
     
    pub fn authenticate_single_credential(
        &self,
        rsp: &PublicKeyCredential,
        state: &AuthenticationState,
        cred: &Credential,
    ) -> Result<AuthenticationResult, WebauthnError> {
         
        let AuthenticationState {
            credentials: _,
            policy,
            challenge: chal,
            appid,
            allow_backup_eligible_upgrade,
        } = state;

        if cred.cred_id.as_slice() != rsp.raw_id.as_slice() {
            return Err(WebauthnError::CredentialNotFound);
        }

        let chal: ChallengeRef = chal;

         
         

         
         
         
         
         

         
         
         

         
         
         

         
         
         
         
         
         

        let auth_data = self.verify_credential_internal(
            rsp,
            *policy,
            chal,
            cred,
            appid,
            *allow_backup_eligible_upgrade,
        )?;
        let mut needs_update = false;
        let counter = auth_data.counter;
        let user_verified = auth_data.user_verified;
        let backup_state = auth_data.backup_state;
        let backup_eligible = auth_data.backup_eligible;

        let extensions = process_authentication_extensions(&auth_data.extensions);

        if backup_state != cred.backup_state {
            needs_update = true;
        }

        if backup_eligible != cred.backup_eligible {
            needs_update = true;
        }

         
         
        if counter > 0 || cred.counter > 0 {
             
             
             
             
             
             
             
             
             
            let counter_shows_compromise = auth_data.counter <= cred.counter;

            if counter > cred.counter {
                needs_update = true;
            }

            if self.require_valid_counter_value && counter_shows_compromise {
                return Err(WebauthnError::CredentialPossibleCompromise);
            }
        }

        Ok(AuthenticationResult {
            cred_id: cred.cred_id.clone(),
            needs_update,
            user_verified,
            backup_eligible,
            backup_state,
            counter,
            extensions,
        })
    }

     
     
     
    #[deprecated(note = "Use webauthn_rs_proto::origin::origins_match")]
    pub fn origins_match(
        allow_subdomains_origin: bool,
        allow_any_port: bool,
        request_origin: &Url,
        allowed_origin: &Url,
    ) -> bool {
        origins_match(
            allow_subdomains_origin,
            allow_any_port,
            request_origin,
            allowed_origin,
        )
    }

     
    pub fn rp_name(&self) -> &str {
        self.rp_name.as_str()
    }
}

