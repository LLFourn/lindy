use futures::future;
use futures::io;
use futures::StreamExt;
use futures::TryFutureExt;
use futures::{Sink, SinkExt, Stream};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::convert::TryFrom;
use std::fmt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio_util::codec::LinesCodecError;
// use std::collections::HashMap;
// use std::sync::RwLock;
use crate::PeerId;
use anyhow::anyhow;
use secp256kfun::XOnly;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LinesCodec};

#[derive(Error, Debug, Clone, Copy)]
pub enum HandshakeError {
    #[error("remote key was invalid")]
    InvalidKey,
    #[error("handshake has an io error of kind {0:?}")]
    Io(io::ErrorKind),
}

impl From<io::Error> for HandshakeError {
    fn from(e: io::Error) -> Self {
        HandshakeError::Io(e.kind())
    }
}

#[derive(Clone, Debug, Copy, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Peer {
    pub addr: std::net::SocketAddr,
    pub key: XOnly,
}

impl core::str::FromStr for Peer {
    type Err = anyhow::Error;

    fn from_str(string: &str) -> Result<Self, anyhow::Error> {
        let mut segments = string.split('@');
        let key = XOnly::from_str(segments.next().unwrap())?;
        let addr =
            std::net::SocketAddr::from_str(segments.next().ok_or(anyhow!("missing remote key"))?)?;
        Ok(Self { addr, key })
    }
}

impl From<Peer> for String {
    fn from(peer: Peer) -> String {
        peer.to_string()
    }
}

impl TryFrom<String> for Peer {
    type Error = anyhow::Error;

    fn try_from(string: String) -> Result<Self, Self::Error> {
        use std::str::FromStr;
        Self::from_str(&string)
    }
}

impl Peer {
    pub fn id(&self) -> PeerId {
        self.key
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", &self.key, &self.addr)
    }
}

async fn recv_handshake(mut connection: TcpStream) -> Result<(Peer, TcpStream), HandshakeError> {
    let mut xonly = [0u8; 32];
    connection.read_exact(&mut xonly).await?;
    let remote_key = XOnly::from_bytes(xonly).ok_or(HandshakeError::InvalidKey)?;

    let peer = Peer {
        addr: connection.peer_addr()?,
        key: remote_key,
    };

    Ok((peer, connection))
}

async fn send_handshake(
    mut connection: TcpStream,
    local_key: &XOnly,
) -> Result<TcpStream, HandshakeError> {
    connection.write_all(local_key.as_bytes()).await?;
    Ok(connection)
}

async fn frame_connection<M: DeserializeOwned + Serialize>(
    peer: Peer,
    connection: TcpStream,
) -> Result<(impl Sink<M, Error = Error>, impl Stream<Item = M>), Error> {
    let framed = Framed::new(connection, LinesCodec::new());
    let (sink, stream) = framed.split();

    let stream = stream
        .take_while(move |res| {
            if let Err(e) = res {
                error!("got error in socket from {}: {}", peer, e);
            }
            future::ready(res.is_ok())
        })
        .map(|res| res.unwrap())
        .map(|line| serde_json::from_str::<M>(&line))
        .take_while(move |res| {
            if let Err(e) = res {
                error!("got invalid JSON from {}: {}", peer, e);
            }
            future::ready(res.is_ok())
        })
        .map(|res| res.unwrap());

    let sink = sink
        .with(async move |message: M| -> Result<String, Error> {
            Ok(serde_json::to_string(&message).unwrap())
        })
        .sink_map_err(Into::into);

    Ok((sink, stream))
}

pub async fn listen<M: DeserializeOwned + Serialize + Send + 'static>(
    socket: TcpListener,
) -> anyhow::Result<
    impl Stream<
        Item = (
            Peer,
            impl Sink<M, Error = Error> + Send + 'static,
            impl Stream<Item = M> + Send + 'static,
        ),
    >,
> {
    let received_connections = socket.filter_map(async move |connection| match connection {
        Ok(connection) => {
            let peer_addr = connection.peer_addr().ok()?;
            let (peer, connection) = recv_handshake(connection)
                .inspect_err(|e| error!("error during handshake with {}: {}", peer_addr, e))
                .await
                .ok()?;
            info!("framed connection from {:?}", peer);
            match frame_connection(peer, connection).await {
                Ok((sender, receiver)) => {
                    info!("connected to {}", peer);
                    Some((peer, sender, receiver))
                }
                Err(e) => {
                    error!("failed to frame connection to {}: {}", peer, e);
                    None
                }
            }
        }
        Err(e) => {
            warn!("IO error while receiving connection {}", e);
            None
        }
    });

    Ok(received_connections)
}

#[derive(Clone, Debug, Error, Copy)]
pub enum Error {
    #[error("error during handshake: {0}")]
    Handshake(#[from] HandshakeError),
    #[error("{0:?}")]
    Io(io::ErrorKind),
    #[error("attempted to send message that was too long")]
    MessageTooLong,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e.kind())
    }
}

impl From<LinesCodecError> for Error {
    fn from(e: LinesCodecError) -> Self {
        match e {
            LinesCodecError::Io(e) => e.into(),
            LinesCodecError::MaxLineLengthExceeded => Error::MessageTooLong,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Connector {
    pub local_key: XOnly,
}

impl Connector {
    pub async fn connect<M: DeserializeOwned + Serialize + Send + 'static>(
        self,
        peer: Peer,
    ) -> Result<
        (
            Peer,
            impl Sink<M, Error = Error> + Send + 'static,
            impl Stream<Item = M> + Send + 'static,
        ),
        Error,
    > {
        let new_connection = TcpStream::connect(peer.addr).await?;
        let connection = send_handshake(new_connection, &self.local_key).await?;
        let (sender, receiver) = frame_connection(peer, connection).await?;
        Ok((peer, sender, receiver))
    }
}
