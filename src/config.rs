use crate::seed::Seed;
use bdk::descriptor::ExtendedDescriptor;
use serde::de::Error;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub http: HttpConfig,
    pub lindy_p2p: LindyP2PConfig,
    pub wallet: Option<Wallet>,
    pub seed: Seed,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Wallet {
    #[serde(deserialize_with = "custom_serde::deserialize_descriptor")]
    pub descritor: ExtendedDescriptor,
    // pub change_descriptor: Option<ExtendedDescriptor>,
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
