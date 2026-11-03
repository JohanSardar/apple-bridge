mod ca;
mod error;
mod fake;
mod identity;
mod link;
mod paircode;
mod proto;

pub use ca::{Ca, HOST, Leaf, configure_gateway, gateway_address, write_private};
pub use error::{Error, Result};
pub use fake::replay_gateway;
pub use identity::{Identity, fingerprint, valid_certificate};
pub use link::{Link, client_config};
pub use paircode::PairCode;
pub use proto::{
    CAPABILITIES, Frame, Principal, VERSION, authenticate, gateway_key, hello, input1, input2,
    join, proof_frame, random_nonce, relay1_client, relay2_client, valid_remote_address, verify,
};
