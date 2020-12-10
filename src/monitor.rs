use std::sync::Arc;
use std::time::Duration;

use bdk::bitcoin::Transaction;
use bdk::bitcoin::Txid;
use bdk::blockchain::Blockchain;
use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::Future;
use futures::StreamExt;

#[derive(Clone)]
pub struct Monitor {
    sender: mpsc::UnboundedSender<Request>,
}

impl Monitor {
    pub async fn wait_for_tx(&self, txid: Txid) -> Transaction {
        let (responder, reciever) = oneshot::channel();
        self.sender
            .unbounded_send(Request::Transaction { txid, responder })
            .unwrap();
        reciever.await.unwrap()
    }
}

enum Request {
    Transaction {
        txid: Txid,
        responder: oneshot::Sender<Transaction>,
    },
}

pub fn start(
    blockchain: impl Blockchain + 'static,
) -> (impl Future<Output = ()> + 'static, Monitor) {
    let (sender, mut receiver) = mpsc::unbounded();
    let blockchain = Arc::new(blockchain);
    let local = tokio::task::LocalSet::new();
    local.spawn_local(async move {
        while let Some(request) = receiver.next().await {
            let blockchain = blockchain.clone();
            match request {
                Request::Transaction { txid, responder } => {
                    tokio::task::spawn_local(async move {
                        #[allow(unused_assignments)]
                        let mut tx = None;

                        while {
                            tx = blockchain.get_tx(&txid).await.unwrap_or(None);
                            tx.is_none()
                        } {
                            debug!("transaction not found {} yet", txid);
                            tokio::time::delay_for(Duration::from_secs(10)).await;
                        }

                        let _ = responder.send(tx.unwrap());
                    });
                }
            }
        }
    });

    (local, Monitor { sender })
}
