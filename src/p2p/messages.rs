use crate::ChannelId;
use ecdsa_fun::adaptor::EncryptedSignature;
use secp256kfun::Point;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum Message {
    RsEcdsaNewChannel {
        channel_id: ChannelId,
        funding_satoshis: u64,
        to_self_delay: u16,
        feerate_per_kw: u32,
        revocation_key: Point,
    },
    RsEcdsaAckNewChannel {
        channel_id: ChannelId,
        commit_sig: EncryptedSignature,
        revocation_key: Point,
    },
}
