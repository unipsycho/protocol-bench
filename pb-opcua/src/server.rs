//! Embedded OPC UA server hub — exposes host points as Variable nodes.

use opcua::server::address_space::Variable;
use opcua::server::diagnostics::NamespaceMetadata;
use opcua::server::node_manager::memory::{
    simple_node_manager, InMemoryNodeManager, SimpleNodeManager, SimpleNodeManagerImpl,
};
use opcua::crypto::SecurityPolicy;
use opcua::server::{ServerBuilder, ServerHandle, ANONYMOUS_USER_TOKEN_ID};
use opcua::types::{BuildInfo, DataValue, DateTime, MessageSecurityMode, NodeId, UAString};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tokio_util::sync::CancellationToken;

const POINTS_NS_URI: &str = "urn:protocol-bench:points";

/// Snapshot of one host point for address-space sync.
#[derive(Debug, Clone)]
pub struct PointSnap {
    pub id: String,
    pub value: String,
}

struct LiveServer {
    handle: ServerHandle,
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    ns: u16,
    folder: NodeId,
    nodes: HashMap<String, NodeId>,
}

/// Embedded UA server running on its own OS thread.
pub struct EmbeddedServer {
    listen: String,
    cancel: CancellationToken,
    join: Option<JoinHandle<()>>,
    live: Arc<Mutex<Option<LiveServer>>>,
}

impl EmbeddedServer {
    pub fn listen_url(&self) -> &str {
        &self.listen
    }

    pub fn start(cfg: EmbeddedServerConfig) -> Result<Self, String> {
        let host = if cfg.bind.trim().is_empty()
            || cfg.bind == "0.0.0.0"
            || cfg.bind == "::"
            || cfg.bind == "[::]"
        {
            "0.0.0.0".to_string()
        } else {
            cfg.bind.clone()
        };
        let advertise = if host == "0.0.0.0" || host == "::" {
            "127.0.0.1".to_string()
        } else {
            host.clone()
        };
        let listen = format!("opc.tcp://{advertise}:{}", cfg.port);
        let pki = resolve_pki_dir(&cfg.pki_dir)?;
        let cancel = CancellationToken::new();
        let cancel_c = cancel.clone();
        let live: Arc<Mutex<Option<LiveServer>>> = Arc::new(Mutex::new(None));
        let live_c = Arc::clone(&live);
        let cfg_c = cfg.clone();

        let join = thread::Builder::new()
            .name("pb-opcua-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_) => return,
                };
                rt.block_on(async move {
                    if let Err(e) = run_server(cfg_c, host, pki, cancel_c, live_c).await {
                        log::error!("embedded OPC UA server: {e}");
                    }
                });
            })
            .map_err(|e| format!("spawn server: {e}"))?;

        // Brief settle so Start status can probe.
        std::thread::sleep(std::time::Duration::from_millis(80));
        Ok(Self {
            listen,
            cancel,
            join: Some(join),
            live,
        })
    }

    /// Create/update Variable nodes for every host point (string NodeIds = point ids).
    pub fn sync_points(&self, points: &[PointSnap]) {
        let mut guard = match self.live.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(live) = guard.as_mut() else {
            return;
        };
        let subscriptions = live.handle.subscriptions().clone();
        let address_space = live.manager.address_space();
        {
            let mut space = address_space.write();
            for p in points {
                if live.nodes.contains_key(&p.id) {
                    continue;
                }
                let nid = NodeId::new(live.ns, p.id.as_str());
                let _ = space.add_variables(
                    vec![Variable::new(&nid, &p.id, &p.id, UAString::from(p.value.as_str()))],
                    &live.folder,
                );
                live.nodes.insert(p.id.clone(), nid);
            }
        }
        let updates: Vec<_> = points
            .iter()
            .filter_map(|p| {
                let nid = live.nodes.get(&p.id)?;
                Some((
                    nid,
                    None,
                    DataValue::new_now(UAString::from(p.value.as_str())),
                ))
            })
            .collect();
        if !updates.is_empty() {
            let _ = live.manager.set_values(
                &subscriptions,
                updates.iter().map(|(n, a, v)| (*n, *a, v.clone())),
            );
        }
    }

    pub fn stop(mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        if let Ok(mut g) = self.live.lock() {
            *g = None;
        }
    }
}

