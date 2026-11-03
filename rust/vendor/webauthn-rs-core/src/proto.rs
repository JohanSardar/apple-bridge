 
 

use crate::base64_data::Base64UrlSafeData;
use crate::error::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, convert::TryFrom};
use std::borrow::Borrow;
use std::ops::Deref;

#[cfg(feature = "wasm")]
use js_sys::{Array, Object, Uint8Array};
#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;


#[cfg(feature = "core")]
impl CredProtect {
     
    pub fn new(
        credential_protection_policy: CredentialProtectionPolicy,
        enforce_credential_protection_policy: Option<bool>,
    ) -> Self {
        CredProtect {
            credential_protection_policy,
            enforce_credential_protection_policy,
        }
    }
}


impl RequestAuthenticationExtensions {
     
    #[must_use]
    pub fn builder() -> RequestAuthenticationExtensionsBuilder {
        RequestAuthenticationExtensionsBuilder::new()
    }
}

 
pub struct RequestAuthenticationExtensionsBuilder(RequestAuthenticationExtensions);

impl RequestAuthenticationExtensionsBuilder {
    pub(crate) fn new() -> Self {
        Self(RequestAuthenticationExtensions {
            appid: None,
        })
    }

     
    pub fn build(self) -> RequestAuthenticationExtensions {
        self.0
    }

     
     
     
     
     
     
     
     
     
     
     
    pub fn appid(mut self, appid: String) -> Self {
        self.0.appid = Some(appid);
        self
    }
}


impl RequestRegistrationExtensions {
     
    #[must_use]
    pub fn builder() -> RequestRegistrationExtensionsBuilder {
        RequestRegistrationExtensionsBuilder::new()
    }
}

 
pub struct RequestRegistrationExtensionsBuilder(RequestRegistrationExtensions);

impl RequestRegistrationExtensionsBuilder {
    pub(crate) fn new() -> Self {
        Self(RequestRegistrationExtensions {
            cred_protect: None,
        })
    }

     
    pub fn build(self) -> RequestRegistrationExtensions {
        self.0
    }

     
     
     
     
     
     
     
     
     
     
     
     
     
     
     
    pub fn cred_protect(mut self, cred_protect: CredProtect) -> Self {
        self.0.cred_protect = Some(cred_protect);
        self
    }
}



 
#[cfg(feature = "core")]


#[cfg(feature = "core")]

#[cfg(feature = "core")]
#[derive(Debug)]
pub(crate) struct AuthenticatorAssertionResponse<T: Ceremony> {
    pub(crate) authenticator_data: AuthenticatorData<T>,
    pub(crate) authenticator_data_bytes: Vec<u8>,
    pub(crate) client_data: CollectedClientData,
    pub(crate) client_data_bytes: Vec<u8>,
    pub(crate) signature: Vec<u8>,
    pub(crate) _user_handle: Option<Vec<u8>>,
}

#[cfg(feature = "core")]
impl<T: Ceremony> TryFrom<&AuthenticatorAssertionResponseRaw>
    for AuthenticatorAssertionResponse<T>
{
    type Error = WebauthnError;
    fn try_from(aarr: &AuthenticatorAssertionResponseRaw) -> Result<Self, Self::Error> {
        Ok(AuthenticatorAssertionResponse {
            authenticator_data: AuthenticatorData::try_from(aarr.authenticator_data.as_ref())?,
            authenticator_data_bytes: aarr.authenticator_data.clone().into(),
            client_data: CollectedClientData::try_from(aarr.client_data_json.as_ref())?,
            client_data_bytes: aarr.client_data_json.clone().into(),
            signature: aarr.signature.clone().into(),
            _user_handle: aarr.user_handle.clone().map(|uh| uh.into()),
        })
    }
}


 
 
 
 
 
 
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PublicKeyCredential {
     
    pub id: String,
     
    #[serde(rename = "rawId")]
    pub raw_id: Base64UrlSafeData,
     
    pub response: AuthenticatorAssertionResponseRaw,
     
    pub extensions: Option<AuthenticationExtensionsClientOutputs>,
     
    #[serde(rename = "type")]
    pub type_: String,
}

impl PublicKeyCredential {
     
    pub fn get_user_handle(&self) -> Option<&[u8]> {
        self.response.user_handle.as_ref().map(|uh| uh.as_ref())
    }
}

