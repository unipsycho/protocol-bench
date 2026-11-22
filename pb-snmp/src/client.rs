//! Blocking SNMP v1 / v2c manager over UDP: GET with retries, batching, and per-OID results.

use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

use crate::ber::{self, SnmpError, Value, ERR_NO_SUCH_NAME, ERR_TOO_BIG};

/// Varbinds per request. Many agents choke on larger PDUs; tooBig halves it further.
pub const MAX_BATCH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// No (valid) answer after all retries.
    Timeout,
    Io(String),
    Snmp(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => f.write_str("no response (timeout)"),
            Self::Io(m) | Self::Snmp(m) => f.write_str(m),
        }
    }
}

impl From<SnmpError> for ClientError {
    fn from(e: SnmpError) -> Self {
        Self::Snmp(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct TargetCfg {
    pub host: String,
    pub port: u16,
    /// `v1` | `v2c`.
    pub version: String,
    pub community: String,
    pub timeout: Duration,
    pub retries: u32,
}

pub struct Manager {
    sock: UdpSocket,
    version: i64,
    community: String,
    retries: u32,
    next_id: i32,
}

/// Result for one requested OID.
pub type OidResult = Result<Value, String>;

impl Manager {
    pub fn open(cfg: &TargetCfg) -> Result<Self, ClientError> {
        let version = match cfg.version.to_ascii_lowercase().as_str() {
            "v1" | "1" => 0,
            "v2c" | "2c" | "v2" => 1,
            other => return Err(ClientError::Snmp(format!("SNMP {other} is not supported (v1 / v2c only)"))),
        };
        let addr = (cfg.host.trim(), cfg.port)
            .to_socket_addrs()
            .map_err(|e| ClientError::Io(format!("{}:{}: {e}", cfg.host, cfg.port)))?
            .next()
            .ok_or_else(|| ClientError::Io(format!("{}:{}: no address", cfg.host, cfg.port)))?;
        let bind = if addr.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        let sock = UdpSocket::bind(bind).map_err(|e| ClientError::Io(e.to_string()))?;
        sock.connect(addr).map_err(|e| ClientError::Io(e.to_string()))?;
        sock.set_read_timeout(Some(cfg.timeout.max(Duration::from_millis(20)))).map_err(|e| ClientError::Io(e.to_string()))?;
        Ok(Self { sock, version, community: cfg.community.clone(), retries: cfg.retries, next_id: 1 })
    }

    /// One request / response with retries. A late answer to an earlier request is skipped.
    fn transact(&mut self, oids: &[String]) -> Result<ber::Response, ClientError> {
        let mut buf = vec![0u8; 65_535];
        for _ in 0..=self.retries {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            let id = self.next_id;
            let req = ber::encode_get(self.version, &self.community, id, oids)?;
            if let Err(e) = self.sock.send(&req) {
                return Err(ClientError::Io(e.to_string()));
            }
            // Drain until the matching id or a timeout (then retry with a fresh id).
            loop {
                match self.sock.recv(&mut buf) {
                    Ok(n) => match ber::decode_response(&buf[..n]) {
                        Ok(r) if r.request_id == id as i64 => return Ok(r),
                        _ => continue, // stale or junk datagram
                    },
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                    Err(e) => return Err(ClientError::Io(e.to_string())),
                }
            }
        }
        Err(ClientError::Timeout)
    }

    /// GET every OID; results line up with `oids`. `Err` is a link-level failure (nothing
    /// answered); per-OID problems (noSuchInstance …) are `Err` inside the vector.
    pub fn get(&mut self, oids: &[String]) -> Result<Vec<OidResult>, ClientError> {
        let mut out: Vec<Option<OidResult>> = vec![None; oids.len()];
        let mut todo: Vec<usize> = (0..oids.len()).collect();
        let mut batch = MAX_BATCH;
        while !todo.is_empty() {
            let take = batch.min(todo.len());
            let idxs: Vec<usize> = todo[..take].to_vec();
            let names: Vec<String> = idxs.iter().map(|i| oids[*i].clone()).collect();
            let resp = self.transact(&names)?;
            match resp.error_status {
                0 => {
                    if resp.varbinds.len() != idxs.len() {
                        return Err(ClientError::Snmp(format!("expected {} varbinds, got {}", idxs.len(), resp.varbinds.len())));
                    }
                    for (i, (_, v)) in idxs.iter().zip(resp.varbinds) {
                        out[*i] = Some(match v {
                            Value::Missing(why) => Err(why.to_string()),
                            other => Ok(other),
                        });
                    }
                    todo.drain(..take);
                }
                ERR_TOO_BIG if take > 1 => batch = (take / 2).max(1),
                ERR_NO_SUCH_NAME if resp.error_index >= 1 && (resp.error_index as usize) <= idxs.len() => {
                    // SNMPv1: the agent names the offending varbind. Drop it and retry the rest.
                    let bad = idxs[resp.error_index as usize - 1];
                    out[bad] = Some(Err("noSuchName".into()));
                    todo.retain(|i| *i != bad);
                }
                status => {
                    let msg = format!("agent error-status {status} (index {})", resp.error_index);
                    for i in &idxs {
                        out[*i] = Some(Err(msg.clone()));
                    }
                    todo.drain(..take);
                }
            }
        }
        Ok(out.into_iter().map(|o| o.unwrap_or_else(|| Err("no result".into()))).collect())
    }
}

#[cfg(test)]
pub(crate) mod testagent {
    use super::*;
    use std::collections::BTreeMap;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// A throwaway SNMP agent on 127.0.0.1.
    pub struct Agent {
        pub addr: SocketAddr,
        pub table: Arc<Mutex<BTreeMap<String, Value>>>,
        /// Ignore the first N datagrams (retry test).
        pub drop_first: Arc<AtomicUsize>,
        /// Answer tooBig for requests with more than this many varbinds (0 = unlimited).
        pub max_varbinds: Arc<AtomicUsize>,
        /// SNMPv1 behaviour: noSuchName + index instead of per-varbind noSuchInstance.
        pub v1_errors: Arc<AtomicBool>,
        pub requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl Agent {
        pub fn start(community: &str) -> Self {
            let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
            sock.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
            let addr = sock.local_addr().unwrap();
            let table: Arc<Mutex<BTreeMap<String, Value>>> = Arc::default();
            let (drop_first, max_varbinds, requests) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            let (v1_errors, stop) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
            let (t, df, mv, rq, v1, st, comm) = (table.clone(), drop_first.clone(), max_varbinds.clone(), requests.clone(), v1_errors.clone(), stop.clone(), community.to_string());
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 65_535];
                while !st.load(Ordering::SeqCst) {
                    let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
                    rq.fetch_add(1, Ordering::SeqCst);
                    if df.load(Ordering::SeqCst) > 0 {
                        df.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let Ok((ver, c, rid, oids)) = ber::decode_get_request(&buf[..n]) else { continue };
                    if c != comm {
                        continue; // wrong community: agents stay silent
                    }
                    let max = mv.load(Ordering::SeqCst);
                    let reply = if max > 0 && oids.len() > max {
                        ber::encode_response(ver, &c, rid, ERR_TOO_BIG, 0, &[])
                    } else {
                        let table = t.lock().unwrap();
                        let mut vbs = Vec::new();
                        let mut v1_err = None;
                        for (i, o) in oids.iter().enumerate() {
                            match table.get(o) {
                                Some(v) => vbs.push((o.clone(), v.clone())),
                                None if v1.load(Ordering::SeqCst) => {
                                    v1_err.get_or_insert(i + 1);
                                    vbs.push((o.clone(), Value::Null));
                                }
                                None => vbs.push((o.clone(), Value::Missing("noSuchInstance"))),
                            }
                        }
                        match v1_err {
                            Some(idx) => ber::encode_response(ver, &c, rid, ERR_NO_SUCH_NAME, idx as i64, &vbs),
                            None => ber::encode_response(ver, &c, rid, 0, 0, &vbs),
                        }
                    };
                    let _ = sock.send_to(&reply, from);
                }
            });
            Self { addr, table, drop_first, max_varbinds, v1_errors, requests, stop }
        }

        pub fn set(&self, oid: &str, v: Value) {
            self.table.lock().unwrap().insert(oid.to_string(), v);
        }
    }

    impl Drop for Agent {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    pub fn cfg(a: &Agent, version: &str, community: &str) -> TargetCfg {
        TargetCfg { host: "127.0.0.1".into(), port: a.addr.port(), version: version.into(), community: community.into(), timeout: Duration::from_millis(150), retries: 2 }
    }
}

#[cfg(test)]
mod tests {
    use super::testagent::*;
    use super::*;
    use std::sync::atomic::Ordering;

    fn oid(i: usize) -> String {
        format!("1.3.6.1.4.1.99.{i}")
    }

    #[test]
    fn gets_typed_values_and_flags_missing_oids_individually() {
        let a = Agent::start("public");
        a.set("1.3.6.1.2.1.1.1.0", Value::Text("Edge switch".into()));
        a.set("1.3.6.1.2.1.1.3.0", Value::Ticks(8_640_000));
        a.set("1.3.6.1.2.1.2.2.1.10.1", Value::UInt(1234));
        let mut m = Manager::open(&cfg(&a, "v2c", "public")).unwrap();
        let r = m
            .get(&["1.3.6.1.2.1.1.1.0".into(), "1.3.6.1.2.1.9.9.9".into(), "1.3.6.1.2.1.1.3.0".into(), "1.3.6.1.2.1.2.2.1.10.1".into()])
            .unwrap();
        assert_eq!(r[0], Ok(Value::Text("Edge switch".into())));
        assert_eq!(r[1], Err("noSuchInstance".into()));
        assert_eq!(r[2], Ok(Value::Ticks(8_640_000)));
        assert_eq!(r[3], Ok(Value::UInt(1234)));
    }

    #[test]
    fn large_oid_lists_are_batched_and_too_big_halves_the_batch() {
        let a = Agent::start("public");
        for i in 0..25 {
            a.set(&oid(i), Value::Int(i as i64));
        }
        let oids: Vec<String> = (0..25).map(oid).collect();
        let mut m = Manager::open(&cfg(&a, "v2c", "public")).unwrap();
        let r = m.get(&oids).unwrap();
        assert!(r.iter().enumerate().all(|(i, v)| *v == Ok(Value::Int(i as i64))));
        assert_eq!(a.requests.load(Ordering::SeqCst), 3, "25 OIDs = 10 + 10 + 5");
        // An agent that only takes 3 varbinds per PDU: 10 -> 5 -> 2 ... still answers everything.
        a.max_varbinds.store(3, Ordering::SeqCst);
        let r = m.get(&oids).unwrap();
        assert!(r.iter().enumerate().all(|(i, v)| *v == Ok(Value::Int(i as i64))));
    }

    #[test]
    fn v1_no_such_name_drops_only_the_offending_oid() {
        let a = Agent::start("public");
        a.v1_errors.store(true, Ordering::SeqCst);
        a.set(&oid(1), Value::Int(1));
        a.set(&oid(3), Value::Int(3));
        let mut m = Manager::open(&cfg(&a, "v1", "public")).unwrap();
        let r = m.get(&[oid(1), oid(2), oid(3)]).unwrap();
        assert_eq!(r[0], Ok(Value::Int(1)));
        assert_eq!(r[1], Err("noSuchName".into()));
        assert_eq!(r[2], Ok(Value::Int(3)));
    }

    #[test]
    fn retries_a_lost_datagram_and_times_out_for_a_silent_agent() {
        let a = Agent::start("public");
        a.set(&oid(1), Value::Int(5));
        a.drop_first.store(1, Ordering::SeqCst);
        let mut m = Manager::open(&cfg(&a, "v2c", "public")).unwrap();
        assert_eq!(m.get(&[oid(1)]).unwrap(), vec![Ok(Value::Int(5))]);
        // Wrong community: the agent never answers.
        let mut wrong = Manager::open(&cfg(&a, "v2c", "private")).unwrap();
        assert_eq!(wrong.get(&[oid(1)]), Err(ClientError::Timeout));
    }

    #[test]
    fn unsupported_version_and_bad_oids_are_clear_errors() {
        let a = Agent::start("public");
        assert!(matches!(Manager::open(&cfg(&a, "v3", "x")), Err(ClientError::Snmp(m)) if m.contains("v1 / v2c")));
        let mut m = Manager::open(&cfg(&a, "v2c", "public")).unwrap();
        assert!(matches!(m.get(&["not-an-oid".into()]), Err(ClientError::Snmp(_))));
    }
}
