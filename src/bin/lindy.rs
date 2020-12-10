#![feature(async_closure)]
use anyhow::Context;
use lindy::Config;
use std::path::PathBuf;
use structopt::StructOpt;

#[macro_use]
extern crate log;

#[derive(Debug, StructOpt)]
#[structopt(name = "lindy")]
pub struct Opt {
    #[structopt(short = "c", long, parse(from_os_str), name = "toml config file")]
    pub config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let opt = Opt::from_args();

    let config: Config = {
        use std::{fs::File, io::Read};
        let mut file = File::open(opt.config.clone())?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        toml::from_str(&content)
            .context(format!("Failed to parse {}", opt.config.to_str().unwrap()))?
    };
    let funder = config.get_funder().await?.expect("we need a funder atm");

    info!("starting p2p on: {}", config.lindy_p2p.listen);
    let p2p_listener = tokio::net::TcpListener::bind(config.lindy_p2p.listen).await?;
    info!("starting http on: {}", config.http.listen);
    let http_listener = tokio::net::TcpListener::bind(config.http.listen).await?;

    let blockchain = config.get_blockchain()?;

    lindy::server_bundle::start(
        config.seed.clone(),
        p2p_listener,
        http_listener,
        funder,
        blockchain,
    )
    .await?;

    Ok(())
}
