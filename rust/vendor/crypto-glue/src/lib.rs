#![deny(warnings)]
#![allow(dead_code)]
#![warn(unused_extern_crates)]
 
#![deny(clippy::suspicious)]
#![deny(clippy::perf)]
 
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
#![deny(clippy::await_holding_lock)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::trivially_copy_pass_by_ref)]
#![deny(clippy::disallowed_types)]
#![deny(clippy::manual_let_else)]
#![allow(clippy::unreachable)]

pub use argon2;
pub use cipher::block_padding;
pub use der;
pub use hex;
pub use pbkdf2;
pub use rand;
pub use spki;
pub use zeroize;

pub mod prelude {}

#[cfg(test)]
mod test_ca;

pub mod traits {
    pub use aes_gcm::aead::AeadInOut;
    pub use crypto_common::{Generate, KeyInit, OutputSizeUser};
    pub use der::{
        Decode as DecodeDer, DecodePem, Encode as EncodeDer, EncodePem,
        pem::LineEnding as LineEndingPem, referenced::OwnedToRef,
    };
    pub use digest::FixedOutput;
    pub use elliptic_curve::sec1::{FromSec1Point, ToSec1Point};
    pub use hmac::{Hmac, Mac};
    pub use pkcs8::{
        DecodePrivateKey as Pkcs8DecodePrivateKey, EncodePrivateKey as Pkcs8EncodePrivateKey,
    };
    pub use signature::{
        DigestSigner, DigestVerifier, Keypair, RandomizedSigner, SignatureEncoding, Signer,
        Verifier,
    };
    pub use sha2::Digest;
    pub use spki::{
        DecodePublicKey as SpkiDecodePublicKey, DynSignatureAlgorithmIdentifier,
        EncodePublicKey as SpkiEncodePublicKey,
    };
    pub use zeroize::Zeroizing;
    pub mod hazmat {
         

        pub use signature::hazmat::PrehashVerifier;
    }
    pub use x509_cert::ext::ToExtension;
}

pub mod x509;

pub mod md5 {
    pub use md5::*;
}

pub mod sha1 {
    use hybrid_array::{Array, sizes::U20};

    pub use sha1::Sha1;

    pub type Sha1Output = Array<u8, U20>;
}

pub mod s256 {
    use hybrid_array::{Array, sizes::U32};

    pub use sha2::Sha256;

    pub type Sha256Output = Array<u8, U32>;
}

pub mod s384 {
    use hybrid_array::{Array, sizes::U48};

    pub use sha2::Sha384;

    pub type Sha384Output = Array<u8, U48>;
}

pub mod s512 {
    use hybrid_array::{Array, sizes::U64};

    pub use sha2::Sha512;

    pub type Sha512Output = Array<u8, U64>;
}

pub mod hkdf_s256 {
    use hkdf::Hkdf;
    use sha2::Sha256;

    pub type HkdfSha256 = Hkdf<Sha256>;
}

pub mod hmac_s1 {
    use crypto_common::Key;
    use crypto_common::Output;

    use hmac::Hmac;
    use hmac::Mac;
    use sha1::Sha1;
    use sha1::digest::CtOutput;
    use zeroize::Zeroizing;

    pub type HmacSha1 = Hmac<Sha1>;

    pub type HmacSha1Key = Zeroizing<Key<Hmac<Sha1>>>;

    pub type HmacSha1Output = CtOutput<HmacSha1>;

    pub type HmacSha1Bytes = Output<HmacSha1>;

    pub fn new_key() -> HmacSha1Key {
        use crypto_common::Generate;
        Key::<HmacSha1>::generate().into()
    }

