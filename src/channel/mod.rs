pub mod channel_db;
pub mod rs_ecdsa;
use crate::funder::FundTx;
use crate::PeerId;
use async_trait::async_trait;
use bdk::bitcoin::Transaction;
use ecdsa_fun::adaptor::EncryptedSignature;
pub type ChannelId = crate::bitcoin::OutPoint;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Channel {
    pub id: ChannelId,
    pub state: ChannelState,
    pub peer_id: PeerId,
    pub value: u64,
    pub i_am_initiator: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub enum ChannelState {
    Pending {
        fund_tx: FundTx,
    },
    ReadyToFund {
        fund_tx: FundTx,
        commit: CommitState,
        commit_sig: EncryptedSignature,
    },
    WaitingForFund,
    Funded {
        fund_tx: Transaction,
    },
    Live {
        index: u32,
        commit: CommitState,
        commit_sig: EncryptedSignature,
    },
    Closed {
        close_tx: Transaction,
    },
}



#[async_trait]
pub trait ChannelDb: Send + Sync + 'static {
    async fn insert_channel(&self, channel: Channel) -> anyhow::Result<()>;
    async fn get_channel(&self, channel_id: ChannelId) -> anyhow::Result<Option<Channel>>;
    async fn list_channels(&self) -> anyhow::Result<Vec<Channel>>;
    async fn get_channel_for_peer(
        &self,
        channel_id: ChannelId,
        peer_id: PeerId,
    ) -> anyhow::Result<Option<Channel>> {
        self.get_channel(channel_id)
            .await
            .map(|opt| opt.filter(|channel| channel.peer_id == peer_id))
    }
}


#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Copy)]
pub struct CommitState {
    p1: u64,
    p2: u64,
}


impl CommitState {
    pub fn funding_value(&self) -> u64 {
        self.p1 + self.p2
    }
}
