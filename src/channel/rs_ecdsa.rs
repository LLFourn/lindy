use crate::funder::Funder;
use crate::keychain::KeyChain;
use crate::keychain::KeyPair;
use crate::monitor::Monitor;
use crate::p2p::event::Event;
use crate::p2p::messages::Message;
use crate::PeerId;
use anyhow::anyhow;
use bdk::bitcoin::hashes::Hash;
use bdk::bitcoin::util::bip143::SigHashCache;
use bdk::bitcoin::util::psbt::PartiallySignedTransaction as Psbt;
use bdk::bitcoin::PublicKey;
use bdk::bitcoin::Script;
use bdk::bitcoin::SigHashType;
use bdk::bitcoin::Transaction;
use bdk::bitcoin::TxIn;
use bdk::bitcoin::TxOut;
use bdk::descriptor::Descriptor;
use bdk::descriptor::Segwitv0;
use bdk::miniscript::policy::concrete::Policy;
use bdk::miniscript::psbt::PsbtInputSatisfier;
use chacha20::cipher::{NewStreamCipher, StreamCipher};
use chacha20::{ChaCha20, Key, Nonce};
use ecdsa_fun::adaptor::EncryptedSignature;
use ecdsa_fun::nonce;
use futures::channel::mpsc;
use rand::rngs::ThreadRng;
use secp256kfun::s;
use secp256kfun::secp256k1::Secp256k1;
use secp256kfun::Point;
use secp256kfun::Scalar;
use secp256kfun::{g, marker::*, G};
use sha2::Sha256;
use std::sync::Arc;

use super::Channel;
use super::ChannelDb;
use super::ChannelId;
use super::ChannelState;
use super::CommitState;

type AdaptorNonceGen = nonce::Synthetic<Sha256, nonce::GlobalRng<ThreadRng>>;

pub struct ChannelManager {
    to_self_delay: u16,
    keychain: KeyChain,
    funder: Arc<dyn Funder>,
    channel_db: Arc<dyn ChannelDb>,
    _msg_sender: mpsc::Sender<Event<Message>>,
    ecdsa_adaptor: ecdsa_fun::adaptor::Adaptor<Sha256, AdaptorNonceGen>,
    monitor: Monitor,
}

impl ChannelManager {
    pub fn new(
        funder: Arc<dyn Funder>,
        channel_db: Arc<dyn ChannelDb>,
        keychain: KeyChain,
        msg_sender: mpsc::Sender<Event<Message>>,
        monitor: Monitor,
    ) -> Self {
        Self {
            to_self_delay: 20,
            keychain,
            funder,
            _msg_sender: msg_sender,
            channel_db,
            ecdsa_adaptor: ecdsa_fun::adaptor::Adaptor::new(AdaptorNonceGen::default()),
            monitor,
        }
    }
    pub async fn new_channel(
        &mut self,
        value: u64,
        peer_id: PeerId,
    ) -> anyhow::Result<RsEcdsaNewChannel> {
        let descriptor = self.funding_descriptor_for(peer_id, true);

        let fund_tx = self
            .funder
            .create_fund_transaction(value, descriptor)
            .await?;

        let channel = Channel {
            id: fund_tx.channel_id(),
            state: ChannelState::Pending {
                fund_tx: fund_tx.clone(),
            },
            peer_id,
            value,
            i_am_initiator: true,
        };

        self.channel_db.insert_channel(channel).await?;

        let new_channel_msg = RsEcdsaNewChannel {
            id: fund_tx.channel_id(),
            funding_satoshis: value,
            to_self_delay: self.to_self_delay,
            feerate_per_kw: 372 * 250,
            publishing_key: self.my_publishing_key_for(0),
        };

        Ok(new_channel_msg)
    }

    fn point_randomizer(&self, peer: PeerId, i_am_party_1: bool) -> PointRandomizer {
        PointRandomizer::from_ecdh(
            &self.keychain.node_keypair(),
            &peer.to_point(),
            i_am_party_1,
        )
    }

    fn funding_descriptor_for(&self, peer: PeerId, i_am_party_1: bool) -> Descriptor<PublicKey> {
        let (p1, p2) = self.funding_public_keys(peer, i_am_party_1);
        let policy = Policy::<PublicKey>::And(vec![Policy::Key(p1), Policy::Key(p2)]);

        let descriptor = Descriptor::Wsh(policy.compile::<Segwitv0>().unwrap());
        descriptor
    }

