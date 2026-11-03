 
 
 
 
 
 
 
 
 
 
 
 
 
 

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(warnings)]
#![warn(unused_extern_crates)]
#![allow(missing_docs)]
#![deny(clippy::todo)]
#![deny(clippy::unimplemented)]
#![deny(clippy::unwrap_used)]
 
#![deny(clippy::panic)]
#![deny(clippy::unreachable)]
#![deny(clippy::await_holding_lock)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::trivially_copy_pass_by_ref)]

#[macro_use]
extern crate tracing;

#[macro_use]
mod macros;

mod constants;

pub mod attestation;
pub mod crypto;
pub mod fake;

mod core;
pub mod error;
mod interface;
pub mod internals;

 
pub mod proto {
    pub use crate::interface::*;
    pub use webauthn_rs_proto::*;
}

pub use crate::core::*;
