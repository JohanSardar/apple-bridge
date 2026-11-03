 

use crate::error::WebauthnError;
use crate::proto::CredentialID;
use crypto_glue::hmac_s256::{self, HmacSha256Key};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

 
 
pub trait FakeCredentialIDDistribution {
     
    fn generate<R: Rng>(seeded_rng: &mut R) -> Vec<CredentialID>;
}

 
 
const CREDENTIAL_MEDIAN: u8 = 1;

 

 
pub const YUBIKEY_CRED_LEN: usize = 64;
 
pub const YUBIKEY_EDDSA_CRED_LEN: usize = 128;
 
pub const TPM_CRED_LEN: usize = 32;
 
pub const G_PIXEL_CRED_LEN: usize = 65;
 
pub const APPLE_CRED_LEN: usize = 20;
 
pub const BITWARDEN_CRED_LEN: usize = 16;

 
 
 
 

 
 
const SD_02: u32 = 94489281;
const SD_15: u32 = 678604833;
 
const SD_84: u32 = 3612067496;
const SD_98: u32 = 4200478015;

 

 

 
 
 
 
 

const CRED_SINGLE_68: u32 = 2946347564;
const CRED_SINGLE_98: u32 = 4200478014;

 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 

const CRED_MULTI_ANDROID: u32 = 2173682947;
const CRED_MULTI_WINDOWS: u32 = CRED_MULTI_ANDROID + 372803161;
const CRED_MULTI_APL: u32 = CRED_MULTI_WINDOWS + 1428076625;
const CRED_MULTI_YK: u32 = CRED_MULTI_APL + 160202280;

 
 
 
pub struct FakePasskeyDistribution;

impl FakeCredentialIDDistribution for FakePasskeyDistribution {
     
    fn generate<R: Rng>(seeded_rng: &mut R) -> Vec<CredentialID> {
         
        let cred_dist = seeded_rng.next_u32();

        let creds_to_generate = if cred_dist < SD_02 {
             
            CREDENTIAL_MEDIAN.saturating_sub(2)
        } else if cred_dist < SD_15 {
             
            CREDENTIAL_MEDIAN.saturating_sub(1)
        } else if cred_dist < SD_84 {
             
            CREDENTIAL_MEDIAN
        } else if cred_dist < SD_98 {
             
            CREDENTIAL_MEDIAN.saturating_add(1)
        } else {
             
            CREDENTIAL_MEDIAN.saturating_add(2)
        };

        let mut credentials = Vec::with_capacity(creds_to_generate as usize);

        if creds_to_generate == 1 {
            let type_dist = seeded_rng.next_u32();

            let cred_len = if type_dist < CRED_SINGLE_68 {
                 
                G_PIXEL_CRED_LEN
            } else if type_dist < CRED_SINGLE_98 {
                 
                APPLE_CRED_LEN
            } else {
                 
                BITWARDEN_CRED_LEN
            };

            let mut cred = vec![0; cred_len];
            seeded_rng.fill_bytes(&mut cred);

            credentials.push(cred);
        } else {
            for _i in 0..creds_to_generate {
                let type_dist = seeded_rng.next_u32();

                let cred_len = if type_dist < CRED_MULTI_ANDROID {
                     
                    G_PIXEL_CRED_LEN
                } else if type_dist < CRED_MULTI_WINDOWS {
                     
                    TPM_CRED_LEN
                } else if type_dist < CRED_MULTI_APL {
                     
                    APPLE_CRED_LEN
                } else if type_dist < CRED_MULTI_YK {
                     
                    YUBIKEY_CRED_LEN
                } else {
                    BITWARDEN_CRED_LEN
                };

                let mut cred = vec![0; cred_len];
                seeded_rng.fill_bytes(&mut cred);

                credentials.push(cred);
            }
        }

        credentials
    }
}

 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
 
pub struct WebauthnFakeCredentialGenerator<D>
where
    D: FakeCredentialIDDistribution,
{
    hmac_key: HmacSha256Key,
    distribution: std::marker::PhantomData<D>,
}

impl<D> WebauthnFakeCredentialGenerator<D>
where
    D: FakeCredentialIDDistribution,
{
     
     
    pub fn new_hmac_key() -> Result<HmacSha256Key, WebauthnError> {
        Ok(hmac_s256::new_key())
    }
}

impl<D> WebauthnFakeCredentialGenerator<D>
where
    D: FakeCredentialIDDistribution,
{
     
     
     
     
    pub fn new(hmac_key: &[u8]) -> Result<Self, WebauthnError> {
        let hmac_key = hmac_s256::key_from_slice(hmac_key).ok_or(WebauthnError::HmacKeyInvalid)?;

        Ok(WebauthnFakeCredentialGenerator {
            hmac_key,
            distribution: std::marker::PhantomData,
        })
    }

     
    pub fn generate(&self, username: &[u8]) -> Result<Vec<CredentialID>, WebauthnError> {
         

        let hmac_out = hmac_s256::oneshot(&self.hmac_key, username);

        let mut seed = [0; 32];
        seed.copy_from_slice(&hmac_out.into_bytes());

         
        let mut seeded_rng = ChaCha8Rng::from_seed(seed);

        let credentials = D::generate(&mut seeded_rng);

        Ok(credentials)
    }
}

