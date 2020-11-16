use crate::channel::channel_db;
use crate::channel::rs_ecdsa::ChannelManager;
use crate::funder::Funder;
use crate::keychain::KeyChain;
use crate::p2p::conn::Connector;
use crate::p2p::conn::Peer;
use crate::p2p::event::Event;
use crate::p2p::event::EventHandler;
use crate::seed::Seed;
use futures::StreamExt;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::sync::RwLock;

pub async fn start(
    seed: Seed,
    p2p_listener: TcpListener,
    http_listener: TcpListener,
    funder: impl Funder,
) -> anyhow::Result<()> {
    let keychain = KeyChain::new(seed.clone());
    let local_key = keychain.node_keypair().public_key.to_xonly();
    let connector = Connector { local_key };

    let (event_handler_loop, event_sender) = EventHandler::start(
        connector,
        async move |peer_id, message: crate::p2p::messages::Message| {
            info!("got message from {}: {:?}", peer_id, message);
        },
    );
    let me = Peer {
        addr: p2p_listener.local_addr().unwrap(),
        key: keychain.node_keypair().public_key.to_xonly(),
    };
    let p2p_listen_loop = crate::p2p::conn::listen(p2p_listener)
        .await?
        .map(|item| Ok(Event::from(item)))
        .forward(event_sender.clone());

    let db = Arc::new(RwLock::new(channel_db::InMemory::default()));
    let channel_manager = Arc::new(Mutex::new(ChannelManager::new(
        Arc::new(funder),
        db.clone(),
        keychain,
        event_sender.clone(),
    )));

    let routes = crate::http::routes(event_sender, channel_manager, db.clone(), me);
    let http_server = warp::serve(routes).serve_incoming(http_listener);
    let _ = futures::join!(event_handler_loop, http_server, p2p_listen_loop);
    Ok(())
}
