#![feature(async_closure)]

#[macro_use]
extern crate log;
mod config;
mod hex;
mod seed;
pub use config::Config;
pub mod http_api;
pub mod keychain;
pub mod p2p;
pub use bdk::bitcoin;

pub type PeerId = std::net::SocketAddr;
pub type ChannelId = bitcoin::OutPoint;
