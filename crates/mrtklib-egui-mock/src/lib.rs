//! Mock of the `mrtk run` JSON-RPC server, for GUI development and tests.
//!
//! It speaks protocol 0.1 of MRTKLIB `docs/rpc/` (pinned copy: `contract/openrpc.json`):
//! the token handshake, the lifecycle methods, status and event polling. Positioning is
//! simulated: while running, it publishes one solution per interval around a fixed point.
//! No MRTKLIB code runs here; behavior that needs the real server (streams, configuration,
//! replayed data) is tested against `mrtk run` itself.

mod connection;
mod rpc;
mod solution;
mod state;

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use state::Shared;

/// How the mock behaves. The defaults suit manual GUI development.
#[derive(Clone, Debug)]
pub struct Config {
    /// Address to listen on.
    pub bind: IpAddr,
    /// Port to listen on; 0 lets the operating system choose.
    pub port: u16,
    /// Token that clients must present. `None` accepts any client, like `--rpc-no-auth`.
    pub token: Option<String>,
    /// Origins accepted from browsers. Requests without an `Origin` header are always accepted.
    pub allowed_origins: Vec<String>,
    /// Time from a lifecycle request to the end of its transition.
    pub transition: Duration,
    /// Interval between solutions while running.
    pub solution_interval: Duration,
    /// Make every start fail with `lastError`.
    pub fail_start: bool,
    /// Stop by itself with `stopReason: "inputEnd"` after this many solutions of a run.
    pub input_end_after: Option<u64>,
    /// Number of events held. The real server's capacity is not fixed; this is a mock value.
    pub event_capacity: usize,
    /// Maximum number of events per `getEvents`. A mock value, like `event_capacity`.
    pub page_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            token: None,
            allowed_origins: Vec::new(),
            transition: Duration::from_millis(300),
            solution_interval: Duration::from_secs(1),
            fail_start: false,
            input_end_after: None,
            event_capacity: 10_000,
            page_limit: 1000,
        }
    }
}

/// A running mock server. Dropping it ends the server like killing the process.
pub struct Running {
    url: String,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Running {
    /// Endpoint to connect to, in the form of the ready line (`ws://HOST:PORT/rpc`).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Block until a client's `shutdown` has been carried out.
    pub fn wait(mut self) {
        self.join();
    }

    fn join(&mut self) {
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shared.exit.store(true, Ordering::SeqCst);
        self.join();
    }
}

/// Start the mock: bind, then serve clients on background threads.
pub fn start(config: Config) -> io::Result<Running> {
    let listener = TcpListener::bind(SocketAddr::new(config.bind, config.port))?;
    listener.set_nonblocking(true)?;
    let url = endpoint_url(listener.local_addr()?);
    let shared = Arc::new(Shared::new(config));

    let accept = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || accept_loop(&listener, &shared))
    };
    let ticker = {
        let shared = Arc::clone(&shared);
        thread::spawn(move || state::ticker(&shared))
    };
    Ok(Running {
        url,
        shared,
        threads: vec![accept, ticker],
    })
}

/// The URL written in the ready line: a wildcard address becomes the loopback address,
/// because the reader is a process on the same machine.
fn endpoint_url(addr: SocketAddr) -> String {
    let host = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    format!("ws://{}/rpc", SocketAddr::new(host, addr.port()))
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    let mut connections = Vec::new();
    while !shared.exit.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = Arc::clone(shared);
                connections.push(thread::spawn(move || connection::serve(stream, &shared)));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                eprintln!("mrtklib-egui-mock: accept failed: {e}");
                thread::sleep(Duration::from_millis(20));
            }
        }
        connections.retain(|handle: &JoinHandle<()>| !handle.is_finished());
    }
    for handle in connections {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_addresses_become_loopback() {
        let v4: SocketAddr = "0.0.0.0:43215".parse().unwrap();
        let v6: SocketAddr = "[::]:43215".parse().unwrap();
        let specific: SocketAddr = "192.168.1.10:43215".parse().unwrap();
        assert_eq!(endpoint_url(v4), "ws://127.0.0.1:43215/rpc");
        assert_eq!(endpoint_url(v6), "ws://[::1]:43215/rpc");
        assert_eq!(endpoint_url(specific), "ws://192.168.1.10:43215/rpc");
    }
}
