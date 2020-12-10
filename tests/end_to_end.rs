use anyhow::anyhow;
use bdk::blockchain;
use bdk::blockchain::Blockchain;
use bdk::database::BatchDatabase;
use bdk::ScriptType;
use bdk::Wallet;
use lindy::bitcoin::Network;
use lindy::bitcoin::Transaction;
use lindy::bitcoin::consensus::Encodable;
use lindy::channel::Channel;
use lindy::channel::ChannelId;
use lindy::channel::ChannelState;
use lindy::http::api_reply::ErrorMessage;
use lindy::http::messages::*;
use lindy::monitor;
use lindy::p2p::conn::Peer;
use lindy::reqwest;
use lindy::reqwest::Url;
use lindy::seed::Seed;
use log::*;
use serde::de::DeserializeOwned;
use std::net::SocketAddrV4;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::Mutex;

async fn start_party(id: u8) -> anyhow::Result<Party<impl Blockchain, impl BatchDatabase>> {
    let seed = Seed::from([id; 64]);
    let xpriv = seed.test_wallet_xpriv();
    info!("initializng party {} wallet", id);
    let user_wallet = bdk::Wallet::new(
        bdk::template::BIP84(xpriv, bdk::ScriptType::External),
        None,
        Network::Regtest,
        bdk::database::MemoryDatabase::new(),
        bdk::blockchain::EsploraBlockchain::new("http://localhost:3000"),
    )
    .await?;
    let node_wallet = bdk::Wallet::new(
        user_wallet
            .public_descriptor(ScriptType::External)
            .unwrap()
            .unwrap(),
        None,
        Network::Regtest,
        bdk::database::MemoryDatabase::new(),
        bdk::blockchain::EsploraBlockchain::new("http://localhost:3000"),
    )
    .await?;
    node_wallet.sync(blockchain::log_progress(), None).await?;

    while node_wallet.get_balance()? < 100_000 {
        fund_wallet(&node_wallet).await?;
        node_wallet.sync(blockchain::log_progress(), None).await?;
    }

    info!("party {} starting with {}", id, node_wallet.get_balance()?);

    let random_port_loopback = SocketAddrV4::from_str("127.0.0.1:0").unwrap();

    let p2p_listener = tokio::net::TcpListener::bind(random_port_loopback).await?;
    info!(
        "party {} listening for p2p connections on {}",
        id,
        p2p_listener.local_addr().unwrap()
    );
    let http_listener = tokio::net::TcpListener::bind(random_port_loopback).await?;
    info!(
        "party {} listening for http connections on {}",
        id,
        http_listener.local_addr().unwrap()
    );
    let http_addr = http_listener.local_addr().unwrap();
    tokio::task::spawn_local(lindy::server_bundle::start(
        seed,
        p2p_listener,
        http_listener,
        Mutex::new(node_wallet),
        bdk::blockchain::EsploraBlockchain::new("http://localhost:3000"),
    ));
    let http_base_url = lindy::reqwest::Url::from_str(&format!("http://{}", http_addr)).unwrap();
    Ok(Party {
        id,
        http_base_url,
        client: lindy::reqwest::Client::new(),
        user_wallet,
    })
}

pub async fn fund_wallet(
    wallet: &Wallet<impl Blockchain, impl BatchDatabase>,
) -> anyhow::Result<()> {
    let new_address = wallet.get_new_address()?;
    println!("funding: {}", new_address);
    lindy::reqwest::Client::new()
        .post("http://localhost:3000/faucet")
        .json(&serde_json::json!({ "address": new_address }))
        .send()
        .await?;
    Ok(())
}

struct Party<B, D> {
    id: u8,
    http_base_url: Url,
    client: lindy::reqwest::Client,
    pub user_wallet: Wallet<B, D>,
}

impl<B: Blockchain, D: BatchDatabase> Party<B, D> {
    fn url(&self, path: &str) -> Url {
        let mut url = self.http_base_url.clone();
        url.set_path(path);
        url
    }

    async fn get_peer_info(&self) -> anyhow::Result<Peer> {
        Ok(self
            .client
            .get(self.url(""))
            .send()
            .await?
            .json::<RootInfo>()
            .await?
            .me)
    }

