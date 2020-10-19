use crate::p2p::messages::Message;
use crate::PeerId;
use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::Future;
use futures::Sink;
use futures::SinkExt;
use futures::Stream;
use futures::StreamExt;
use std::collections::HashMap;
use std::pin::Pin;

use super::conn;

pub type PeerSink = Pin<Box<dyn Sink<Message, Error = conn::Error> + Send + 'static>>;
pub type PeerStream = Pin<Box<dyn Stream<Item = Message> + Send + 'static>>;

pub enum Event {
    ConnectionEstablished {
        peer_id: PeerId,
        incoming: PeerStream,
        outgoing: PeerSink,
    },
    ConnectionFailed {
        peer_id: PeerId,
        reason: conn::Error,
    },
    OutgoingConnectionReq {
        peer_id: PeerId,
        notify: Option<Notifier>,
    },
    SendMessageReq {
        peer_id: PeerId,
        message: (Message, Option<Notifier>),
    },
    NewIncomingMessage {
        peer_id: PeerId,
        message: Message,
    },
    Disconnected {
        peer_id: PeerId,
    },
}

impl<Si, St> From<(PeerId, Si, St)> for Event
where
    Si: Sink<Message, Error = conn::Error> + Send + 'static,
    St: Stream<Item = Message> + Send + 'static,
{
    fn from((peer_id, outgoing, incoming): (PeerId, Si, St)) -> Self {
        Event::ConnectionEstablished {
            peer_id,
            incoming: Box::pin(incoming),
            outgoing: Box::pin(outgoing),
        }
    }
}

pub type Notifier = oneshot::Sender<Result<(), conn::Error>>;

enum ConnectState {
    Connected(mpsc::UnboundedSender<(Message, Option<Notifier>)>),
    Connecting {
        notify_on_connect: Vec<Notifier>,
        pending_messages: Vec<(Message, Option<Notifier>)>,
    },
}

impl Default for ConnectState {
    fn default() -> Self {
        ConnectState::Connecting {
            notify_on_connect: vec![],
            pending_messages: vec![],
        }
    }
}

pub struct EventHandler {
    peers: HashMap<PeerId, ConnectState>,
    sender: mpsc::Sender<Event>,
}

impl EventHandler {
    pub fn start() -> (impl Future, mpsc::Sender<Event>) {
        let (sender, mut receiver) = mpsc::channel(10);
        let mut handler = EventHandler {
            peers: HashMap::default(),
            sender: sender.clone(),
        };
        let fut = async move {
            while let Some(event) = receiver.next().await {
                handler.handle_event(event);
            }
        };

        (fut, sender)
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::OutgoingConnectionReq { peer_id, notify } => match self.maybe_connect(peer_id) {
                ConnectState::Connected(_) => info!(
                    "Already connected to {}. Ingorning connection request.",
                    peer_id
                ),
                ConnectState::Connecting {
                    notify_on_connect, ..
                } => {
                    if let Some(notify) = notify {
                        notify_on_connect.push(notify);
                    }
                }
            },
            Event::SendMessageReq { peer_id, message } => match self.maybe_connect(peer_id) {
                ConnectState::Connected(outgoing_queue) => {
                    debug!("Using existing connection to send message to {}", peer_id);
                    outgoing_queue.unbounded_send(message).unwrap();
                }
                ConnectState::Connecting {
                    pending_messages, ..
                } => {
                    debug!(
                        "Currently connecting to {}, so putting message in queue",
                        peer_id
                    );
                    pending_messages.push(message);
                }
            },
            Event::NewIncomingMessage { peer_id, message } => {
                info!("received message from {}: {}", peer_id, message.message);
            }
            Event::Disconnected { peer_id } => {
                let removed = self.peers.remove(&peer_id);
                match removed.is_some() {
                    true => info!("disconnected from {}", peer_id),
                    false => debug!("redundant disconnect event for {}", peer_id),
                }
            }
            Event::ConnectionEstablished {
                peer_id,
                incoming,
                outgoing,
            } => {
                self.spawn_outgoing_messages(peer_id, outgoing);
                self.spawn_incoming_messages(peer_id, incoming);
            }
            Event::ConnectionFailed { peer_id, reason } => {
                if let Some(ConnectState::Connecting {
                    mut notify_on_connect,
                    mut pending_messages,
                }) = self.peers.remove(&peer_id)
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

    fn maybe_connect(&mut self, peer_id: PeerId) -> &mut ConnectState {
        let mut sender = self.sender.clone();
        self.peers.entry(peer_id).or_insert_with(move || {
            tokio::spawn(async move {
                match crate::p2p::conn::connect(peer_id).await {
                    Ok((incoming, outgoing)) => {
                        info!("successfully connected to {}", peer_id);
                        sender
                            .send(Event::from((peer_id, incoming, outgoing)))
                            .await
                            .unwrap();
                    }
                    Err(e) => sender
                        .send(Event::ConnectionFailed { peer_id, reason: e })
                        .await
                        .unwrap(),
                }
            });
            ConnectState::default()
        })
    }

    fn spawn_outgoing_messages(&mut self, peer_id: PeerId, mut sink: PeerSink) {
        let connect_state = self.peers.entry(peer_id).or_default();
        let (sender, mut receiver) = mpsc::unbounded();
        if let ConnectState::Connecting {
            notify_on_connect,
            pending_messages,
        } = connect_state
        {
            for notifier in notify_on_connect.drain(0..) {
                let _ = notifier.send(Ok(()));
            }
            let message_sender = sender.clone();
            for pending_message in pending_messages.drain(0..) {
                // queue up pending messages
                message_sender.unbounded_send(pending_message).unwrap();
            }
        }

        tokio::spawn(async move {
            while let Some((message, notifier)) = receiver.next().await {
                let res = match sink.send(message).await {
                    Ok(_) => {
                        debug!("successfully sent message to {}", peer_id);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to send message to {}: {}", peer_id, e);
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

    fn spawn_incoming_messages(&self, peer_id: PeerId, mut stream: PeerStream) {
        let mut sender = self.sender.clone();
        let read_msg_loop = async move {
            while let Some(message) = stream.next().await {
                debug!("received message from {}", peer_id);
                sender
                    .send(Event::NewIncomingMessage { peer_id, message })
                    .await
                    .unwrap();
            }
            sender.send(Event::Disconnected { peer_id }).await.unwrap();
        };

        tokio::spawn(read_msg_loop);
    }
}
