use crate::channel::ChannelId;
use crate::seed::Seed;
use crate::PeerId;
use secp256kfun::{marker::*, Point, Scalar};

#[derive(Clone, Debug)]
pub struct KeyChain {
    seed: Seed,
    keypair: KeyPair,
}

#[derive(Clone, Debug)]
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

    pub fn channel_seed(&self, peer_id: PeerId, channel_id: ChannelId) -> Seed {
        self.seed
            .child(peer_id.to_string().as_bytes())
            .child(channel_id.to_string().as_bytes())
    }
}