    pub fn oneshot(key: &HmacSha1Key, data: &[u8]) -> HmacSha1Output {
        use crypto_common::KeyInit;

        let mut hmac = HmacSha1::new(key);
        hmac.update(data);
        hmac.finalize()
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn key_from_vec(bytes: Vec<u8>) -> Option<HmacSha1Key> {
        key_from_slice(&bytes)
    }

    pub fn key_from_slice(bytes: &[u8]) -> Option<HmacSha1Key> {
        use crypto_common::KeySizeUser;
         
        if bytes.len() < 16 || bytes.len() > Hmac::<Sha1>::key_size() {
            None
        } else {
            let mut key = Key::<Hmac<Sha1>>::default();
            let key_ref = &mut key.as_mut_slice()[..bytes.len()];
            key_ref.copy_from_slice(bytes);
            Some(key.into())
        }
    }

    pub fn key_from_bytes(bytes: [u8; 64]) -> HmacSha1Key {
        Key::<Hmac<Sha1>>::from(bytes).into()
    }

    pub fn key_size() -> usize {
        use crypto_common::KeySizeUser;
        Hmac::<Sha1>::key_size()
    }
}

pub mod hmac_s256 {
    use crypto_common::Key;
    use crypto_common::Output;

    use hmac::Hmac;
    use hmac::Mac;
    use sha2::Sha256;
    use sha2::digest::CtOutput;
    use zeroize::Zeroizing;

    pub type HmacSha256 = Hmac<Sha256>;

    pub type HmacSha256Key = Zeroizing<Key<Hmac<Sha256>>>;

    pub type HmacSha256Output = CtOutput<HmacSha256>;

    pub type HmacSha256Bytes = Output<HmacSha256>;

    pub fn new_key() -> HmacSha256Key {
        use crypto_common::Generate;
        Key::<HmacSha256>::generate().into()
    }

    pub fn oneshot(key: &HmacSha256Key, data: &[u8]) -> HmacSha256Output {
        use crypto_common::KeyInit;

        let mut hmac = HmacSha256::new(key);
        hmac.update(data);
        hmac.finalize()
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn key_from_vec(bytes: Vec<u8>) -> Option<HmacSha256Key> {
        key_from_slice(&bytes)
    }

    pub fn key_from_slice(bytes: &[u8]) -> Option<HmacSha256Key> {
        use crypto_common::KeySizeUser;
         
        if bytes.len() < 16 || bytes.len() > Hmac::<Sha256>::key_size() {
            None
        } else {
            let mut key = Key::<Hmac<Sha256>>::default();
            let key_ref = &mut key.as_mut_slice()[..bytes.len()];
            key_ref.copy_from_slice(bytes);
            Some(key.into())
        }
    }

    pub fn key_from_bytes(bytes: [u8; 64]) -> HmacSha256Key {
        Key::<Hmac<Sha256>>::from(bytes).into()
    }

    pub fn key_size() -> usize {
        use crypto_common::KeySizeUser;
        Hmac::<Sha256>::key_size()
    }
}

pub mod hmac_s512 {
    use crypto_common::Key;
    use crypto_common::Output;

    use hmac::Hmac;
    use sha2::Sha512;
    use sha2::digest::CtOutput;
    use zeroize::Zeroizing;

    pub use hmac::Mac;

    pub type HmacSha512 = Hmac<Sha512>;

    pub type HmacSha512Key = Zeroizing<Key<Hmac<Sha512>>>;

    pub type HmacSha512Output = CtOutput<HmacSha512>;

    pub type HmacSha512Bytes = Output<HmacSha512>;

    pub fn new_hmac_sha512_key() -> HmacSha512Key {
        use crypto_common::Generate;
        Key::<HmacSha512>::generate().into()
    }

    pub fn oneshot(key: &HmacSha512Key, data: &[u8]) -> HmacSha512Output {
        use crypto_common::KeyInit;

        let mut hmac = HmacSha512::new(key);
        hmac.update(data);
        hmac.finalize()
    }

    pub fn key_from_slice(bytes: &[u8]) -> Option<HmacSha512Key> {
        use crypto_common::KeySizeUser;
         
        if bytes.len() < 16 || bytes.len() > Hmac::<Sha512>::key_size() {
            None
        } else {
            let mut key = Key::<Hmac<Sha512>>::default();
            let key_ref = &mut key.as_mut_slice()[..bytes.len()];
            key_ref.copy_from_slice(bytes);
            Some(key.into())
        }
    }

