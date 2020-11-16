use bdk::Wallet;
use bdk::blockchain;
use bdk::blockchain::Blockchain;
use bdk::database::BatchDatabase;
use lindy::bitcoin::Network;
use lindy::channel::ChannelId;
use lindy::http::api_reply::ErrorMessage;
use lindy::http::messages::*;
use lindy::p2p::conn::Peer;
use lindy::reqwest::Url;
use lindy::seed::Seed;
use log::*;
use std::net::SocketAddrV4;
use std::str::FromStr;
use tokio::sync::Mutex;
use anyhow::anyhow;

async fn start_party(id: u8) -> anyhow::Result<Party> {
    let seed = Seed::from([id; 64]);
    let xpriv = seed.test_wallet_xpriv();
    let descriptor = bdk::template::BIP84(xpriv, bdk::ScriptType::External);
    let esplora = bdk::blockchain::EsploraBlockchain::new("http://localhost:3000");
    let db = bdk::database::MemoryDatabase::new();
    info!("initializng party {} wallet", id);
    let wallet = bdk::Wallet::new(descriptor, None, Network::Regtest, db, esplora).await?;
    wallet.sync(blockchain::log_progress(),None).await?;

    while wallet.get_balance()?  < 100_000 {
        fund_wallet(&wallet).await?;
        wallet.sync(blockchain::log_progress(),None).await?;
    }

    info!("party {} starting with {}",id, wallet.get_balance()?);

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

    tokio::spawn(lindy::server_bundle::start(
        seed,
        p2p_listener,
        http_listener,
        Mutex::new(wallet),
    ));
    let http_base_url = lindy::reqwest::Url::from_str(&format!("http://{}", http_addr)).unwrap();
    Ok(Party {
        id,
        http_base_url,
        client: lindy::reqwest::Client::new(),
    })
}

pub async fn fund_wallet(
    wallet: &Wallet<impl Blockchain, impl BatchDatabase>,
) -> anyhow::Result<()> {
    let new_address = wallet.get_new_address()?;
    println!("funding: {}", new_address);
    lindy::reqwest::Client::new().post("http://localhost:3000/faucet")
        .json(&serde_json::json!({ "address": new_address }))
        .send()
        .await?;
    Ok(())
}

struct Party {
    id: u8,
    http_base_url: Url,
    client: lindy::reqwest::Client,
}

impl Party {
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

        if res.headers().get("content-type").unwrap() == "application/json" {
            if res.status().is_client_error() || res.status().is_server_error() {
                let error = res.json::<ErrorMessage>().await?;
                Err(anyhow!("{}", error.error))
            }
            else {
               Ok(res.json::<NewChannelRes>().await?.channel_id)
            }
        }
        else {
            let _ = res.error_for_status()?;
            Err(anyhow!("Didn't get JSON from server"))
        }
    }
}

#[tokio::test]
async fn end_to_end() {
    let _ = env_logger::builder().is_test(true).try_init();
    let (party_1, party_2) = futures::join!(start_party(0), start_party(1));
    let (party_1, party_2) = (party_1.unwrap(), party_2.unwrap());

    let peer2 = party_2.get_peer_info().await.unwrap();

    let channel_id = party_1.new_channel(peer2, 100_000).await.unwrap();
    dbg!(channel_id);
    panic!()
}
