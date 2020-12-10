use crate::PeerId;
use futures::channel::mpsc;
use futures::channel::mpsc::UnboundedSender;
use futures::channel::oneshot;
use futures::Future;
use futures::Sink;
use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use std::collections::HashMap;
use std::pin::Pin;

use super::conn;
use super::conn::Connector;
use super::conn::Peer;

pub type PeerSink<M> = Pin<Box<dyn Sink<M, Error = conn::Error> + Send + 'static>>;
pub type PeerStream<M> = Pin<Box<dyn Stream<Item = M> + Send + 'static>>;

pub enum Event<M> {
    ConnectionEstablished {
        peer: Peer,
        incoming: PeerStream<M>,
        outgoing: PeerSink<M>,
    },
    ConnectionFailed {
        peer: Peer,
        reason: conn::Error,
    },
    OutgoingConnectionReq {
        peer: Peer,
        notify: Option<Notifier>,
    },
    SendMessageReq {
        peer: Peer,
        message: M,
        notifier: Option<Notifier>,
    },
    NewIncomingMessage {
        peer: Peer,
        message: M,
    },
    Disconnected {
        peer: Peer,
    },
}

impl<M> Event<M> {
    pub async fn send_message(
        event_sender: &mut mpsc::Sender<Self>,
        peer: Peer,
        message: M,
    ) -> Result<(), conn::Error> {
        let (sender, receiver) = oneshot::channel();
        event_sender
            .send(Event::SendMessageReq {
                peer,
                message: message,
                notifier: Some(sender),
            })
            .await
            .expect("receiver won't be dropped");
        receiver.await.expect("sender can't be dropped")
    }
}

impl<M, Si, St> From<(Peer, Si, St)> for Event<M>
where
    M: Send + 'static,
    Si: Sink<M, Error = conn::Error> + Send + 'static,
    St: Stream<Item = M> + Send + 'static,
{
    fn from((peer, outgoing, incoming): (Peer, Si, St)) -> Self {
        Event::ConnectionEstablished {
            peer,
            incoming: Box::pin(incoming),
            outgoing: Box::pin(outgoing),
        }
    }
}

pub type Notifier = oneshot::Sender<Result<(), conn::Error>>;

enum ConnectState<M> {
    Connected(mpsc::UnboundedSender<(M, Option<Notifier>)>),
    Connecting {
        notify_on_connect: Vec<Notifier>,
        pending_messages: Vec<(M, Option<Notifier>)>,
    },
}

impl<M> Default for ConnectState<M> {
    fn default() -> Self {
        ConnectState::Connecting {
            notify_on_connect: vec![],
            pending_messages: vec![],
        }
    }
}

pub struct EventHandler<M> {
    peers: HashMap<PeerId, ConnectState<M>>,
    sender: mpsc::Sender<Event<M>>,
    connector: Connector,
    msg_sender: UnboundedSender<(Peer, M)>,
}

