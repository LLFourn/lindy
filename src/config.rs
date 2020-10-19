use crate::seed::Seed;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub http: HttpConfig,
    pub lindy_p2p: LindyP2PConfig,
    pub seed: Seed,
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
