#![feature(async_closure)]
use anyhow::Context;
use futures::StreamExt;
use lindy::keychain::KeyChain;
use lindy::p2p::conn::Connector;
use lindy::p2p::event::Event;
use lindy::p2p::event::EventHandler;
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

    let keychain = KeyChain::new(config.seed);
    let local_key = keychain.node_keypair().public_key.to_xonly();

    let connector = Connector { local_key };

    let (event_handler_loop, event_sender) =
        EventHandler::start(connector, async move |peer_id, message: lindy::p2p::messages::Message| {
            info!("got message from {}: {:?}", peer_id, message);
        });

    info!("starting p2p on: {}", config.lindy_p2p.listen);

    let p2p_listen_loop = lindy::p2p::conn::listen(local_key, config.lindy_p2p.listen)
        .await?
        .map(|item| Ok(Event::from(item)))
        .forward(event_sender.clone());

    let routes = lindy::http_api::routes(event_sender);
    let http_listener = warp::serve(routes).run(config.http.listen);

    let _ = futures::join!(event_handler_loop, http_listener, p2p_listen_loop);

    Ok(())
}
