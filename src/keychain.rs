use crate::seed::Seed;
use secp256kfun::{marker::*, Point, Scalar};

pub struct KeyChain {
    seed: Seed,
    keypair: KeyPair,
}

pub struct KeyPair {
    pub secret_key: Scalar,
    pub public_key: Point<EvenY>,
}

impl KeyChain {
    pub fn new(seed: Seed) -> Self {
        let keypair = seed.child(b"lindy").keypair(b"node-key");
        Self { seed, keypair }
    }


    pub fn node_keypair(&self) -> &KeyPair {
        &self.keypair
    }
}
