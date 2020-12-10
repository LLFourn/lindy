use crate::channel::rs_ecdsa::RsEcdsaAckNewChannel;
use crate::channel::rs_ecdsa::RsEcdsaNewChannel;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Message {
    RsEcdsaNewChannel(RsEcdsaNewChannel),
    RsEcdsaAckNewChannel(RsEcdsaAckNewChannel),
}