impl Drop for EmbeddedServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddedServerConfig {
    pub bind: String,
    pub port: u16,
    pub application_uri: String,
    pub application_name: String,
    pub allow_anonymous: bool,
    pub enable_sign_and_encrypt: bool,
    pub auto_generate_cert: bool,
    pub auto_trust_clients: bool,
    pub pki_dir: String,
}

async fn run_server(
    cfg: EmbeddedServerConfig,
    host: String,
    pki: PathBuf,
    cancel: CancellationToken,
    live: Arc<Mutex<Option<LiveServer>>>,
) -> Result<(), String> {
    let tokens: Vec<&str> = if cfg.allow_anonymous {
        vec![ANONYMOUS_USER_TOKEN_ID]
    } else {
        vec![ANONYMOUS_USER_TOKEN_ID] // v1 still offers anonymous; users[] later
    };

    let mut builder = ServerBuilder::new()
        .application_name(cfg.application_name.clone())
        .application_uri(cfg.application_uri.clone())
        .product_uri("urn:protocol-bench")
        .host(host)
        .port(cfg.port)
        .pki_dir(pki)
        .create_sample_keypair(cfg.auto_generate_cert)
        .trust_client_certs(cfg.auto_trust_clients)
        .build_info(BuildInfo {
            product_uri: "urn:protocol-bench".into(),
            manufacturer_name: "Agile Automation".into(),
            product_name: "Protocol Bench".into(),
            software_version: env!("CARGO_PKG_VERSION").into(),
            build_number: "1".into(),
            build_date: DateTime::now(),
        })
        .with_node_manager(simple_node_manager(
            NamespaceMetadata {
                namespace_uri: POINTS_NS_URI.to_owned(),
                ..Default::default()
            },
            "points",
        ))
        .add_endpoint(
            "none",
            (
                "/",
                SecurityPolicy::None,
                MessageSecurityMode::None,
                &tokens as &[&str],
            ),
        );

    if cfg.enable_sign_and_encrypt {
        builder = builder.add_endpoint(
            "basic256sha256_sign_encrypt",
            (
                "/",
                SecurityPolicy::Basic256Sha256,
                MessageSecurityMode::SignAndEncrypt,
                &tokens as &[&str],
            ),
        );
    }

    let (server, handle) = builder.build().map_err(|e| format!("server build: {e:?}"))?;
    let manager = handle
        .node_managers()
        .get_of_type::<SimpleNodeManager>()
        .ok_or_else(|| "points node manager missing".to_string())?;
    let ns = handle
        .get_namespace_index(POINTS_NS_URI)
        .ok_or_else(|| "points namespace missing".to_string())?;

    let folder = NodeId::new(ns, "Points");
    {
        let address_space = manager.address_space();
        let mut space = address_space.write();
        space.add_folder(&folder, "Points", "Points", &NodeId::objects_folder_id());
    }

    if let Ok(mut g) = live.lock() {
        *g = Some(LiveServer {
            handle: handle.clone(),
            manager,
            ns,
            folder,
            nodes: HashMap::new(),
        });
    }

    let handle_c = handle.clone();
    tokio::spawn(async move {
        cancel.cancelled().await;
        handle_c.cancel();
    });

    server.run().await.map_err(|e| format!("server run: {e}"))?;
    Ok(())
}

fn resolve_pki_dir(override_path: &str) -> Result<PathBuf, String> {
    if !override_path.trim().is_empty() {
        let p = PathBuf::from(override_path.trim());
        std::fs::create_dir_all(&p).map_err(|e| format!("pki dir: {e}"))?;
        return Ok(p);
    }
    let dirs = directories::ProjectDirs::from("ca", "AgileAutomation", "ProtocolBench")
        .ok_or_else(|| "no config dir for OPC UA PKI".to_string())?;
    let p = dirs.data_dir().join("opcua-pki");
    std::fs::create_dir_all(&p).map_err(|e| format!("pki dir: {e}"))?;
    Ok(p)
}
