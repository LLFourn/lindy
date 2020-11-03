use std::sync::Arc;

use bdk::bitcoin::PublicKey;
use bdk::descriptor::Descriptor;
use bdk::descriptor::Segwitv0;
use bdk::miniscript::policy::concrete::Policy;
use chacha20::cipher::{NewStreamCipher, StreamCipher};
use chacha20::{ChaCha20, Key, Nonce};
use futures::channel::mpsc;
use secp256kfun::Point;
use secp256kfun::Scalar;
use secp256kfun::{g, marker::*, G};

use crate::funder::Funder;
use crate::keychain::KeyChain;
use crate::keychain::KeyPair;
use crate::p2p::conn::Peer;
use crate::p2p::event::Event;
use crate::p2p::messages::Message;
use crate::ChannelId;

pub struct ChannelManager {
    to_self_delay: u16,
    keychain: KeyChain,
    funder: Arc<dyn Funder>,
    msg_sender: mpsc::Sender<Event<Message>>,
}

impl ChannelManager {
    pub fn new(
        funder: Arc<dyn Funder>,
        keychain: KeyChain,
        msg_sender: mpsc::Sender<Event<Message>>,
    ) -> Self {
        Self {
            to_self_delay: 20,
            keychain,
            funder,
            msg_sender,
        }
    }
    pub async fn new_channel(
        &mut self,
        value: u64,
        peer: Peer,
    ) -> Result<ChannelId, anyhow::Error> {
        let pr =
            PointRandomizer::from_ecdh(&self.keychain.node_keypair(), &peer.key.to_point(), true);
        let ((_, p1), (_, p2)) = pr.points_at(0);

        let policy = Policy::<PublicKey>::Or(vec![
            (
                1,
                Policy::Key(PublicKey {
                    compressed: false,
                    key: p1.into(),
                }),
            ),
            (
                1,
                Policy::Key(PublicKey {
                    compressed: false,
                    key: p2.into(),
                }),
            ),
        ]);

        let descriptor = Descriptor::Wsh(policy.compile::<Segwitv0>().unwrap());
        let fund_tx = self
            .funder
            .create_fund_transaction(value, descriptor)
            .await?;

        let message = Message::RsEcdsaNewChannel {
            channel_id: fund_tx.channel_id(),
            funding_satoshis: value,
            to_self_delay: 40,
            feerate_per_kw: 372 * 250,
            revocation_key: G.clone().mark::<Normal>(),
        };

        Event::send_message(&mut self.msg_sender, peer, message).await?;

        Ok(fund_tx.channel_id())
    }
}

pub struct PointRandomizer {
    base_1: Point,
    base_2: Point,
    shared_secret: Key,
}

impl PointRandomizer {
    pub fn new(base_1: Point, base_2: Point, shared_secret: [u8; 32]) -> Self {
        Self {
            base_1,
            base_2,
            shared_secret: Key::from(shared_secret),
        }
    }

    pub fn from_ecdh(local: &KeyPair, remote: &Point<EvenY>, i_am_party_1: bool) -> Self {
        use crate::bitcoin::hashes::{sha256, Hash};
        let point = g!(local.secret_key * remote).mark::<Normal>();
        let res = sha256::Hash::hash(point.to_bytes().as_ref());
        match i_am_party_1 {
            true => Self::new(
                local.public_key.clone().mark::<Normal>(),
                remote.clone().mark::<Normal>(),
                res.into_inner(),
            ),
            false => Self::new(
                remote.clone().mark::<Normal>(),
                local.public_key.clone().mark::<Normal>(),
                res.into_inner(),
            ),
        }
    }

    pub fn points_at(&self, index: u64) -> ((Scalar, Point), (Scalar, Point)) {
        let mut chacha =
            ChaCha20::new(&self.shared_secret, Nonce::from_slice(&index.to_be_bytes()));
        let mut r1 = [0u8; 32];
        let mut r2 = [0u8; 32];
        chacha.encrypt(&mut r1);
        chacha.encrypt(&mut r2);
        let r1 = Scalar::from_bytes_mod_order(r1).mark::<NonZero>().unwrap();
        let r2 = Scalar::from_bytes_mod_order(r2).mark::<NonZero>().unwrap();
        // we can unwrap here because in order to trigger it an attacker would have to generate a
        // shared secret that produces a zero mod q. It would probably be hard to find a 0 chacha
        // stream in general but to find the shared secret for that against our key would be
        // impossible in itself.
        let point1 = g!(self.base_1 + r1 * G)
            .mark::<(Normal, NonZero)>()
            .unwrap();
        let point2 = g!(self.base_2 + r2 * G)
            .mark::<(Normal, NonZero)>()
            .unwrap();
        ((r1, point1), (r2, point2))
    }
}