    pub fn key_size() -> usize {
        use crypto_common::KeySizeUser;
        Hmac::<Sha512>::key_size()
    }
}

pub mod aes128 {
    use aes;
    use crypto_common::Key;
    use zeroize::Zeroizing;

    pub type Aes128Key = Zeroizing<Key<aes::Aes128>>;

    pub fn key_size() -> usize {
        use crypto_common::KeySizeUser;
        aes::Aes128::key_size()
    }

    pub fn key_from_slice(bytes: &[u8]) -> Option<Aes128Key> {
        Key::<aes::Aes128>::try_from(bytes)
            .ok()
            .map(|key| key.into())
    }

    pub fn key_from_bytes(bytes: [u8; 16]) -> Aes128Key {
        Key::<aes::Aes128>::from(bytes).into()
    }

    pub fn new_key() -> Aes128Key {
        use crypto_common::Generate;
        Key::<aes::Aes128>::generate().into()
    }
}

pub mod aes128gcm {
    use aes::cipher::consts::{U12, U16};

    pub use aes_gcm::aead::{Aead, AeadInOut, Payload};
    pub use crypto_common::KeyInit;

    pub use crate::aes128::Aes128Key;

    pub type Aes128Gcm = aes_gcm::Aes128Gcm;

    pub type Aes128GcmNonce = aes_gcm::Nonce<U12>;
    pub type Aes128GcmTag = aes_gcm::Tag<U16>;

    pub fn new_nonce() -> Aes128GcmNonce {
        use crypto_common::Generate;
        Aes128GcmNonce::generate()
    }
}

pub mod aes128kw {
    use hybrid_array::{Array, sizes::U24};

    pub use crypto_common::KeyInit;

    pub type Aes128Kw = aes_kw::KwAes128;

    pub type Aes128KwWrapped = Array<u8, U24>;
}

pub mod aes256 {
    use aes;
    use aes::cipher::Array;
    use crypto_common::Key;
    use zeroize::Zeroizing;

    pub use aes::Aes256;
    pub use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt};

    pub type Aes256Key = Zeroizing<Key<aes::Aes256>>;
    pub type Aes256BlockSize = <aes::Aes256 as aes::cipher::BlockSizeUser>::BlockSize;
    pub type Aes256Block = Array<u8, <aes::Aes256 as aes::cipher::BlockSizeUser>::BlockSize>;

    pub fn key_size() -> usize {
        use crypto_common::KeySizeUser;
        aes::Aes256::key_size()
    }

    pub fn key_from_slice(bytes: &[u8]) -> Option<Aes256Key> {
        Key::<aes::Aes256>::try_from(bytes)
            .ok()
            .map(|key| key.into())
    }

    pub fn key_from_bytes(bytes: [u8; 32]) -> Aes256Key {
        Key::<aes::Aes256>::from(bytes).into()
    }

    pub fn new_key() -> Aes256Key {
        use crypto_common::Generate;
        Key::<aes::Aes256>::generate().into()
    }
}

pub mod aes256gcm {
    use aes::Aes256;
    use aes::cipher::consts::{U12, U16};
    use aes_gcm::AesGcm;

    pub use aes_gcm::aead::{Aead, AeadInOut, Payload};
    pub use crypto_common::KeyInit;

    pub use crate::aes256::Aes256Key;

     
    pub type Aes256Gcm = aes_gcm::Aes256Gcm;

    pub type Aes256GcmN16 = AesGcm<Aes256, U16, U16>;
    pub type Aes256GcmNonce16 = aes_gcm::Nonce<U16>;

    pub type Aes256GcmNonce = aes_gcm::Nonce<U12>;
    pub type Aes256GcmTag = aes_gcm::Tag<U16>;

    pub fn new_nonce() -> Aes256GcmNonce {
        use crypto_common::Generate;
        Aes256GcmNonce::generate()
    }
}

pub mod aes256cts {
    use aes::cipher::Array;
    use aes::cipher::consts::U16;

