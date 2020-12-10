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
use crate::channel::ChannelDb;
use crate::channel::ChannelId;
use crate::http::messages::*;
use crate::p2p::conn::Peer;
use crate::p2p::event::Event;
use crate::p2p::messages::Message;

type CM = Arc<Mutex<ChannelManager>>;
type Db = Arc<dyn ChannelDb>;

pub fn routes(
    event_sender: mpsc::Sender<Event<Message>>,
    channel_manager: CM,
    channel_db: Db,
    me: Peer,
) -> impl Filter<Extract = impl warp::Reply, Error = Rejection> + Clone {
    let get_root = warp::get().and(warp::path::end()).and_then(
        async move || -> Result<ApiReply<RootInfo>, Infallible> {
            Ok(ApiReply::Ok(RootInfo { me }))
        },
    );

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
        .and(require_this(event_sender.clone()))
        .and(warp::path("channels"))
        .and(warp::body::json::<NewChannelReq>())
        .and_then(open_channel);

    let get_channels = warp::get()
        .and(require_this(channel_db.clone()))
        .and(warp::path("channels"))
        .and(warp::path::end())
        .and_then(list_channels);

    let get_channel = warp::get()
        .and(require_this(channel_db.clone()))
        .and(warp::path("channels"))
        .and(warp::path::param::<ChannelId>())
        .and_then(channel_info);

    let put_channel = warp::put()
        .and(require_this(channel_manager.clone()))
        .and(warp::path("channels"))
        .and(warp::path::param::<ChannelId>())
        .and(warp::body::json::<ChannelUpdate>())
        .and_then(update_channel);

    get_root
        .or(post_channel)
        .or(get_channels)
        .or(get_channel)
        .or(new_peer)
        .or(put_channel)
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
            Err(_) => ApiReply::Err(ErrorMessage::from_status(
                http::StatusCode::FAILED_DEPENDENCY,
            )),
        })
        //TOOD: Fix unwrap
        .unwrap_or(ApiReply::Err(ErrorMessage::internal_server_error())))
}

async fn open_channel(
    channel_manager: CM,
    mut event_sender: mpsc::Sender<Event<Message>>,
    new_channel: NewChannelReq,
) -> Result<ApiReply<NewChannelRes>, Infallible> {
    let mut channel_manager = channel_manager.lock().await;
    match channel_manager
        .new_channel(new_channel.value, new_channel.peer.id())
        .await
    {
        Ok(new_channel_msg) => {
            let channel_id = new_channel_msg.id;

            Ok(
                match Event::send_message(
                    &mut event_sender,
                    new_channel.peer,
                    Message::RsEcdsaNewChannel(new_channel_msg),
                )
                .await
                {
                    Ok(()) => ApiReply::Ok(NewChannelRes { channel_id }),
                    Err(e) => ApiReply::Err(
                        ErrorMessage::from_status(http::StatusCode::FAILED_DEPENDENCY)
                            .with_message(&e.to_string()),
                    ),
                },
            )
        }
        Err(e) => Ok(match e.downcast::<bdk::Error>() {
            Ok(e) => ApiReply::Err(
                ErrorMessage::from_status(http::StatusCode::BAD_REQUEST)
                    .with_message(e.to_string()),
            ),
            Err(_) => ApiReply::Err(ErrorMessage::internal_server_error()),
        }),
    }
}

async fn channel_info(
    channel_db: Db,
    channel_id: ChannelId,
) -> Result<ApiReply<GetChannel>, Infallible> {
    match channel_db.get_channel(channel_id).await {
        Ok(Some(channel)) => Ok(ApiReply::Ok(GetChannel { channel })),
        Ok(None) => Ok(ApiReply::Err(ErrorMessage::not_found())),
        Err(e) => {
            error!("{}", e);
            Ok(ApiReply::Err(ErrorMessage::internal_server_error()))
        }
    }
}

async fn list_channels(channel_db: Db) -> anyhow::Result<ApiReply<GetChannels>, Infallible> {
    match channel_db.list_channels().await {
        Ok(channels) => Ok(ApiReply::Ok(GetChannels {
            channels: channels.into_iter().map(|channel| channel.id).collect(),
        })),
        Err(e) => {
            error!("{}", e);
            Ok(ApiReply::Err(ErrorMessage::internal_server_error()))
        }
    }
}

async fn update_channel(
    channel_manager: CM,
    channel_id: ChannelId,
    update: ChannelUpdate,
) -> Result<ApiReply<ForceCloseChannelRes>, Infallible> {
    let channel_manager = channel_manager.lock().await;
    match update {
        ChannelUpdate::ForceClose {} => {
            match channel_manager.force_close_channel(channel_id).await {
                Ok(tx) => Ok(ApiReply::Ok(ForceCloseChannelRes { tx })),
                Err(e) => Ok(ApiReply::Err(
                    ErrorMessage::internal_server_error().with_message(e.to_string()),
                )),
            }
        }
    }
}
// async fn get_channel(
//     channel_manager: CM,
// ) -> Result<ApiReply<>>
