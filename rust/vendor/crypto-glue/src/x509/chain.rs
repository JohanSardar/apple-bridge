use crate::x509::{AlgorithmIdentifier, BasicConstraints, Certificate, KeyUsage, oiddb::rfc5912};
use crate::{
    ecdsa_p256::{EcdsaP256DerSignature, EcdsaP256PublicKey, EcdsaP256VerifyingKey},
    ecdsa_p384::{EcdsaP384DerSignature, EcdsaP384PublicKey, EcdsaP384VerifyingKey},
    s256::{Sha256, Sha256Output},
    traits::{Digest, Verifier, hazmat::PrehashVerifier},
};
use der::Encode;
use der::referenced::OwnedToRef;
use std::time::{Duration, SystemTime};
use tracing::{debug, error, trace, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X509VerificationError {
    InvalidSystemTime,
    BasicConstraintsNotPresent,
    LeafMustNotBeCA,
    KeyUsageNotValid,
    ExtensionFailure,
    NotBefore,
    NotAfter,
    NoMatchingIssuer,
    InvalidIssuer,
    ExcessivePathLength,
    CaNotMarkedAsSuch,
    PathLengthExceeded,
    SignatureAlgorithmMismatch,
    SignatureAlgorithmNotImplemented,
    DerSignatureInvalid,
    VerifyingKeyFromSpki,
    SignatureVerificationFailed,
    CertificateSerialisation,
    KeyUsageNotPresent,
    SubjectPublicKeyInformationInvalid,
}

pub struct X509Store {
    store: Vec<Certificate>,
    leaf_basic_constraints_required: bool,
}

impl X509Store {
    pub fn new(ca_roots: &[Certificate]) -> Self {
        Self {
            store: ca_roots.iter().map(|c| (*c).clone()).collect(),
            leaf_basic_constraints_required: true,
        }
    }

    pub fn leaf_basic_constraints_required(&mut self, enable: bool) {
        self.leaf_basic_constraints_required = enable;
    }

    pub fn verify(
        &self,
        leaf: &Certificate,
        intermediates: &[Certificate],
        current_time: SystemTime,
    ) -> Result<&Certificate, X509VerificationError> {
         
         
         
         
         
         
         
         
         
         
         
         
         

        let current_time_unix = current_time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| X509VerificationError::InvalidSystemTime)?;

        let mut certificate_to_validate = leaf;
        self.validate_leaf(certificate_to_validate, current_time_unix)?;

         
        let mut path_length = 0;

        for intermediate in intermediates {
             
             

            self.validate_pair(
                certificate_to_validate,
                intermediate,
                current_time_unix,
                path_length,
            )?;

             
            certificate_to_validate = intermediate;

             
            path_length = path_length
                .checked_add(1)
                .ok_or(X509VerificationError::ExcessivePathLength)?;
        }

         
         
         

         
         

        let authority_cert = self.locate_authority_certificate(certificate_to_validate)?;

        self.validate_pair(
            certificate_to_validate,
            authority_cert,
            current_time_unix,
            path_length,
        )?;

         
         

         
        Ok(authority_cert)
    }

    fn validate_leaf(
        &self,
        certificate_to_validate: &Certificate,
        current_time: Duration,
    ) -> Result<(), X509VerificationError> {
         
         
         
        let maybe_basic_constraints = certificate_to_validate
            .tbs_certificate()
            .get_extension::<BasicConstraints>()
            .map_err(|_err| X509VerificationError::ExtensionFailure)?;
         
         

        if let Some((_critical, basic_constraints)) = maybe_basic_constraints {
            if basic_constraints.ca {
                return Err(X509VerificationError::LeafMustNotBeCA);
            }
        } else if self.leaf_basic_constraints_required {
            return Err(X509VerificationError::BasicConstraintsNotPresent);
        };

        let maybe_keyusage = certificate_to_validate
            .tbs_certificate()
            .get_extension::<KeyUsage>()
            .map_err(|_err| X509VerificationError::ExtensionFailure)?;

        if let Some((_critical, key_usage)) = maybe_keyusage {
             
             
            if !key_usage.digital_signature() {
                return Err(X509VerificationError::KeyUsageNotValid);
            }
        }

         
        let not_before = certificate_to_validate
            .tbs_certificate()
            .validity()
            .not_before
            .to_unix_duration();

        if not_before > current_time {
            trace!(?not_before, ?current_time);
            return Err(X509VerificationError::NotBefore);
        }

        let not_after = certificate_to_validate
            .tbs_certificate()
            .validity()
            .not_after
            .to_unix_duration();

        if current_time > not_after {
            debug!(?current_time, ?not_after);
            return Err(X509VerificationError::NotAfter);
        }

        Ok(())
    }

    fn validate_pair(
        &self,
        certificate_to_validate: &Certificate,
        authority: &Certificate,
        current_time: Duration,
        path_length: u8,
    ) -> Result<(), X509VerificationError> {
        if authority.tbs_certificate().subject()
            != certificate_to_validate.tbs_certificate().issuer()
        {
            return Err(X509VerificationError::InvalidIssuer);
        }

         
         
         
         

        let (_critical, basic_constraints) = authority
            .tbs_certificate()
            .get_extension::<BasicConstraints>()
            .map_err(|_err| X509VerificationError::ExtensionFailure)?
             
            .ok_or(X509VerificationError::BasicConstraintsNotPresent)?;

        if !basic_constraints.ca {
            return Err(X509VerificationError::CaNotMarkedAsSuch);
        }

        if let Some(ca_pathlen) = basic_constraints.path_len_constraint {
             
             
            if path_length > ca_pathlen {
                return Err(X509VerificationError::PathLengthExceeded);
            }
        }

        let (_critical, key_usage) = authority
            .tbs_certificate()
            .get_extension::<KeyUsage>()
            .map_err(|_err| X509VerificationError::ExtensionFailure)?
            .ok_or(X509VerificationError::KeyUsageNotPresent)?;

        if !key_usage.key_cert_sign() {
            return Err(X509VerificationError::KeyUsageNotValid);
        }

         
        let not_before = authority
            .tbs_certificate()
            .validity()
            .not_before
            .to_unix_duration();

        if not_before > current_time {
            return Err(X509VerificationError::NotBefore);
        }

        let not_after = authority
            .tbs_certificate()
            .validity()
            .not_after
            .to_unix_duration();

        if current_time > not_after {
            return Err(X509VerificationError::NotAfter);
        }

         
         
         
         
        if certificate_to_validate.signature_algorithm() != authority.tbs_certificate().signature()
        {
            warn!(validate_signature = ?certificate_to_validate.signature_algorithm(), authority_signature = ?authority.tbs_certificate().signature());
             
        }

        let cert_to_validate_data = certificate_to_validate
            .tbs_certificate()
            .to_der()
            .map_err(|_err| X509VerificationError::CertificateSerialisation)?;

        let cert_to_validate_signature = certificate_to_validate
            .signature()
            .as_bytes()
            .ok_or(X509VerificationError::DerSignatureInvalid)?;

        verify_der_signature(
            &cert_to_validate_data,
            cert_to_validate_signature,
            certificate_to_validate.signature_algorithm(),
            authority,
        )?;

        Ok(())
    }

    fn locate_authority_certificate(
        &self,
        certificate_to_validate: &Certificate,
    ) -> Result<&Certificate, X509VerificationError> {
        self.store
            .iter()
            .find(|ca_cert| {
                ca_cert.tbs_certificate().subject()
                    == certificate_to_validate.tbs_certificate().issuer()
            })
            .ok_or(X509VerificationError::NoMatchingIssuer)
    }
}

 
 
 
fn verify_der_signature(
    data: &[u8],
    signature: &[u8],
    signature_algorithm: &AlgorithmIdentifier<der::Any>,
    certificate: &Certificate,
) -> Result<(), X509VerificationError> {
    let subject_public_key_info = certificate
        .tbs_certificate()
        .subject_public_key_info()
        .owned_to_ref();

    let (spki_alg_oid, spki_alg_params) = subject_public_key_info
        .algorithm
        .oids()
        .map_err(|_| X509VerificationError::SubjectPublicKeyInformationInvalid)?;

    trace!(?signature_algorithm.oid, ?spki_alg_oid, ?spki_alg_params);

    match (signature_algorithm.oid, spki_alg_oid, spki_alg_params) {
        (rfc5912::ECDSA_WITH_SHA_256, rfc5912::ID_EC_PUBLIC_KEY, Some(rfc5912::SECP_256_R_1)) => {
            let signature = EcdsaP256DerSignature::try_from(signature).map_err(|err| {
                error!(?err);
                X509VerificationError::DerSignatureInvalid
            })?;

            let verifier = EcdsaP256PublicKey::try_from(subject_public_key_info)
                .map(EcdsaP256VerifyingKey::from)
                .map_err(|_err| X509VerificationError::VerifyingKeyFromSpki)?;

            verifier
                .verify(data, &signature)
                .map_err(|_err| X509VerificationError::SignatureVerificationFailed)?;
        }
        (rfc5912::ECDSA_WITH_SHA_256, rfc5912::ID_EC_PUBLIC_KEY, Some(rfc5912::SECP_384_R_1)) => {
            let signature = EcdsaP384DerSignature::try_from(signature)
                .map_err(|_err| X509VerificationError::DerSignatureInvalid)?;

            let verifier = EcdsaP384PublicKey::try_from(subject_public_key_info)
                .map(EcdsaP384VerifyingKey::from)
                .map_err(|_err| X509VerificationError::VerifyingKeyFromSpki)?;

             
            let mut hasher = Sha256::new();
            hasher.update(data);
            let out: Sha256Output = hasher.finalize();

            verifier
                .verify_prehash(out.as_slice(), &signature)
                .map_err(|_err| X509VerificationError::SignatureVerificationFailed)?;
        }
        (rfc5912::ECDSA_WITH_SHA_384, rfc5912::ID_EC_PUBLIC_KEY, Some(rfc5912::SECP_384_R_1)) => {
            let signature = EcdsaP384DerSignature::try_from(signature)
                .map_err(|_err| X509VerificationError::DerSignatureInvalid)?;

            let verifier = EcdsaP384PublicKey::try_from(subject_public_key_info)
                .map(EcdsaP384VerifyingKey::from)
                .map_err(|_err| X509VerificationError::VerifyingKeyFromSpki)?;

            verifier
                .verify(data, &signature)
                .map_err(|_err| X509VerificationError::SignatureVerificationFailed)?;
        }


        (signature_algorithm_oid, spki_alg_oid, spki_alg_params) => {
            error!(?signature_algorithm_oid, ?spki_alg_oid, ?spki_alg_params);
            return Err(X509VerificationError::SignatureAlgorithmNotImplemented);
        }
    }

    Ok(())
}