    fn funding_public_keys(&self, peer: PeerId, i_am_party_1: bool) -> (PublicKey, PublicKey) {
        let pr = self.point_randomizer(peer, i_am_party_1);
        let ((_, p1), (_, p2)) = pr.points_at(PointKind::Funding);
        (
            PublicKey {
                compressed: true,
                key: p1.into(),
            },
            PublicKey {
                compressed: true,
                key: p2.into(),
            },
        )
    }

    fn funding_secret_key_for(&self, peer: PeerId, i_am_party_1: bool) -> Scalar {
        let pr = self.point_randomizer(peer, i_am_party_1);
        let ((r1, _), (r2, _)) = pr.points_at(PointKind::Funding);
        let sk = &self.keychain.node_keypair().secret_key;
        let funding_sk = if i_am_party_1 {
            s!(sk + r1)
        } else {
            s!(sk + r2)
        };

        funding_sk
            .mark::<NonZero>()
            .expect("computationally unreachable since r1/r2 is sampled as function of sk")
    }

    fn balance_descriptors(
        &self,
        peer: PeerId,
        i_am_party_1: bool,
    ) -> (Descriptor<PublicKey>, Descriptor<PublicKey>) {
        let pr = self.point_randomizer(peer, i_am_party_1);
        let ((_, p1), (_, p2)) = pr.points_at(PointKind::Balance);

        let p1 = Descriptor::Wpkh(PublicKey {
            compressed: true,
            key: p1.into(),
        });

        let p2 = Descriptor::Wpkh(PublicKey {
            compressed: true,
            key: p2.into(),
        });

        (p1, p2)
    }

    pub fn state_tx(
        &self,
        channel_id: ChannelId,
        _index: u32,
        peer: PeerId,
        commit_state: CommitState,
        i_am_party_1: bool,
    ) -> Transaction {
        let (p1_bal_des, p2_bal_des) = self.balance_descriptors(peer, i_am_party_1);
        let mut outputs = vec![];

        if commit_state.p1 > 0 {
            outputs.push(TxOut {
                script_pubkey: p1_bal_des.script_pubkey(),
                value: commit_state.p1 - 1000,
            })
        }

        if commit_state.p2 > 0 {
            outputs.push(TxOut {
                script_pubkey: p2_bal_des.script_pubkey(),
                value: commit_state.p2,
            })
        }

        let state_tx = Transaction {
            version: 1,
            lock_time: 0,
            input: vec![TxIn {
                previous_output: channel_id,
                script_sig: Script::default(),
                sequence: 0xFFFFFFFF,
                witness: vec![],
            }],
            output: outputs,
        };

        state_tx
    }