    pub use crate::aes256::Aes256Key;
    pub use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, InnerIvInit, KeyIvInit};

    pub use cts::Decrypt as CtsDecrypt;
    pub use cts::Encrypt as CtsEncrypt;

    pub type Aes256CtsEnc = cts::CbcCs3<aes::Aes256>;
    pub type Aes256CtsDec = cts::CbcCs3<aes::Aes256>;

    pub type Aes256CtsIv = Array<u8, U16>;

    pub fn new_iv() -> Aes256CtsIv {
        use crypto_common::Generate;
        Aes256CtsIv::generate()
    }
}

pub mod aes256cbc {
    use crate::hmac_s256::HmacSha256;
    use crate::hmac_s256::HmacSha256Output;
    use aes::cipher::Array;
    use aes::cipher::consts::U16;

    pub use crate::aes256::Aes256Key;

    pub use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit, block_padding};

    pub type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
    pub type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

    pub type Aes256CbcIv = Array<u8, U16>;

    pub fn new_iv() -> Aes256CbcIv {
        use crypto_common::Generate;
        Aes256CbcIv::generate()
    }

    pub fn enc<P>(
        key: &Aes256Key,
        data: &[u8],
    ) -> Result<(HmacSha256Output, Aes256CbcIv, Vec<u8>), crypto_common::InvalidLength>
    where
        P: block_padding::Padding,
    {
        use cipher::BlockModeEncrypt;
        use cipher::KeyInit;
        use hmac::Mac;

        let iv = new_iv();
        let enc = Aes256CbcEnc::new(key, &iv);

        let ciphertext = enc.encrypt_padded_vec::<P>(data);

        let mut hmac = HmacSha256::new_from_slice(key.as_slice())?;
        hmac.update(&ciphertext);
        let mac = hmac.finalize();

        Ok((mac, iv, ciphertext))
    }

    pub fn dec<P>(
        key: &Aes256Key,
        mac: &HmacSha256Output,
        iv: &Aes256CbcIv,
        ciphertext: &[u8],
    ) -> Option<Vec<u8>>
    where
        P: block_padding::Padding,
    {
        use cipher::BlockModeDecrypt;
        use cipher::KeyInit;
        use hmac::Mac;

        let mut hmac = HmacSha256::new_from_slice(key.as_slice()).ok()?;
        hmac.update(ciphertext);
        let check_mac = hmac.finalize();

        if check_mac != *mac {
            return None;
        }

        let dec = Aes256CbcDec::new(key, iv);

        let plaintext = dec.decrypt_padded_vec::<P>(ciphertext).ok()?;

        Some(plaintext)
    }
}

pub mod aes256kw {
    use hybrid_array::{Array, sizes::U40};

    pub use crypto_common::KeyInit;

    pub type Aes256Kw = aes_kw::KwAes256;

    pub type Aes256KwWrapped = Array<u8, U40>;
}



pub mod ec {
    pub use sec1::EcPrivateKey;
}

pub mod ecdh {
    pub use elliptic_curve::ecdh::diffie_hellman;
}

pub mod ecdh_p256 {
    use elliptic_curve::ecdh::{EphemeralSecret, SharedSecret};
    use elliptic_curve::sec1::Sec1Point;
    use elliptic_curve::{FieldBytes, PublicKey};
    use hkdf::Hkdf;
    use p256::NistP256;
    use sha2::Sha256;

    pub type EcdhP256EphemeralSecret = EphemeralSecret<NistP256>;
    pub type EcdhP256SharedSecret = SharedSecret<NistP256>;
    pub type EcdhP256PublicKey = PublicKey<NistP256>;
    pub type EcdhP256PublicSec1Point = Sec1Point<NistP256>;
    pub type EcdhP256FieldBytes = FieldBytes<NistP256>;

    pub type EcdhP256Hkdf = Hkdf<Sha256>;

    pub type EcdhP256Digest = Sha256;

