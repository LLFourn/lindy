#[derive(Clone)]
pub struct Seed([u8; 64]);
use crate::bitcoin::hashes::{sha512, Hash, HashEngine, Hmac, HmacEngine};
use crate::keychain::KeyPair;
use secp256kfun::{marker::*, Point, Scalar, G};

crate::impl_fromstr_deserailize! {
    name => "32-byte lindy seed",
    fn from_bytes(bytes: [u8;64]) -> Option<Seed> {
        Some(Seed(bytes))
    }
}

impl std::fmt::Debug for Seed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX")
    }
}

impl Seed {
    fn to_sha2_hmac(&self) -> HmacEngine<sha512::Hash> {
        HmacEngine::<sha512::Hash>::new(&self.0[..])
    }

    pub fn child(&self, tag: &[u8]) -> Self {
        let mut hmac = self.to_sha2_hmac();
        hmac.input(tag);
        let res = Hmac::from_engine(hmac);
        let mut bytes = [0u8; 64];
        bytes.copy_from_slice(&res[..]);
        Seed(bytes)
    }

    pub const fn new(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    pub fn keypair(&self, tag: &[u8]) -> KeyPair {
        let mut hmac = self.to_sha2_hmac();
        hmac.input(tag);
        let res = Hmac::from_engine(hmac);
        Self::keypair_from_slice(&res[..])
    }

    fn keypair_from_slice(hmac: &[u8]) -> KeyPair {
        let mut secret_key = Scalar::from_slice_mod_order(&hmac[..32])
            .expect("is 32 bytes long")
            .mark::<NonZero>()
            .expect("Computationally unreachable");
        let public_key = Point::<EvenY>::from_scalar_mul(G, &mut secret_key);
        KeyPair {
            public_key,
            secret_key,
        }
    }
}

impl From<Seed> for [u8; 64] {
    fn from(seed: Seed) -> Self {
        seed.0
    }
}
