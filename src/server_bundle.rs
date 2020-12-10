use crate::channel::channel_db;
use crate::channel::rs_ecdsa::ChannelManager;
use crate::funder::Funder;
use crate::keychain::KeyChain;
use crate::monitor;
use crate::p2p::conn::Connector;
use crate::p2p::conn::Peer;
use crate::p2p::event::Event;
use crate::p2p::event::EventHandler;
use crate::p2p::messages::Message;
use crate::seed::Seed;
use bdk::blockchain::Blockchain;
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
    blockchain: impl Blockchain + 'static,
) -> anyhow::Result<()> {
    let keychain = KeyChain::new(seed.clone());
    let local_key = keychain.node_keypair().public_key.to_xonly();
    let connector = Connector { local_key };

    let (event_handler_loop, mut event_sender, mut message_receiver) =
        EventHandler::start(connector);
    let me = Peer {
        addr: p2p_listener.local_addr().unwrap(),
        key: keychain.node_keypair().public_key.to_xonly(),
    };
    let p2p_listen_loop = crate::p2p::conn::listen(p2p_listener)
        .await?
        .map(|item| Ok(Event::from(item)))
        .forward(event_sender.clone());

    let db = Arc::new(RwLock::new(channel_db::InMemory::default()));
    let (monitor_loop, monitor) = monitor::start(blockchain);
    let channel_manager = Arc::new(Mutex::new(ChannelManager::new(
        Arc::new(funder),
        db.clone(),
        keychain,
        event_sender.clone(),
        monitor,
    )));

    let routes = crate::http::routes(
        event_sender.clone(),
        channel_manager.clone(),
        db.clone(),
        me,
    );
    let http_server = warp::serve(routes).serve_incoming(http_listener);

    let message_handler_loop = async move {
        while let Some((peer, message)) = message_receiver.next().await {
            info!("received message from {}", peer);
            let mut channel_manager = channel_manager.lock().await;
            match message {
                Message::RsEcdsaNewChannel(msg) => {
                    let ack = channel_manager.recv_new_channel(peer.id(), msg).await;
                    match ack {
                        Ok(ack) => {
                            if let Err(e) = Event::send_message(
                                &mut event_sender,
                                peer,
                                Message::RsEcdsaAckNewChannel(ack),
                            )
                            .await
                            {
                                error!("Failed to send message to {}: {}", peer, e);
                            }
                        }
                        Err(e) => error!("Failed to accept new incoming channel: {}", e),
                    }
                }
                Message::RsEcdsaAckNewChannel(msg) => {
                    let channel_id = msg.id;
                    match channel_manager.recv_new_channel_ack(peer.id(), msg).await {
                        Ok(_) => info!(
                            "Received ack for our new channel {} from {}",
                            channel_id, peer
                        ),
                        Err(e) => error!(
                            "Failure during processing ack for new channel {} from {}: {}",
                            channel_id,
                            peer.id(),
                            e
                        ),
                    }
                }
            }
        }
    };

    let _ = futures::join!(
        event_handler_loop,
        http_server,
        p2p_listen_loop,
        message_handler_loop,
        monitor_loop
    );
    Ok(())
}