    pub fn state_psbt(
        &self,
        channel_id: ChannelId,
        _index: u32,
        peer: PeerId,
        state: CommitState,
        i_am_party_1: bool,
    ) -> Psbt {
        let tx = self.state_tx(channel_id, _index, peer, state, i_am_party_1);
        let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();
        let funding_descriptor = self.funding_descriptor_for(peer, i_am_party_1);
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: state.p1 + state.p2,
            script_pubkey: funding_descriptor.script_pubkey(),
        });
        psbt.inputs[0].witness_script = Some(funding_descriptor.witness_script());
        psbt
    }

    pub fn state_sighash(
        &self,
        channel_id: ChannelId,
        _index: u32,
        peer: PeerId,
        state: CommitState,
        i_am_party_1: bool,
    ) -> [u8; 32] {
        let tx = self.state_tx(channel_id, _index, peer, state, i_am_party_1);
        let funding_descriptor = self.funding_descriptor_for(peer, i_am_party_1);
        let mut sighash_cache = SigHashCache::new(&tx);
        let sighash = sighash_cache.signature_hash(0, &funding_descriptor.witness_script(), state.funding_value(), SigHashType::All);
        sighash.into_inner()
    }

    pub async fn recv_new_channel(
        &mut self,
        peer: PeerId,
        channel: RsEcdsaNewChannel,
    ) -> anyhow::Result<RsEcdsaAckNewChannel> {
        let funding_secret_key = self.funding_secret_key_for(peer, false);
        let commit_sig = self.ecdsa_adaptor.encrypted_sign(
            &funding_secret_key,
            &channel.publishing_key,
            &self.state_sighash(
                channel.id,
                0,
                peer,
                CommitState {
                    p1: channel.funding_satoshis,
                    p2: 0,
                },
                false,
            ),
        );

        let channel = Channel {
            id: channel.id,
            state: ChannelState::WaitingForFund,
            peer_id: peer,
            value: channel.funding_satoshis,
            i_am_initiator: false,
        };

        self.channel_db.insert_channel(channel.clone()).await?;

        self.wait_for_funding(channel.clone());

        Ok(RsEcdsaAckNewChannel {
            id: channel.id,
            commit_sig,
        })
    }

    fn my_revocation_key_for(&self, _index: u32) -> Scalar {
        s!(42)
    }

    fn my_publishing_secret_key(&self, index: u32) -> Scalar {
        let my_key = &self.keychain.node_keypair().secret_key;
        let my_rev_key = self.my_revocation_key_for(index);
        // the revocation key is pseudoranom and chosen by us so this can never be zero.
        s!(my_key + my_rev_key)
            .mark::<NonZero>()
            .expect("computationally unreachable")
    }

    fn my_publishing_key_for(&self, index: u32) -> Point {
        let publishing_secret_key = self.my_publishing_secret_key(index);
        g!(publishing_secret_key * G).mark::<Normal>()
    }

    pub async fn recv_new_channel_ack(
        &mut self,
        peer: PeerId,
        channel: RsEcdsaAckNewChannel,
    ) -> anyhow::Result<()> {
        let mut channel_in_db = self
            .channel_db
            .get_channel_for_peer(channel.id, peer)
            .await?
            .ok_or(anyhow!(
                "channel {} for peer {} did not exist",
                channel.id,
                peer
            ))?;

        match channel_in_db.state.clone() {
            ChannelState::Pending { fund_tx } => {
                let commit_state = CommitState {
                    p1: fund_tx.output_value(),
                    p2: 0,
                };

                let pr = self.point_randomizer(peer, true);
                let (_, (_, p2)) = pr.points_at(PointKind::Funding);
                let sighash = &self.state_sighash(channel.id, 0, peer, commit_state, true);

                if self.ecdsa_adaptor.verify_encrypted_signature(
                    &p2,
                    &self.my_publishing_key_for(0),
                    sighash,
                    &channel.commit_sig,
                ) {
                    channel_in_db.state = ChannelState::ReadyToFund {
                        fund_tx: fund_tx.clone(),
                        commit: commit_state,
                        commit_sig: channel.commit_sig,
                    };
                    self.channel_db
                        .insert_channel(channel_in_db.clone())
                        .await?;
                    self.wait_for_funding(channel_in_db);
                    Ok(())
                } else {
                    Err(anyhow!("adaptor signature was invalid"))
                }
            }
            _ => Err(anyhow!("received channel ack that we weren't expecting")),
        }
    }

    fn wait_for_funding(&self, mut channel: Channel) {
        let db = self.channel_db.clone();
        let monitor = self.monitor.clone();
        match channel.state.clone() {
            ChannelState::ReadyToFund {
                fund_tx,
                commit_sig,
                commit,
            } => {
                tokio::spawn(async move {
                    let on_chain_tx = monitor.wait_for_tx(channel.id.txid).await;
                    if fund_tx.matches(&on_chain_tx) {
                        channel.state = ChannelState::Live {
                            index: 0,
                            commit_sig: commit_sig.clone(),
                            commit: commit.clone(),
                        };
                        db.insert_channel(channel).await.unwrap();
                    } else {
                        unreachable!("we created this transaction so it must match")
                    }
                });
            }
            ChannelState::WaitingForFund => {
                let descriptor = self.funding_descriptor_for(channel.peer_id, false);
                tokio::spawn(async move {
                    let on_chain_tx = monitor.wait_for_tx(channel.id.txid).await;
                    match on_chain_tx.output.get(channel.id.vout as usize) {
                        Some(output) => {
                            if output.script_pubkey == descriptor.script_pubkey()
                                && output.value == channel.value
                            {
                                channel.state = ChannelState::Funded {
                                    fund_tx: on_chain_tx,
                                };
                                db.insert_channel(channel).await.unwrap();
                            }
                        }
                        None => error!("funding transaction was not what was expected"),
                    }
                });
            }
            _ => unreachable!("wait for funding is never called in other states"),
        }
    }

    pub async fn force_close_channel(&self, channel_id: ChannelId) -> anyhow::Result<Transaction> {
        let mut channel = self
            .channel_db
            .get_channel(channel_id)
            .await?
            .ok_or(anyhow!("channel does not exist"))?;
        match channel.state {
            ChannelState::Live {
                index,
                commit,
                commit_sig,
            } => {
                let remote_signature = self
                    .ecdsa_adaptor
                    .decrypt_signature(&self.my_publishing_secret_key(index), commit_sig);
                let mut psbt = self.state_psbt(
                    channel_id,
                    index,
                    channel.peer_id,
                    commit,
                    channel.i_am_initiator,
                );
                let my_funding_secret_key =
                    self.funding_secret_key_for(channel.peer_id, channel.i_am_initiator);
                let (mut local_key, mut remote_key) =
                    self.funding_public_keys(channel.peer_id, channel.i_am_initiator);

                if !channel.i_am_initiator {
                    core::mem::swap(&mut local_key, &mut remote_key);
                }

                let sighash = self.state_sighash(
                    channel_id,
                    0,
                    channel.peer_id,
                    commit,
                    channel.i_am_initiator,
                );
                let local_signature = self
                    .ecdsa_adaptor
                    .ecdsa
                    .sign(&my_funding_secret_key, &sighash);

                for (key, sig) in vec![(local_key, local_signature), (remote_key, remote_signature)]
                {
                    assert!(self
                        .ecdsa_adaptor
                        .ecdsa
                        .verify(&key.key.into(), &sighash, &sig));
                    let signature = secp256kfun::secp256k1::Signature::from(sig);
                    assert!(Secp256k1::new()
                        .verify(
                            &secp256kfun::secp256k1::Message::from_slice(&sighash[..]).unwrap(),
                            &signature,
                            &key.key
                        )
                        .is_ok());
                    psbt.inputs[0].partial_sigs.insert(key, {
                        let mut final_signature = Vec::with_capacity(75);
                        final_signature.extend_from_slice(&signature.serialize_der());
                        final_signature.push(SigHashType::All as u8);
                        final_signature
                    });
                }

                let descriptor =
                    self.funding_descriptor_for(channel.peer_id, channel.i_am_initiator);
                let mut tmp = TxIn::default();
                match descriptor.satisfy(&mut tmp, PsbtInputSatisfier::new(&psbt, 0)) {
                    Ok(()) => psbt.inputs[0].final_script_witness = Some(tmp.witness),
                    Err(e) => error!("unable to force close channel because our secret data doesn't satisfy the funding tx output: {}", e)
                }

                let close_tx = psbt.extract_tx();

                channel.state = ChannelState::Closed {
                    close_tx: close_tx.clone(),
                };
                self.channel_db.insert_channel(channel).await?;

                Ok(close_tx)
            }
            _ => Err(anyhow!("cannot force close channel that isn't live")),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct RsEcdsaNewChannel {
    pub id: ChannelId,
    pub funding_satoshis: u64,
    pub to_self_delay: u16,
    pub feerate_per_kw: u32,
    pub publishing_key: Point,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct RsEcdsaAckNewChannel {
    pub id: ChannelId,
    pub commit_sig: EncryptedSignature,
}

pub struct PointRandomizer {
    base_1: Point,
    base_2: Point,
    shared_secret: Key,
}

pub enum PointKind {
    Funding,
    Balance,
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
        use crate::bitcoin::hashes::sha256;
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

    pub fn points_at(&self, point_kind: PointKind) -> ((Scalar, Point), (Scalar, Point)) {
        let index = [&[0u8; 4][..], &(point_kind as u64).to_be_bytes()[..]].concat();
        let mut chacha = ChaCha20::new(&self.shared_secret, Nonce::from_slice(&index[..]));
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
