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
    /// Public BrowseName; empty means the point id.
    pub browse_name: String,
    /// Public DisplayName; empty means the browse name.
    pub display_name: String,
    /// Folder path under `Points` (`WellA/Tank1`); empty = directly under `Points`.
    pub folder: String,
}

impl PointSnap {
    fn browse(&self) -> &str {
        if self.browse_name.is_empty() { &self.id } else { &self.browse_name }
    }
    fn display(&self) -> &str {
        if self.display_name.is_empty() { self.browse() } else { &self.display_name }
    }
    fn folder_path(&self) -> String {
        self.folder
            .split('/')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    }
    fn signature(&self) -> (String, String, String) {
        (self.browse().to_string(), self.display().to_string(), self.folder_path())
    }
}

struct LiveServer {
    handle: ServerHandle,
    manager: Arc<InMemoryNodeManager<SimpleNodeManagerImpl>>,
    ns: u16,
    folder: NodeId,
    nodes: HashMap<String, (NodeId, (String, String, String))>,
    folders: HashMap<String, NodeId>,
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
            // Drop nodes whose point left the snapshot, then folders nothing lives in.
            let current: std::collections::HashSet<&str> = points.iter().map(|p| p.id.as_str()).collect();
            let gone: Vec<String> = live.nodes.keys().filter(|k| !current.contains(k.as_str())).cloned().collect();
            for k in gone {
                if let Some((nid, _)) = live.nodes.remove(&k) {
                    space.delete(&nid, true);
                }
            }
            for p in points {
                let sig = p.signature();
                if let Some((nid, old)) = live.nodes.get(&p.id) {
                    if *old == sig {
                        continue;
                    }
                    // Catalog edit: NodeId is stable, so drop and re-add under the new names.
                    let nid = nid.clone();
                    space.delete(&nid, true);
                    live.nodes.remove(&p.id);
                }
                let parent = ensure_folders(&mut space, live.ns, &live.folder, &mut live.folders, &sig.2);
                let nid = NodeId::new(live.ns, p.id.as_str());
                let _ = space.add_variables(
                    vec![Variable::new(&nid, &sig.0, &sig.1, UAString::from(p.value.as_str()))],
                    &parent,
                );
                live.nodes.insert(p.id.clone(), (nid, sig));
            }
            prune_folders(&mut space, &live.nodes, &mut live.folders);
        }
        let updates: Vec<_> = points
            .iter()
            .filter_map(|p| {
                let (nid, _) = live.nodes.get(&p.id)?;
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
    /// Stored from the project but **not enforced yet**: the server always offers anonymous login
    /// because user accounts are not implemented (see `tokens` in `run_server`).
    #[allow(dead_code)]
    pub allow_anonymous: bool,
    pub enable_sign_and_encrypt: bool,
    pub auto_generate_cert: bool,
    pub auto_trust_clients: bool,
    pub pki_dir: String,
}

/// Delete cached folders that no live point sits in (or under), deepest first.
fn prune_folders(
    space: &mut opcua::server::address_space::AddressSpace,
    nodes: &HashMap<String, (NodeId, (String, String, String))>,
    folders: &mut HashMap<String, NodeId>,
) {
    let mut needed = std::collections::HashSet::new();
    for (_, sig) in nodes.values() {
        let mut acc = String::new();
        for seg in sig.2.split('/').filter(|s| !s.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            needed.insert(acc.clone());
        }
    }
    let mut stale: Vec<String> = folders.keys().filter(|k| !needed.contains(*k)).cloned().collect();
    stale.sort_by_key(|k| std::cmp::Reverse(k.matches('/').count()));
    for k in stale {
        if let Some(id) = folders.remove(&k) {
            space.delete(&id, true);
        }
    }
}

/// Create any missing folder nodes along `path` (under `root`); returns the leaf folder.
fn ensure_folders(
    space: &mut opcua::server::address_space::AddressSpace,
    ns: u16,
    root: &NodeId,
    cache: &mut HashMap<String, NodeId>,
    path: &str,
) -> NodeId {
    let mut parent = root.clone();
    let mut acc = String::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        if let Some(id) = cache.get(&acc) {
            parent = id.clone();
            continue;
        }
        let id = NodeId::new(ns, format!("Points/{acc}"));
        space.add_folder(&id, seg, seg, &parent);
        cache.insert(acc.clone(), id.clone());
        parent = id;
    }
    parent
}

async fn run_server(
    cfg: EmbeddedServerConfig,
    host: String,
    advertise: String,
    pki: PathBuf,
    cancel: CancellationToken,
    live: Arc<Mutex<Option<LiveServer>>>,
) -> Result<(), String> {
    // v1 always offers anonymous, even when `allow_anonymous` is off: user accounts (`users[]`)
    // are not implemented yet, so refusing anonymous would leave no way in.
    let tokens: Vec<&str> = vec![ANONYMOUS_USER_TOKEN_ID];

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
            folders: HashMap::new(),
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

    /// A port nothing is listening on (fixed ports collide with parallel test runs).
    /// The listener may come up a moment after `start` returns on a loaded machine.
    fn connects(port: u16) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let addr = format!("127.0.0.1:{port}").parse().unwrap();
            if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(500)).is_ok() {
                return true;
            }
            if std::time::Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn start_binds_tcp() {
        let dir = std::env::temp_dir().join(format!("pb-opcua-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let port = free_port();
        let server = EmbeddedServer::start(cfg(port, "127.0.0.1", &dir.to_string_lossy())).expect("start");
        assert!(server.live.lock().map(|g| g.is_some()).unwrap_or(false));
        assert!(connects(port), "listener on {port}");
        server.stop();
    }

    #[test]
    fn start_binds_tcp_zerohost_app_pki() {
        // Own temp PKI: never touch the user's real certificate store from a test.
        let dir_buf = std::env::temp_dir().join(format!("pb-opcua-probe0-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir_buf);
        let dir = dir_buf.to_str().unwrap();
        let port = free_port();
        let server = EmbeddedServer::start(cfg(port, "0.0.0.0", dir)).expect("start");
        assert!(connects(port), "listener on {port}");
        server.stop();
    }
}
