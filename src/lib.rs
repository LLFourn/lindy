#![feature(async_closure)]

#[macro_use]
extern crate log;
mod config;
mod hex;
mod seed;
pub use config::Config;
pub mod http_api;
pub mod p2p;

pub type PeerId = std::net::SocketAddr;
