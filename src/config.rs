use crate::funder::Funder;
use crate::seed::Seed;
use bdk::bitcoin::Network;
use bdk::blockchain;
use bdk::blockchain::AnyBlockchain;
use bdk::blockchain::ConfigurableBlockchain;
use bdk::descriptor::ExtendedDescriptor;
use bdk::sled;
use serde::de::Error;
use serde::Deserialize;
use tokio::sync::Mutex;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub http: HttpConfig,
    pub lindy_p2p: LindyP2PConfig,
    pub wallet: Option<Wallet>,
    pub seed: Seed,
    pub blockchain: bdk::blockchain::AnyBlockchainConfig,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Wallet {
    #[serde(deserialize_with = "custom_serde::deserialize_descriptor")]
    pub descriptor: ExtendedDescriptor,
    pub database: String,
    // pub change_descriptor: Option<ExtendedDescriptor>,
}

impl Config {
    pub async fn get_funder(&self) -> anyhow::Result<Option<impl Funder>> {
        match &self.wallet {
            Some(wallet_config) => {
                let database = sled::open(wallet_config.database.as_str())?;
                let tree = database.open_tree("wallet")?;
                let blockchain = self.get_blockchain()?;
                let wallet = bdk::Wallet::new(
                    wallet_config.descriptor.clone(),
                    None,
                    Network::Regtest,
                    tree,
                    blockchain,
                )
                .await?;

                wallet.sync(blockchain::log_progress(), None).await?;

                Ok(Some(Mutex::new(wallet)))
            }
            None => Ok(None),
        }
    }

    pub fn get_blockchain(&self) -> anyhow::Result<AnyBlockchain> {
        Ok(AnyBlockchain::from_config(&self.blockchain)?)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    pub listen: std::net::SocketAddr,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LindyP2PConfig {
    pub listen: std::net::SocketAddr,
}

mod custom_serde {
    use super::*;
    pub fn deserialize_descriptor<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<ExtendedDescriptor, D::Error> {
        use std::str::FromStr;
        let string = String::deserialize(d)?;
        ExtendedDescriptor::from_str(&string).map_err(|e| D::Error::custom(format!("{}", e)))
    }
}
