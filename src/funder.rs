use crate::bitcoin::util::psbt::PartiallySignedTransaction;
use crate::ChannelId;
use bdk::bitcoin::OutPoint;
use bdk::bitcoin::PublicKey;
use bdk::blockchain::Blockchain;
use bdk::database::BatchDatabase;
use bdk::descriptor::Descriptor;
use tokio::sync::Mutex;
type Psbt = PartiallySignedTransaction;
use async_trait::async_trait;
use bdk::TxBuilder;

#[async_trait]
pub trait Funder: Send + Sync + 'static {
    async fn create_fund_transaction(
        &self,
        value: u64,
        descriptor: Descriptor<PublicKey>,
    ) -> anyhow::Result<FundTx>;
}

#[async_trait]
impl<B: Blockchain + Send + 'static, D: BatchDatabase + Send + 'static> Funder
    for Mutex<bdk::Wallet<B, D>>
{
    async fn create_fund_transaction(
        &self,
        value: u64,
        descriptor: Descriptor<PublicKey>,
    ) -> anyhow::Result<FundTx> {
        let wallet = self.lock().await;
        let (psbt, _tx_details) =
            wallet.create_tx(TxBuilder::new().add_recipient(descriptor.script_pubkey(), value))?;
        Ok(FundTx { descriptor, psbt })
    }
}

pub struct FundTx {
    descriptor: Descriptor<PublicKey>,
    psbt: Psbt,
}

impl FundTx {
    pub fn channel_id(&self) -> ChannelId {
        let script_pubkey = self.descriptor.script_pubkey();
        let i = self
            .psbt
            .global
            .unsigned_tx
            .output
            .iter()
            .enumerate()
            .find_map(|(i, o)| {
                if o.script_pubkey == script_pubkey {
                    Some(i)
                } else {
                    None
                }
            })
            .unwrap();
        OutPoint {
            txid: self.psbt.clone().extract_tx().txid(),
            vout: i as u32,
        }
    }
}

// impl<B, D> Funder for bdk::Wallet<B,D> {
//     fn create_fund_transaction(&self,  value: u64, descriptor: ExtendedDescriptor) -> Psbt {
//         todo!()
//     }
// }