    pub fn new_secret() -> EcdhP256EphemeralSecret {
        use crypto_common::Generate;
        EcdhP256EphemeralSecret::generate()
    }
}

pub mod ecdsa_p256 {
    use ecdsa::DigestAlgorithm;
    use ecdsa::{Signature, SignatureBytes, SigningKey, VerifyingKey};
    use elliptic_curve::point::AffinePoint;
    use elliptic_curve::scalar::NonZeroScalar;
    use elliptic_curve::sec1::FromSec1Point;
    use elliptic_curve::sec1::Sec1Point;
    use elliptic_curve::{FieldBytes, PublicKey, SecretKey};
    use hybrid_array::{Array, sizes::U32};
    use p256::{NistP256, ecdsa::DerSignature};

    pub type EcdsaP256Digest = <NistP256 as DigestAlgorithm>::Digest;

    pub type EcdsaP256PrivateKey = SecretKey<NistP256>;
    pub type EcdsaP256NonZeroScalar = NonZeroScalar<NistP256>;

    pub type EcdsaP256FieldBytes = FieldBytes<NistP256>;
    pub type EcdsaP256AffinePoint = AffinePoint<NistP256>;

    pub type EcdsaP256PublicKey = PublicKey<NistP256>;

    pub type EcdsaP256PublicCoordinate = Array<u8, U32>;
    pub type EcdsaP256PublicSec1Point = Sec1Point<NistP256>;

    pub type EcdsaP256SigningKey = SigningKey<NistP256>;
    pub type EcdsaP256VerifyingKey = VerifyingKey<NistP256>;

    pub type EcdsaP256Signature = Signature<NistP256>;
    pub type EcdsaP256DerSignature = DerSignature;
    pub type EcdsaP256SignatureBytes = SignatureBytes<NistP256>;

    pub fn new_key() -> EcdsaP256PrivateKey {
        use crypto_common::Generate;

        EcdsaP256PrivateKey::generate()
    }

    pub fn from_coords_raw(x: &[u8], y: &[u8]) -> Option<EcdsaP256PublicKey> {
        let mut field_x = EcdsaP256FieldBytes::default();
        if x.len() != field_x.len() {
            return None;
        }

        let mut field_y = EcdsaP256FieldBytes::default();
        if y.len() != field_y.len() {
            return None;
        }

        field_x.copy_from_slice(x);
        field_y.copy_from_slice(y);

        let ep = EcdsaP256PublicSec1Point::from_affine_coordinates(&field_x, &field_y, false);

        EcdsaP256PublicKey::from_sec1_point(&ep).into_option()
    }
}

pub mod ecdsa_p384 {
    use ecdsa::DigestAlgorithm;
    use ecdsa::{Signature, SignatureBytes, SigningKey, VerifyingKey};
    use elliptic_curve::point::AffinePoint;
    use elliptic_curve::sec1::FromSec1Point;
    use elliptic_curve::sec1::Sec1Point;
    use elliptic_curve::{FieldBytes, PublicKey, SecretKey};
    use p384::{NistP384, ecdsa::DerSignature};
     

    pub type EcdsaP384Digest = <NistP384 as DigestAlgorithm>::Digest;

    pub type EcdsaP384PrivateKey = SecretKey<NistP384>;

    pub type EcdsaP384FieldBytes = FieldBytes<NistP384>;
    pub type EcdsaP384AffinePoint = AffinePoint<NistP384>;

    pub type EcdsaP384PublicKey = PublicKey<NistP384>;

     
    pub type EcdsaP384PublicSec1Point = Sec1Point<NistP384>;

    pub type EcdsaP384SigningKey = SigningKey<NistP384>;
    pub type EcdsaP384VerifyingKey = VerifyingKey<NistP384>;

    pub type EcdsaP384Signature = Signature<NistP384>;
    pub type EcdsaP384DerSignature = DerSignature;
    pub type EcdsaP384SignatureBytes = SignatureBytes<NistP384>;

    pub fn new_key() -> EcdsaP384PrivateKey {
        use crypto_common::Generate;
        EcdsaP384PrivateKey::generate()
    }

