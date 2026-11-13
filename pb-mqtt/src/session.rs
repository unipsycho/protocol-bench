//! rumqttc worker thread: connect, auto-reconnect unless stopped, feed events.
//!
//! Pattern from TopicBench `src/mqtt/live.rs` (EventLoop worker only — no UI).

use rumqttc::{Client, Event, MqttOptions, Packet, QoS, Transport};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const MIN_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Tcp,
    Tls,
    Ws,
    Wss,
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    State { connected: bool, detail: String },
    Message { topic: String, payload: Vec<u8> },
    Error { detail: String },
}

pub struct MqttSession {
    client: Client,
    stop: Arc<AtomicBool>,
    rx: Receiver<SessionEvent>,
}

impl MqttSession {
    pub fn spawn(
        host: String,
        port: u16,
        client_id: String,
        username: String,
        password: String,
        transport: TransportKind,
        keep_alive_secs: u64,
    ) -> Result<Self, String> {
        let mut opts = MqttOptions::new(&client_id, &host, port);
        opts.set_keep_alive(Duration::from_secs(keep_alive_secs.max(1)));
        opts.set_clean_session(true);
        opts.set_pending_throttle(Duration::from_secs(1));
        if !username.is_empty() {
            opts.set_credentials(&username, &password);
        }
        opts.set_transport(build_transport(transport)?);

        let (client, mut connection) = Client::new(opts, 1024);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_w = Arc::clone(&stop);
        let (tx, rx) = mpsc::channel();
        let endpoint = format!("{host}:{port}");

        let _ = tx.send(SessionEvent::State {
            connected: false,
            detail: format!("Connecting to {endpoint}…"),
        });

        thread::Builder::new()
            .name("pb-mqtt-worker".into())
            .spawn(move || {
                worker_loop(&mut connection, tx, stop_w, endpoint);
            })
            .map_err(|e| format!("spawn worker: {e}"))?;

        Ok(Self { client, stop, rx })
    }

    pub fn try_recv(&self) -> Result<SessionEvent, TryRecvError> {
        self.rx.try_recv()
    }

    pub fn subscribe(&self, topic: &str, qos: u8) -> Result<(), String> {
        self.client
            .subscribe(topic, to_qos(qos))
            .map_err(|e| format!("subscribe: {e}"))
    }

    pub fn publish(&self, topic: &str, payload: &[u8], qos: u8, retain: bool) -> Result<(), String> {
        self.client
            .publish(topic, to_qos(qos), retain, payload)
            .map_err(|e| format!("publish: {e}"))
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.client.disconnect();
    }
}

fn build_transport(kind: TransportKind) -> Result<Transport, String> {
    Ok(match kind {
        TransportKind::Tcp => Transport::Tcp,
        TransportKind::Tls => Transport::tls_with_default_config(),
        TransportKind::Ws => Transport::Ws,
        TransportKind::Wss => Transport::wss_with_default_config(),
    })
}

fn to_qos(qos: u8) -> QoS {
    match qos {
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

fn worker_loop(
    connection: &mut rumqttc::Connection,
    tx: Sender<SessionEvent>,
    stop: Arc<AtomicBool>,
    endpoint: String,
) {
    let mut backoff = MIN_BACKOFF;

    for notification in connection.iter() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match notification {
            Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if ack.code == rumqttc::ConnectReturnCode::Success {
                    backoff = MIN_BACKOFF;
                    let _ = tx.send(SessionEvent::State {
                        connected: true,
                        detail: format!("Connected to {endpoint}"),
                    });
                } else {
                    let _ = tx.send(SessionEvent::State {
                        connected: false,
                        detail: format!(
                            "Connection refused: {:?} (retry in {}s)",
                            ack.code,
                            backoff.as_secs().max(1)
                        ),
                    });
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(backoff);
                    backoff = (backoff.saturating_mul(2)).min(MAX_BACKOFF);
                }
            }
            Ok(Event::Incoming(Packet::Publish(p))) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = tx.send(SessionEvent::Message {
                    topic: p.topic,
                    payload: p.payload.to_vec(),
                });
            }
            Ok(Event::Incoming(Packet::Disconnect)) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = tx.send(SessionEvent::State {
                    connected: false,
                    detail: "Broker sent disconnect".into(),
                });
            }
            Ok(_) => {}
            Err(e) => {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let _ = tx.send(SessionEvent::State {
                    connected: false,
                    detail: format!(
                        "Connection error: {e} (retry in {}s)",
                        backoff.as_secs().max(1)
                    ),
                });
                thread::sleep(backoff);
                backoff = (backoff.saturating_mul(2)).min(MAX_BACKOFF);
            }
        }
    }

    if stop.load(Ordering::SeqCst) {
        let _ = tx.send(SessionEvent::State {
            connected: false,
            detail: "Disconnected by user".into(),
        });
    } else {
        let _ = tx.send(SessionEvent::Error {
            detail: format!("MQTT worker exited ({endpoint})"),
        });
    }
}
