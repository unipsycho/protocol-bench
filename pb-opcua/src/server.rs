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
        let boot_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let boot_err_c = Arc::clone(&boot_err);
        let cfg_c = cfg.clone();
        let advertise_c = advertise.clone();

        let join = thread::Builder::new()
            .name("pb-opcua-server".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        if let Ok(mut g) = boot_err_c.lock() {
                            *g = Some(format!("tokio runtime: {e}"));
                        }
                        return;
                    }
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    rt.block_on(async move {
                        run_server(cfg_c, host, advertise_c, pki, cancel_c, live_c).await
                    })
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        log::error!("embedded OPC UA server: {e}");
                        if let Ok(mut g) = boot_err_c.lock() {
                            *g = Some(e);
                        }
                    }
                    Err(panic) => {
                        let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                            format!("panic: {s}")
                        } else if let Some(s) = panic.downcast_ref::<String>() {
                            format!("panic: {s}")
                        } else {
                            "panic: embedded OPC UA server thread".into()
                        };
                        log::error!("embedded OPC UA server: {msg}");
                        if let Ok(mut g) = boot_err_c.lock() {
                            *g = Some(msg);
                        }
                    }
                }
            })
            .map_err(|e| format!("spawn server: {e}"))?;

        // Wait until address space is live (build succeeded) or boot failed.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            if live.lock().map(|g| g.is_some()).unwrap_or(false) {
                break;
            }
            let failed = boot_err.lock().ok().and_then(|g| g.clone());
            if let Some(e) = failed {
                let _ = join.join();
                return Err(e);
            }
            if std::time::Instant::now() >= deadline {
                let detail = boot_err
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .unwrap_or_else(|| "embedded OPC UA server did not become ready".into());
                cancel.cancel();
                let _ = join.join();
                return Err(detail);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
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
    advertise: String,
    pki: PathBuf,
    cancel: CancellationToken,
    live: Arc<Mutex<Option<LiveServer>>>,
) -> Result<(), String> {
    let tokens: Vec<&str> = if cfg.allow_anonymous {
        vec![ANONYMOUS_USER_TOKEN_ID]
    } else {
        vec![ANONYMOUS_USER_TOKEN_ID] // v1 still offers anonymous; users[] later
    };

    // async-opcua requires at least one discovery URL (build fails otherwise).
    let discovery = format!("opc.tcp://{advertise}:{}/", cfg.port);

    let mut builder = ServerBuilder::new()
        .application_name(cfg.application_name.clone())
        .application_uri(cfg.application_uri.clone())
        .product_uri("urn:protocol-bench")
        .host(host)
        .port(cfg.port)
        .discovery_urls(vec![discovery])
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

    // Mark ready before run() so start() can unblock while accept loop starts.
    {
        let mut g = live
            .lock()
            .map_err(|_| "embedded OPC UA live lock poisoned".to_string())?;
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

#[cfg(test)]
mod listen_probe {
    use super::*;

    fn cfg(port: u16, bind: &str, pki: &str) -> EmbeddedServerConfig {
        EmbeddedServerConfig {
            bind: bind.into(),
            port,
            application_uri: "urn:protocol-bench:probe".into(),
            application_name: "probe".into(),
            allow_anonymous: true,
            enable_sign_and_encrypt: false,
            auto_generate_cert: true,
            auto_trust_clients: true,
            pki_dir: pki.into(),
        }
    }

    #[test]
    fn start_binds_tcp() {
        let dir = std::env::temp_dir().join(format!("pb-opcua-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let server = EmbeddedServer::start(cfg(4847, "127.0.0.1", &dir.to_string_lossy())).expect("start");
        assert!(server.live.lock().map(|g| g.is_some()).unwrap_or(false));
        assert!(std::net::TcpStream::connect_timeout(
            &"127.0.0.1:4847".parse().unwrap(),
            std::time::Duration::from_millis(500),
        )
        .is_ok());
        server.stop();
    }

    #[test]
    fn start_binds_tcp_zerohost_app_pki() {
        let dir = "/home/mike/.local/share/protocolbench/opcua-pki";
        let _ = std::fs::create_dir_all(dir);
        let server = EmbeddedServer::start(cfg(4848, "0.0.0.0", dir)).expect("start");
        assert!(std::net::TcpStream::connect_timeout(
            &"127.0.0.1:4848".parse().unwrap(),
            std::time::Duration::from_millis(500),
        )
        .is_ok());
        server.stop();
    }
}
