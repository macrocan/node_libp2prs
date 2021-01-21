// Copyright 2020 Netwarps Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use std::time::Duration;
#[macro_use]
extern crate lazy_static;

use libp2p_rs::core::identity::Keypair;
use libp2p_rs::core::transport::upgrade::TransportUpgrade;
use libp2p_rs::core::upgrade::Selector;
use libp2p_rs::core::{Multiaddr, PeerId};
use libp2p_rs::yamux;
use libp2p_rs::secio;
use libp2p_rs::noise::{Keypair as NKeypair, NoiseConfig, X25519Spec};
use libp2p_rs::swarm::identify::IdentifyConfig;
use libp2p_rs::swarm::Swarm;
use libp2p_rs::runtime::task;
use libp2p_rs::tcp::TcpConfig;
use libp2p_rs::mplex;
use xcli::App;
use std::convert::TryFrom;
use libp2p_rs::kad::kad::{Kademlia, KademliaConfig};
use libp2p_rs::kad::store::MemoryStore;
use libp2p_rs::kad::cli::dht_cli_commands;
use libp2p_rs::swarm::cli::swarm_cli_commands;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if std::env::args().len() == 3 {
        log::info!("Starting server...");
        let a1 = std::env::args().nth(1).unwrap();
        let a2 = std::env::args().nth(2).unwrap();

        let peer = match PeerId::try_from(a1) {
            Ok(peer) => peer,
            Err(e) => {
                println!("bad peer id: {:?}", e);
                return;
            }
        };
        let addr = match Multiaddr::try_from(a2) {
            Ok(addr) => addr,
            Err(e) => {
                println!("bad multiaddr: {:?}", e);
                return;
            }
        };

        run_server(peer, addr);
    } else {
        println!("Usage: {} <bootstrap-address>", std::env::args().next().unwrap());
    }
}

lazy_static! {
    static ref SERVER_KEY: Keypair = Keypair::generate_ed25519_fixed();
}

#[allow(clippy::empty_loop)]
fn run_server(bootstrap_peer: PeerId, bootstrap_addr: Multiaddr) {
    let keys = SERVER_KEY.clone();

    let listen_addr1: Multiaddr = "/ip4/0.0.0.0/tcp/8086".parse().unwrap();

    let dh = NKeypair::<X25519Spec>::new().into_authentic(&keys).unwrap();

    let sec_noise = NoiseConfig::xx(dh, keys.clone());
    let sec_secio = secio::Config::new(keys.clone());
    let sec = Selector::new(sec_noise, sec_secio);

    let mux = Selector::new(yamux::Config::new(), mplex::Config::new());
    let tu = TransportUpgrade::new(TcpConfig::default(), mux, sec);

    let mut swarm = Swarm::new(keys.public())
        .with_transport(Box::new(tu))
        .with_identify(IdentifyConfig::new(false));

    log::info!("Swarm created, local-peer-id={:?}", swarm.local_peer_id());

    task::block_on(async {
        let mut swarm_control = swarm.control();
        swarm.listen_on(vec![listen_addr1]).unwrap();

        // build Kad
        let config = KademliaConfig::default()
            .with_query_timeout(Duration::from_secs(90));

        let store = MemoryStore::new(swarm.local_peer_id().clone());
        let kad = Kademlia::with_config(swarm.local_peer_id().clone(), store, config);
        let mut kad_control = kad.control();

        // update Swarm to support Kad and Routing
        swarm = swarm.with_protocol(Box::new(kad.handler())).with_routing(Box::new(kad.control()));

        kad.start(swarm_control.clone());
        swarm.start();

        kad_control.add_node(bootstrap_peer, vec![bootstrap_addr]).await;
        kad_control.bootstrap().await;

        let mut app = App::new("xCLI").version("v0.1").author("kingwel.xie@139.com");

        app.add_subcommand_with_userdata(swarm_cli_commands(), Box::new(swarm_control.clone()));
        app.add_subcommand_with_userdata(dht_cli_commands(), Box::new(kad_control.clone()));

        app.run();

        kad_control.close().await;
        swarm_control.close().await;

        task::sleep(Duration::from_secs(1)).await;
    });
}