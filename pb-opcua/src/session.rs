//! Outbound OPC UA client session (tokio worker + mpsc events).

use opcua::client::{ClientBuilder, DataChangeCallback, IdentityToken, Session};
use opcua::crypto::SecurityPolicy;
use opcua::types::{
    BrowseDescription, BrowseDescriptionResultMask, BrowseDirection, MessageSecurityMode,
    MonitoredItemCreateRequest, NodeClass, NodeClassMask, NodeId, TimestampsToReturn,
    UserTokenPolicy,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Events drained by the pack poll loop.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    State { connected: bool, detail: String },
    Browse {
        nodes: Vec<BrowseNode>,
    },
    Data {
        node_id: String,
        value: String,
    },
    Error { detail: String },
}

#[derive(Debug, Clone)]
pub struct BrowseNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub is_variable: bool,
}

enum Cmd {
    Browse { node_id: String },
    Subscribe { node_ids: Vec<String>, interval_ms: u64 },
    Read { node_ids: Vec<String> },
    Stop,
}

/// Live client session running on a dedicated OS thread + tokio runtime.
pub struct OpcUaSession {
    cmd_tx: Sender<Cmd>,
    ev_rx: Receiver<SessionEvent>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl OpcUaSession {
    pub fn spawn(
        endpoint_url: String,
        security_policy: String,
        security_mode: String,
        trust_server: bool,
        pki_dir: String,
        username: String,
        password: String,
    ) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (ev_tx, ev_rx) = mpsc::channel::<SessionEvent>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_c = Arc::clone(&stop);

        let join = thread::Builder::new()
            .name("pb-opcua-client".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ev_tx.send(SessionEvent::Error {
                            detail: format!("tokio runtime: {e}"),
                        });
                        return;
                    }
                };
                rt.block_on(async move {
                    run_session(
                        endpoint_url,
                        security_policy,
                        security_mode,
                        trust_server,
                        pki_dir,
                        username,
                        password,
                        cmd_rx,
                        ev_tx,
                        stop_c,
                    )
                    .await;
                });
            })
            .map_err(|e| format!("spawn client: {e}"))?;

        Ok(Self {
            cmd_tx,
            ev_rx,
            stop,
            join: Some(join),
        })
    }

    pub fn try_recv(&self) -> Result<SessionEvent, TryRecvError> {
        self.ev_rx.try_recv()
    }

    pub fn browse(&self, node_id: &str) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Browse {
                node_id: node_id.to_string(),
            })
            .map_err(|_| "client session closed".into())
    }

    pub fn subscribe(&self, node_ids: Vec<String>, interval_ms: u64) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Subscribe {
                node_ids,
                interval_ms,
            })
            .map_err(|_| "client session closed".into())
    }

    pub fn read_poll(&self, node_ids: Vec<String>) -> Result<(), String> {
        self.cmd_tx
            .send(Cmd::Read { node_ids })
            .map_err(|_| "client session closed".into())
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.cmd_tx.send(Cmd::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for OpcUaSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.cmd_tx.send(Cmd::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

async fn run_session(
    endpoint_url: String,
    security_policy: String,
    security_mode: String,
    trust_server: bool,
    pki_dir: String,
    username: String,
    password: String,
    cmd_rx: Receiver<Cmd>,
    ev_tx: Sender<SessionEvent>,
    stop: Arc<AtomicBool>,
) {
    let _ = ev_tx.send(SessionEvent::State {
        connected: false,
        detail: "connecting".into(),
    });

    let policy = parse_policy(&security_policy);
    let mode = parse_mode(&security_mode);
    let identity = if username.trim().is_empty() {
        IdentityToken::Anonymous
    } else {
        IdentityToken::UserName(username, opcua::client::Password::new(password))
    };

    let mut builder = ClientBuilder::new()
        .application_name("Protocol Bench")
        .application_uri("urn:protocol-bench:opcua-client")
        .product_uri("urn:protocol-bench")
        .trust_server_certs(trust_server)
        .create_sample_keypair(true)
        .session_retry_limit(3);
    if !pki_dir.trim().is_empty() {
        builder = builder.pki_dir(pki_dir);
    }
    let mut client = match builder.client() {
        Ok(c) => c,
        Err(errs) => {
            let _ = ev_tx.send(SessionEvent::Error {
                detail: format!(
                    "client config: {}. Checklist: PKI dir writable; auto-cert enabled.",
                    errs.join("; ")
                ),
            });
            return;
        }
    };

    let endpoint = (
        endpoint_url.as_str(),
        policy.to_str(),
        mode,
        UserTokenPolicy::anonymous(),
    );
    let (session, event_loop) = match client
        .connect_to_matching_endpoint(endpoint, identity)
        .await
    {
        Ok(pair) => pair,
        Err(e) => {
            let _ = ev_tx.send(SessionEvent::Error {
                detail: format!(
                    "connect failed: {e}. Checklist: endpoint reachable (opc.tcp), \
                     security mode/policy match, cert trusted (or trust_once)."
                ),
            });
            return;
        }
    };
    let handle = event_loop.spawn();
    session.wait_for_connection().await;
    let _ = ev_tx.send(SessionEvent::State {
        connected: true,
        detail: "connected".into(),
    });

    // Initial Objects folder browse so the panel has a tree seed.
    if let Ok(nodes) = browse_children(&session, &NodeId::objects_folder_id()).await {
        let _ = ev_tx.send(SessionEvent::Browse { nodes });
    }

    let mut poll_nodes: Vec<String> = Vec::new();
    let mut poll_every = Duration::from_millis(500);
    let mut last_poll = tokio::time::Instant::now();

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        // Drain commands without blocking the event loop forever.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Stop => {
                    stop.store(true, Ordering::SeqCst);
                }
                Cmd::Browse { node_id } => {
                    let nid = parse_node_id(&node_id).unwrap_or_else(NodeId::objects_folder_id);
                    match browse_children(&session, &nid).await {
                        Ok(nodes) => {
                            let _ = ev_tx.send(SessionEvent::Browse { nodes });
                        }
                        Err(e) => {
                            let _ = ev_tx.send(SessionEvent::Error {
                                detail: format!("browse: {e}"),
                            });
                        }
                    }
                }
                Cmd::Subscribe {
                    node_ids,
                    interval_ms,
                } => {
                    poll_every = Duration::from_millis(interval_ms.max(100));
                    match subscribe_nodes(&session, &node_ids, interval_ms, ev_tx.clone()).await {
                        Ok(()) => {
                            poll_nodes = node_ids;
                        }
                        Err(e) => {
                            // Subscription unsupported → poll fallback.
                            let _ = ev_tx.send(SessionEvent::State {
                                connected: true,
                                detail: format!("subscribe failed ({e}); using poll fallback"),
                            });
                            poll_nodes = node_ids;
                            let _ = read_and_emit(&session, &poll_nodes, &ev_tx).await;
                        }
                    }
                }
                Cmd::Read { node_ids } => {
                    poll_nodes = node_ids;
                    let _ = read_and_emit(&session, &poll_nodes, &ev_tx).await;
                }
            }
        }
        if !poll_nodes.is_empty() && last_poll.elapsed() >= poll_every {
            last_poll = tokio::time::Instant::now();
            let _ = read_and_emit(&session, &poll_nodes, &ev_tx).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let _ = session.disconnect().await;
    let _ = handle.await;
    let _ = ev_tx.send(SessionEvent::State {
        connected: false,
        detail: "disconnected".into(),
    });
}

async fn browse_children(
    session: &Arc<Session>,
    node_id: &NodeId,
) -> Result<Vec<BrowseNode>, String> {
    let desc = BrowseDescription {
        node_id: node_id.clone(),
        browse_direction: BrowseDirection::Forward,
        reference_type_id: NodeId::null(),
        include_subtypes: true,
        node_class_mask: (NodeClassMask::OBJECT | NodeClassMask::VARIABLE | NodeClassMask::METHOD)
            .bits(),
        result_mask: BrowseDescriptionResultMask::all().bits(),
    };
    let results = session
        .browse(&[desc], 0, None)
        .await
        .map_err(|e| format!("{e}"))?;
    let mut out = Vec::new();
    if let Some(page) = results.first() {
        if let Some(refs) = &page.references {
            for r in refs {
                let is_variable = r.node_class == NodeClass::Variable;
                out.push(BrowseNode {
                    node_id: r.node_id.node_id.to_string(),
                    browse_name: r.browse_name.name.to_string(),
                    display_name: r.display_name.text.to_string(),
                    is_variable,
                });
            }
        }
    }
    Ok(out)
}

async fn subscribe_nodes(
    session: &Arc<Session>,
    node_ids: &[String],
    interval_ms: u64,
    ev_tx: Sender<SessionEvent>,
) -> Result<(), String> {
    let subscription_id = session
        .create_subscription(
            Duration::from_millis(interval_ms.max(100)),
            10,
            30,
            0,
            0,
            true,
            DataChangeCallback::new(move |dv, item| {
                let nid = item.item_to_monitor().node_id.to_string();
                let value = match &dv.value {
                    Some(v) => format!("{v:?}"),
                    None => dv
                        .status
                        .as_ref()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "null".into()),
                };
                let _ = ev_tx.send(SessionEvent::Data {
                    node_id: nid,
                    value,
                });
            }),
        )
        .await
        .map_err(|e| format!("{e}"))?;
    let items: Vec<MonitoredItemCreateRequest> = node_ids
        .iter()
        .filter_map(|s| parse_node_id(s))
        .map(|n| n.into())
        .collect();
    if items.is_empty() {
        return Err("no valid NodeIds".into());
    }
    session
        .create_monitored_items(subscription_id, TimestampsToReturn::Both, items)
        .await
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

async fn read_and_emit(
    session: &Arc<Session>,
    node_ids: &[String],
    ev_tx: &Sender<SessionEvent>,
) -> Result<(), String> {
    use opcua::types::{AttributeId, ReadValueId};
    let nodes: Vec<ReadValueId> = node_ids
        .iter()
        .filter_map(|s| parse_node_id(s))
        .map(|node_id| ReadValueId {
            node_id,
            attribute_id: AttributeId::Value as u32,
            ..Default::default()
        })
        .collect();
    if nodes.is_empty() {
        return Ok(());
    }
    let values = session
        .read(&nodes, TimestampsToReturn::Both, 0.0)
        .await
        .map_err(|e| format!("{e}"))?;
    for (req, dv) in nodes.iter().zip(values.iter()) {
        let value = match &dv.value {
            Some(v) => format!("{v:?}"),
            None => "null".into(),
        };
        let _ = ev_tx.send(SessionEvent::Data {
            node_id: req.node_id.to_string(),
            value,
        });
    }
    Ok(())
}

fn parse_policy(raw: &str) -> SecurityPolicy {
    match raw.trim().to_ascii_lowercase().as_str() {
        "basic256sha256" => SecurityPolicy::Basic256Sha256,
        "basic256" => SecurityPolicy::Basic256,
        "none" | "" => SecurityPolicy::None,
        _ => SecurityPolicy::None,
    }
}

fn parse_mode(raw: &str) -> MessageSecurityMode {
    match raw.trim().to_ascii_lowercase().replace('_', "").as_str() {
        "signandencrypt" | "sign&encrypt" => MessageSecurityMode::SignAndEncrypt,
        "sign" => MessageSecurityMode::Sign,
        _ => MessageSecurityMode::None,
    }
}

/// Accept `ns=2;s=foo`, `i=85`, bare strings (as s=), or Objects folder.
pub fn parse_node_id(raw: &str) -> Option<NodeId> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("objects") {
        return Some(NodeId::objects_folder_id());
    }
    if let Ok(n) = s.parse::<NodeId>() {
        return Some(n);
    }
    // Bare string → ns=2;s=<id> convention used by many demos; host binds prefer full form.
    Some(NodeId::new(2, s))
}
