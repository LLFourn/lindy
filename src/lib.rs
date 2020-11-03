#![feature(async_closure)]

#[macro_use]
extern crate log;
mod config;
mod hex;
mod seed;
pub use config::Config;
pub mod channel;
pub mod keychain;
pub mod p2p;
pub use bdk::bitcoin;
pub mod funder;
pub mod http;

pub type PeerId = secp256kfun::XOnly;
pub type ChannelId = bitcoin::OutPoint;