    pub fn from_coords_raw(x: &[u8], y: &[u8]) -> Option<EcdsaP384PublicKey> {
        let mut field_x = EcdsaP384FieldBytes::default();
        if x.len() != field_x.len() {
            return None;
        }

        let mut field_y = EcdsaP384FieldBytes::default();
        if y.len() != field_y.len() {
            return None;
        }

        field_x.copy_from_slice(x);
        field_y.copy_from_slice(y);

        let ep = EcdsaP384PublicSec1Point::from_affine_coordinates(&field_x, &field_y, false);

        EcdsaP384PublicKey::from_sec1_point(&ep).into_option()
    }
}

pub mod ecdsa_p521 {
    use ecdsa::DigestAlgorithm;
    use ecdsa::{Signature, SignatureBytes, SigningKey, VerifyingKey};
    use elliptic_curve::point::AffinePoint;
    use elliptic_curve::sec1::FromSec1Point;
    use elliptic_curve::sec1::Sec1Point;
    use elliptic_curve::{FieldBytes, PublicKey, SecretKey};
    use p521::{NistP521, ecdsa::DerSignature};

    pub type EcdsaP521Digest = <NistP521 as DigestAlgorithm>::Digest;

    pub type EcdsaP521PrivateKey = SecretKey<NistP521>;

    pub type EcdsaP521FieldBytes = FieldBytes<NistP521>;
    pub type EcdsaP521AffinePoint = AffinePoint<NistP521>;

    pub type EcdsaP521PublicKey = PublicKey<NistP521>;

     
    pub type EcdsaP521PublicSec1Point = Sec1Point<NistP521>;

    pub type EcdsaP521SigningKey = SigningKey<NistP521>;
    pub type EcdsaP521VerifyingKey = VerifyingKey<NistP521>;

    pub type EcdsaP521Signature = Signature<NistP521>;
    pub type EcdsaP521DerSignature = DerSignature;
    pub type EcdsaP521SignatureBytes = SignatureBytes<NistP521>;

    pub fn new_key() -> EcdsaP521PrivateKey {
        use crypto_common::Generate;
        EcdsaP521PrivateKey::generate()
    }

    pub fn from_coords_raw(x: &[u8], y: &[u8]) -> Option<EcdsaP521PublicKey> {
        let mut field_x = EcdsaP521FieldBytes::default();
        if x.len() != field_x.len() {
            return None;
        }

        let mut field_y = EcdsaP521FieldBytes::default();
        if y.len() != field_y.len() {
            return None;
        }

        field_x.copy_from_slice(x);
        field_y.copy_from_slice(y);

        let ep = EcdsaP521PublicSec1Point::from_affine_coordinates(&field_x, &field_y, false);

        EcdsaP521PublicKey::from_sec1_point(&ep).into_option()
    }
}

pub mod nist_sp800_108_kdf_hmac_sha256 {
    use crate::traits::Zeroizing;
    use crypto_common::KeySizeUser;
    use digest::consts::*;
    use hmac::Hmac;
    use kbkdf::{Counter, Kbkdf, Params};
    use sha2::Sha256;

    struct MockOutput;

    impl KeySizeUser for MockOutput {
        type KeySize = U32;
    }

    type HmacSha256 = Hmac<Sha256>;

    pub fn derive_key_aes256(
        key_in: &[u8],
        label: &[u8],
        context: &[u8],
    ) -> Option<Zeroizing<Vec<u8>>> {
        let counter = Counter::<HmacSha256, MockOutput>::default();
        let params = Params::builder(key_in)
            .with_label(label)
            .with_context(context)
            .use_l(true)
            .use_separator(true)
            .use_counter(true)
            .build();
        let key = counter.derive(params).ok()?;

        let mut output = Zeroizing::new(vec![0; MockOutput::key_size()]);
        output.copy_from_slice(key.as_slice());
        Some(output)
    }
}

pub mod pkcs8 {
    pub use pkcs8::PrivateKeyInfo;
}

