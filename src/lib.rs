#![feature(async_closure)]

#[macro_use]
extern crate log;
mod config;
mod hex;
pub mod seed;
pub use config::Config;
pub mod channel;
pub mod funder;
pub mod http;
pub mod keychain;
pub mod monitor;
pub mod p2p;
pub mod server_bundle;
pub use bdk::{bitcoin, reqwest};

pub type PeerId = secp256kfun::XOnly;