impl<M> EventHandler<M>
where
    M: Send + serde::Serialize + serde::de::DeserializeOwned + 'static,
{
    pub fn start(
        connector: Connector,
    ) -> (
        impl Future<Output = ()> + Send,
        mpsc::Sender<Event<M>>,
        impl Stream<Item = (Peer, M)>,
    ) {
        let (sender, mut receiver) = mpsc::channel(10);
        let (msg_sender, msg_receiver) = mpsc::unbounded();
        let mut handler = EventHandler {
            peers: HashMap::default(),
            sender: sender.clone(),
            msg_sender,
            connector,
        };
        let fut = async move {
            while let Some(event) = receiver.next().await {
                handler.handle_event(event);
            }
        };

        (fut, sender, msg_receiver)
    }

    fn handle_event(&mut self, event: Event<M>) {
        match event {
            Event::OutgoingConnectionReq { peer, notify } => match self.maybe_connect(peer) {
                ConnectState::Connected(_) => info!(
                    "Already connected to {}. Ingorning connection request.",
                    peer
                ),
                ConnectState::Connecting {
                    notify_on_connect, ..
                } => {
                    if let Some(notify) = notify {
                        notify_on_connect.push(notify);
                    }
                }
            },
            Event::SendMessageReq {
                peer,
                message,
                notifier,
            } => match self.maybe_connect(peer) {
                ConnectState::Connected(outgoing_queue) => {
                    debug!("Using existing connection to send message to {}", peer);
                    outgoing_queue.unbounded_send((message, notifier)).unwrap();
                }
                ConnectState::Connecting {
                    pending_messages, ..
                } => {
                    debug!(
                        "Currently connecting to {}, so putting message in queue",
                        peer
                    );
                    pending_messages.push((message, notifier));
                }
            },
            Event::NewIncomingMessage { peer, message } => {
                let _ = self.msg_sender.unbounded_send((peer, message));
            }
            Event::Disconnected { peer } => {
                let removed = self.peers.remove(&peer.id());
                match removed.is_some() {
                    true => info!("disconnected from {}", peer),
                    false => debug!("redundant disconnect event for {}", peer),
                }
            }
            Event::ConnectionEstablished {
                peer,
                incoming,
                outgoing,
            } => {
                self.spawn_outgoing_messages(peer, outgoing);
                self.spawn_incoming_messages(peer, incoming);
            }
            Event::ConnectionFailed { peer, reason } => {
                if let Some(ConnectState::Connecting {
                    mut notify_on_connect,
                    mut pending_messages,
                }) = self.peers.remove(&peer.id())
                {
                    for notify in notify_on_connect.drain(..) {
                        let _ = notify.send(Err(reason));
                    }

                    for (_, notify) in pending_messages.drain(..) {
                        if let Some(notify) = notify {
                            let _ = notify.send(Err(reason));
                        }
                    }
                }
            }
        }
    }

    fn maybe_connect(&mut self, peer: Peer) -> &mut ConnectState<M> {
        let mut sender = self.sender.clone();
        let connection = self.connector.connect(peer);
        self.peers.entry(peer.id()).or_insert_with(move || {
            tokio::spawn(async move {
                match connection.await {
                    Ok((peer, incoming, outgoing)) => {
                        info!("successfully connected to {}", peer);
                        sender
                            .send(Event::<M>::from((peer, incoming, outgoing)))
                            .await
                            .unwrap();
                    }
                    Err(e) => sender
                        .send(Event::ConnectionFailed { peer, reason: e })
                        .await
                        .unwrap(),
                }
            });
            ConnectState::default()
        })
    }

    fn spawn_outgoing_messages(&mut self, peer: Peer, mut sink: PeerSink<M>) {
        let connect_state = self.peers.entry(peer.id()).or_default();
        let (sender, mut receiver) = mpsc::unbounded();
        if let ConnectState::Connecting {
            notify_on_connect,
            pending_messages,
        } = connect_state
        {
            for notifier in notify_on_connect.drain(..) {
                let _ = notifier.send(Ok(()));
            }
            let message_sender = sender.clone();
            for pending_message in pending_messages.drain(..) {
                // queue up pending messages
                message_sender.unbounded_send(pending_message).unwrap();
            }
        }

        tokio::spawn(async move {
            while let Some((message, notifier)) = receiver.next().await {
                let res = match sink.send(message).await {
                    Ok(_) => {
                        debug!("successfully sent message to {}", peer);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to send message to {}: {}", peer, e);
                        Err(e)
                    }
                };

                if let Some(notifier) = notifier {
                    let _ = notifier.send(res);
                }
            }
        });

        *connect_state = ConnectState::Connected(sender);
    }

    fn spawn_incoming_messages(&self, peer: Peer, mut stream: PeerStream<M>) {
        let mut sender = self.sender.clone();
        let read_msg_loop = async move {
            while let Some(message) = stream.next().await {
                debug!("received message from {}", peer);
                sender
                    .send(Event::NewIncomingMessage { peer, message })
                    .await
                    .unwrap();
            }
            sender.send(Event::Disconnected { peer }).await.unwrap();
        };

        tokio::spawn(read_msg_loop);
    }
}
