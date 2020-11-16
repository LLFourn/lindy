use crate::channel::ChannelId;
use crate::p2p::conn::Peer;
use crate::PeerId;

#[derive(serde::Deserialize, serde::Serialize)]
struct ChannelCreate {
    pub hello: String,
}

pub type NewPeerReq = Peer;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct NewPeerRes {
    pub id: PeerId,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct RootInfo {
    pub me: Peer,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct NewChannelRes {
    pub channel_id: ChannelId,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct NewChannelReq {
    pub peer: Peer,
    pub value: u64,
}
