use std::convert::Infallible;
use std::sync::Arc;

use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::SinkExt;
use tokio::sync::Mutex;
use warp::http;
use warp::Filter;
use warp::Rejection;

use super::api_reply::ApiReply;
use super::api_reply::ErrorMessage;
use crate::channel::rs_ecdsa::ChannelManager;
use crate::p2p::conn::Peer;
use crate::p2p::event::Event;
use crate::p2p::messages::Message;
use crate::ChannelId;
use crate::PeerId;

#[derive(serde::Deserialize, serde::Serialize)]
struct ChannelCreate {
    hello: String,
}

type NewPeerReq = Peer;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct NewPeerRes {
    id: PeerId,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Error {
    description: String,
}

impl<E: std::error::Error> From<E> for Error {
    fn from(e: E) -> Self {
        Error {
            description: format!("{}", e),
        }
    }
}

type CM = Arc<Mutex<ChannelManager>>;

pub fn routes(
    event_sender: mpsc::Sender<Event<Message>>,
    channel_manager: CM,
) -> impl Filter<Extract = impl warp::Reply, Error = Rejection> + Clone {
    let new_peer = warp::post()
        .and(require_this(event_sender.clone()))
        .and(warp::path("peers"))
        .and(warp::body::json::<NewPeerReq>())
        .and_then(connect);

    // let update_peer = warp::put()
    //     .and(require_this(event_sender.clone()))
    //     .and(warp::path("peers"))
    //     .and(warp::path::param())
    //     .and(warp::body::json::<Message>())
    //     .and_then(send_message);

    let post_channel = warp::post()
        .and(require_this(channel_manager.clone()))
        .and(warp::path("channels"))
        .and(warp::body::json::<NewChannelReq>())
        .and_then(open_channel);

    new_peer.or(post_channel)
}

fn require_this<T: Clone + Send + 'static>(
    thing: T,
) -> impl Filter<Extract = (T,), Error = Infallible> + Clone {
    warp::any().map(move || thing.clone())
}

async fn connect(
    mut sender: mpsc::Sender<Event<Message>>,
    new_peer: NewPeerReq,
) -> Result<ApiReply<NewPeerRes>, Infallible> {
    let (notifier, on_connect) = oneshot::channel();
    let _ = sender
        .send(Event::OutgoingConnectionReq {
            peer: new_peer,
            notify: Some(notifier),
        })
        .await;

    Ok(on_connect
        .await
        .map(|res| match res {
            Ok(_) => ApiReply::Created(
                format!("/{}", new_peer.id()),
                NewPeerRes { id: new_peer.id() },
            ),
            Err(e) => ApiReply::Err(ErrorMessage::from_status(
                http::StatusCode::FAILED_DEPENDENCY,
            )),
        })
        //TOOD: Fix unwrap
        .unwrap())
}

// async fn send_message(
//     mut sender: mpsc::Sender<Event<Message>>,
//     peer: Peer,
//     message: Message,
// ) -> Result<impl warp::Reply, Infallible> {
//     let (notifier, on_sent) = oneshot::channel();
//     let _ = sender
//         .send(Event::SendMessageReq {
//             peer,
//             message: (message, Some(notifier)),
//         })
//         .await;

//     Ok(on_sent
//         .await
//         .map(|_| warp::reply::with_status(warp::reply(), StatusCode::NO_CONTENT))
//         .unwrap_or_else(|_e| {
//             warp::reply::with_status(warp::reply(), StatusCode::FAILED_DEPENDENCY)
//         }))
// }

#[derive(serde::Deserialize, serde::Serialize)]
pub struct NewChannelReq {
    peer: Peer,
    value: u64,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct NewChannelRes {
    channel_id: ChannelId,
}

async fn open_channel(
    channel_manager: CM,
    new_channel: NewChannelReq,
) -> Result<ApiReply<NewChannelRes>, Infallible> {
    let mut channel_manager = channel_manager.lock().await;
    match channel_manager
        .new_channel(new_channel.value, new_channel.peer)
        .await
    {
        Ok(channel_id) => Ok(ApiReply::Ok(NewChannelRes { channel_id })),
        Err(e) => Ok(ApiReply::Err(ErrorMessage::internal_server_error())),
    }
}
