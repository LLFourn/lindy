use crate::bitcoin::util::psbt::PartiallySignedTransaction as Psbt;
use crate::channel::ChannelId;
use async_trait::async_trait;
use bdk::bitcoin::OutPoint;
use bdk::bitcoin::PublicKey;
use bdk::bitcoin::Transaction;
use bdk::blockchain::Blockchain;
use bdk::database::BatchDatabase;
use bdk::descriptor::Descriptor;
use bdk::TxBuilder;
use tokio::sync::Mutex;

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

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub struct FundTx {
    pub descriptor: Descriptor<PublicKey>,
    pub psbt: Psbt,
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

    pub fn output_value(&self) -> u64 {
        self.psbt.global.unsigned_tx.output[self.channel_id().vout as usize].value
    }

    pub fn into_psbt(self) -> Psbt {
        self.psbt
    }

    pub fn matches(&self, tx: &Transaction) -> bool {
        let channel_id = self.channel_id();
        match tx.output.get(channel_id.vout as usize) {
            Some(output) => {
                tx.txid() == channel_id.txid
                    && output.value == self.output_value()
                    && output.script_pubkey == self.descriptor.script_pubkey()
            }
            None => false,
        }
    }
}

// impl<B, D> Funder for bdk::Wallet<B,D> {
//     fn create_fund_transaction(&self,  value: u64, descriptor: ExtendedDescriptor) -> Psbt {
//         todo!()
//     }
// }
