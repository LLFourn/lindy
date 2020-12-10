use super::{Channel, ChannelDb, ChannelId};
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::RwLock;

#[async_trait]
impl ChannelDb for RwLock<InMemory> {
    async fn insert_channel(&self, channel: Channel) -> anyhow::Result<()> {
        let mut db = self.write().await;
        db.insert_channel(channel);
        Ok(())
    }

    async fn get_channel(&self, channel_id: ChannelId) -> anyhow::Result<Option<Channel>> {
        let db = self.read().await;
        Ok(db.get_channel(channel_id))
    }

    async fn list_channels(&self) -> anyhow::Result<Vec<Channel>> {
        let db = self.read().await;
        Ok(db.list_channels().await)
    }
}

#[derive(Default)]
pub struct InMemory {
    inner: HashMap<ChannelId, Channel>,
}

impl InMemory {
    pub fn insert_channel(&mut self, channel: Channel) {
        self.inner.insert(channel.id, channel);
    }

    pub fn get_channel(&self, channel_id: ChannelId) -> Option<Channel> {
        self.inner.get(&channel_id).map(Clone::clone)
    }

    async fn list_channels(&self) -> Vec<Channel> {
        self.inner.values().map(Clone::clone).collect()
    }
}
