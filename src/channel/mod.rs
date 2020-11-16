pub mod channel_db;
pub mod rs_ecdsa;
use crate::PeerId;
use async_trait::async_trait;

pub type ChannelId = crate::bitcoin::OutPoint;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Channel {
    id: ChannelId,
    state: ChannelState,
    peer: PeerId,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum ChannelState {
    Pending,
}

#[async_trait]
pub trait ChannelDb: Send + Sync + 'static {
    async fn insert_channel(&self, channel: Channel) -> anyhow::Result<()>;
    async fn get_channel(&self, chanel_id: ChannelId) -> anyhow::Result<Option<Channel>>;
}