    async fn new_channel(&self, peer: Peer, value: u64) -> anyhow::Result<ChannelId> {
        let res = self
            .client
            .post(self.url("/channels"))
            .json(&NewChannelReq { peer, value })
            .send()
            .await?;

        Ok(Self::try_deser::<NewChannelRes>(res).await?.channel_id)
    }

    async fn _list_channels(&self) -> anyhow::Result<Vec<ChannelId>> {
        Ok(self
            .client
            .get(self.url("/channels"))
            .send()
            .await?
            .json::<GetChannels>()
            .await?
            .channels)
    }

    async fn get_channel(&self, channel_id: ChannelId) -> anyhow::Result<Channel> {
        let res = self
            .client
            .get(self.url(&format!("/channels/{}", channel_id)))
            .send()
            .await?;

        Ok(Self::try_deser::<GetChannel>(res).await?.channel)
    }

    async fn force_close(&self, channel_id: ChannelId) -> anyhow::Result<Transaction> {
        let res = self
            .client
            .put(self.url(&format!("/channels/{}", channel_id)))
            .json(&ChannelUpdate::ForceClose {})
            .send()
            .await?;

        Ok(Self::try_deser::<ForceCloseChannelRes>(res).await?.tx)
    }

    async fn try_deser<O: DeserializeOwned>(res: reqwest::Response) -> anyhow::Result<O> {
        if res.headers().get("content-type").unwrap() == "application/json" {
            if res.status().is_client_error() || res.status().is_server_error() {
                let error = res.json::<ErrorMessage>().await?;
                Err(anyhow!("{}", error.error))
            } else {
                Ok(res.json::<O>().await?)
            }
        } else {
            let _ = res.error_for_status()?;
            Err(anyhow!("Didn't get JSON from server"))
        }
    }
}

#[tokio::test]
async fn end_to_end() {
    let _ = env_logger::builder().is_test(true).try_init();
    info!("starting test");
    let local = tokio::task::LocalSet::new();
    let (party_1, party_2) = local
        .run_until(futures::future::join(start_party(0), start_party(1)))
        .await;
    let (party_1, party_2) = (party_1.unwrap(), party_2.unwrap());

    info!("get peer info");
    let peer2 = local.run_until(party_2.get_peer_info()).await.unwrap();

    info!("new channel");
    let channel_id = local
        .run_until(party_1.new_channel(peer2, 100_000))
        .await
        .unwrap();

    match local
        .run_until(party_2.get_channel(channel_id))
        .await
        .unwrap()
        .state
    {
        ChannelState::WaitingForFund { .. } => {
            info!("party_2 received the ack'd the channel request")
        }
        _ => panic!("party_2 is not in right state"),
    }

    while let ChannelState::Pending { .. } = local
        .run_until(party_1.get_channel(channel_id))
        .await
        .unwrap()
        .state
    {
        tokio::time::delay_for(Duration::from_millis(5000)).await;
    }

    let channel = local
        .run_until(party_1.get_channel(channel_id))
        .await
        .unwrap();

    match channel.state {
        ChannelState::ReadyToFund { fund_tx, .. } => {
            let (psbt, is_final) = party_1.user_wallet.sign(fund_tx.psbt, None).unwrap();
            assert!(is_final, "tx should be final");
            let tx = psbt.extract_tx();
            local
                .run_until(party_1.user_wallet.broadcast(tx))
                .await
                .unwrap();
            info!("waiting for channel to be funded");
            while let ChannelState::ReadyToFund { .. } = local
                .run_until(party_1.get_channel(channel_id))
                .await
                .unwrap()
                .state
            {
                local
                    .run_until(tokio::time::delay_for(Duration::from_millis(2000)))
                    .await;
            }

            let tx = local
                .run_until(party_1.force_close(channel_id))
                .await
                .unwrap();

            let mut bytes = vec![];
            tx.consensus_encode(&mut bytes).unwrap();
            println!("{}", secp256kfun::hex::encode(&bytes));

            party_1.user_wallet.broadcast(tx).await.unwrap();
        }
        _ => panic!("should be in ReadyToFund"),
    }
}
