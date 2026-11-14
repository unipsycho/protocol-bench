//! Embedded MQTT broker (rumqttd) — LAN listen with optional CONNECT users.
//!
//! Start runs accept on a dedicated thread; Stop cancels via oneshot so the
//! listener socket is dropped and the port frees for a later start.

use rumqttd::protocol::v4::V4;
use rumqttd::{ConnectionSettings, LinkType, Router, RouterConfig, Server, ServerSettings};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Live embedded broker handle (stop by dropping the accept loop).
pub struct EmbeddedBroker {
    listen: SocketAddr,
    /// Fires once to cancel `Server::start`.
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl EmbeddedBroker {
    pub fn listen_addr(&self) -> SocketAddr {
        self.listen
    }

    /// Bind and accept MQTT 3.1.1 on `bind:port`. Empty `users` → anonymous OK.
    pub fn start(bind: &str, port: u16, users: &[(String, String)]) -> Result<Self, String> {
        let host: std::net::IpAddr = bind
            .parse()
            .map_err(|e| format!("invalid bind address '{bind}': {e}"))?;
        let listen = SocketAddr::new(host, port);

        // Fail fast if the port is taken (clearer than rumqttd accept errors).
        {
            let probe = std::net::TcpListener::bind(listen)
                .map_err(|e| format!("listen {listen}: {e}"))?;
            drop(probe);
        }

        let auth = if users.is_empty() {
            None
        } else {
            let mut map = HashMap::new();
            for (u, p) in users {
                map.insert(u.clone(), p.clone());
            }
            Some(map)
        };

        let server_settings = ServerSettings {
            name: "pb-embedded".into(),
            listen,
            tls: None,
            next_connection_delay_ms: 1,
            connections: ConnectionSettings {
                connection_timeout_ms: 60_000,
                max_payload_size: 256 * 1024,
                max_inflight_count: 100,
                auth,
                external_auth: None,
                dynamic_filters: true,
            },
        };

        let router = Router::new(
            0,
            RouterConfig {
                max_connections: 1024,
                max_outgoing_packet_count: 200,
                max_segment_size: 1024 * 1024,
                max_segment_count: 4,
                custom_segment: None,
                initialized_filters: None,
                shared_subscriptions_strategy: Default::default(),
            },
        );
        let router_tx = router.spawn();
        let mut server = Server::new(server_settings, router_tx, V4);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let join = thread::Builder::new()
            .name("pb-mqtt-broker".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_) => return,
                };
                rt.block_on(async move {
                    tokio::select! {
                        _ = server.start(LinkType::Remote) => {}
                        _ = shutdown_rx => {}
                    }
                });
            })
            .map_err(|e| format!("spawn broker: {e}"))?;

        // Brief settle so bind owns the port before we return.
        thread::sleep(Duration::from_millis(40));
        if std::net::TcpStream::connect_timeout(&listen, Duration::from_millis(200)).is_err() {
            // Still starting or firewalled; keep running — connect smoke may retry.
        }

        Ok(Self {
            listen,
            shutdown: Some(shutdown_tx),
            join: Some(join),
        })
    }

    pub fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for EmbeddedBroker {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_stop_listens() {
        let port = 1891;
        let broker = EmbeddedBroker::start("127.0.0.1", port, &[]).expect("start");
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).expect("connect");
        broker.stop();
        // Port should free after stop.
        std::thread::sleep(Duration::from_millis(100));
        let _ = EmbeddedBroker::start("127.0.0.1", port, &[]).expect("restart");
    }
}
