//! Byte links shared by line-oriented ext packs: TCP client, TCP listener (fan-out to every
//! connected peer) and serial port. Reads never block longer than ~50 ms so a worker can
//! interleave reading, periodic sends and stop checks on one thread.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::ConnExt;

/// How a connection reaches its peer.
#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    TcpClient { host: String, port: u16 },
    TcpListen { host: String, port: u16 },
    Serial { port: String, baud: u32, parity: String, data_bits: u8, stop_bits: u8 },
}

impl Transport {
    /// From connection fields: `transport` = `tcp_client` | `tcp_listen` | `serial`, plus
    /// `host` / `port` or `serial_port` / `baud` / `parity` / `data_bits` / `stop_bits`.
    pub fn from_conn(c: &Map<String, Value>, default_port: u16) -> Result<Self, String> {
        let port = c.n("port", default_port as f64);
        if !(1.0..=65535.0).contains(&port) {
            return Err(format!("port {port} out of range"));
        }
        let port = port as u16;
        match c.s("transport", "tcp_client").to_ascii_lowercase().as_str() {
            "tcp_client" | "tcp" | "client" => {
                let host = c.s("host", "127.0.0.1");
                Ok(Self::TcpClient { host, port })
            }
            "tcp_listen" | "listen" | "server" => Ok(Self::TcpListen { host: c.s("host", "0.0.0.0"), port }),
            "serial" | "rs232" | "rs485" => {
                let sp = c.s("serial_port", "");
                if sp.is_empty() {
                    return Err("no serial port set".into());
                }
                Ok(Self::Serial {
                    port: sp,
                    baud: c.n("baud", 9600.0) as u32,
                    parity: c.s("parity", "none"),
                    data_bits: c.n("data_bits", 8.0) as u8,
                    stop_bits: c.n("stop_bits", 1.0) as u8,
                })
            }
            other => Err(format!("unknown transport \"{other}\"")),
        }
    }

    pub fn endpoint(&self) -> String {
        match self {
            Self::TcpClient { host, port } => format!("{host}:{port}"),
            Self::TcpListen { host, port } => format!("listen {host}:{port}"),
            Self::Serial { port, baud, .. } => format!("{port} @ {baud}"),
        }
    }
}

enum Kind {
    Stream(TcpStream),
    Listen { listener: TcpListener, peers: Vec<TcpStream> },
    Serial(Box<dyn serialport::SerialPort>),
}

/// An open link.
pub struct ByteLink {
    kind: Kind,
}

const SLICE: Duration = Duration::from_millis(50);

impl ByteLink {
    pub fn open(t: &Transport) -> io::Result<Self> {
        let kind = match t {
            Transport::TcpClient { host, port } => {
                let addr = (host.as_str(), *port)
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| io::Error::new(ErrorKind::NotFound, format!("cannot resolve {host}")))?;
                let s = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
                s.set_read_timeout(Some(SLICE))?;
                s.set_nodelay(true).ok();
                Kind::Stream(s)
            }
            Transport::TcpListen { host, port } => {
                let l = TcpListener::bind((host.as_str(), *port))?;
                l.set_nonblocking(true)?;
                Kind::Listen { listener: l, peers: Vec::new() }
            }
            Transport::Serial { port, baud, parity, data_bits, stop_bits } => {
                let parity = match parity.to_ascii_lowercase().as_str() {
                    "even" => serialport::Parity::Even,
                    "odd" => serialport::Parity::Odd,
                    _ => serialport::Parity::None,
                };
                let data_bits = match data_bits {
                    7 => serialport::DataBits::Seven,
                    _ => serialport::DataBits::Eight,
                };
                let stop_bits = if *stop_bits == 2 { serialport::StopBits::Two } else { serialport::StopBits::One };
                let p = serialport::new(port, *baud)
                    .parity(parity)
                    .data_bits(data_bits)
                    .stop_bits(stop_bits)
                    .flow_control(serialport::FlowControl::None)
                    .timeout(SLICE)
                    .open()
                    .map_err(|e| io::Error::new(ErrorKind::Other, format!("{port}: {e}")))?;
                Kind::Serial(p)
            }
        };
        Ok(Self { kind })
    }

    /// Append whatever arrived (waiting at most ~50 ms) to `buf`. Returns the byte count.
    /// A client / serial link that closed is an error; a listener drops dead peers quietly.
    pub fn read(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let mut tmp = [0u8; 4096];
        match &mut self.kind {
            Kind::Stream(s) => match s.read(&mut tmp) {
                Ok(0) => Err(io::Error::new(ErrorKind::ConnectionAborted, "peer closed the connection")),
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    Ok(n)
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => Ok(0),
                Err(e) => Err(e),
            },
            Kind::Serial(p) => match p.read(&mut tmp) {
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    Ok(n)
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => Ok(0),
                Err(e) => Err(e),
            },
            Kind::Listen { listener, peers } => {
                loop {
                    match listener.accept() {
                        Ok((s, _)) => {
                            s.set_nonblocking(true).ok();
                            s.set_nodelay(true).ok();
                            peers.push(s);
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e),
                    }
                }
                let mut total = 0;
                peers.retain_mut(|s| loop {
                    match s.read(&mut tmp) {
                        Ok(0) => return false,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            total += n;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => return true,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(_) => return false,
                    }
                });
                if total == 0 {
                    std::thread::sleep(SLICE.min(Duration::from_millis(20)));
                }
                Ok(total)
            }
        }
    }

    /// Send to the peer (listener: every connected peer; failed peers are dropped).
    pub fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match &mut self.kind {
            Kind::Stream(s) => s.write_all(data),
            Kind::Serial(p) => p.write_all(data),
            Kind::Listen { peers, .. } => {
                peers.retain_mut(|s| {
                    s.set_nonblocking(false).ok();
                    s.set_write_timeout(Some(Duration::from_secs(2))).ok();
                    let ok = s.write_all(data).is_ok();
                    s.set_nonblocking(true).ok();
                    ok
                });
                Ok(())
            }
        }
    }

    /// Connected peers for a listener; `None` for point-to-point links.
    pub fn peers(&self) -> Option<u64> {
        match &self.kind {
            Kind::Listen { peers, .. } => Some(peers.len() as u64),
            _ => None,
        }
    }
}

/// Split complete lines (`\n`, optional `\r`) off the front of `buf`; a partial line stays.
pub fn take_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(i) = buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buf.drain(..=i).collect();
        let text = String::from_utf8_lossy(&line);
        out.push(text.trim_end_matches(['\r', '\n']).to_string());
    }
    // A runaway line without a newline must not grow forever.
    if buf.len() > 64 * 1024 {
        buf.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transports_from_fields() {
        let c = serde_json::json!({"transport":"tcp_listen","port":"5100"});
        assert_eq!(Transport::from_conn(c.as_object().unwrap(), 1).unwrap(), Transport::TcpListen { host: "0.0.0.0".into(), port: 5100 });
        let c = serde_json::json!({"transport":"serial"});
        assert!(Transport::from_conn(c.as_object().unwrap(), 1).is_err());
        let c = serde_json::json!({"transport":"serial","serial_port":"/dev/ttyS0","baud":19200});
        assert_eq!(Transport::from_conn(c.as_object().unwrap(), 1).unwrap().endpoint(), "/dev/ttyS0 @ 19200");
        let c = serde_json::json!({"port":70000});
        assert!(Transport::from_conn(c.as_object().unwrap(), 1).is_err());
        let c = serde_json::json!({"transport":"carrier pigeon"});
        assert!(Transport::from_conn(c.as_object().unwrap(), 1).is_err());
    }

    #[test]
    fn lines_split_and_partials_wait() {
        let mut b = b"a\r\nbb\ncc".to_vec();
        assert_eq!(take_lines(&mut b), vec!["a".to_string(), "bb".to_string()]);
        assert_eq!(b, b"cc");
    }

    #[test]
    fn listener_fans_out_and_reads_from_peers() {
        let port = crate::testhost::free_port();
        let mut l = ByteLink::open(&Transport::TcpListen { host: "127.0.0.1".into(), port }).unwrap();
        let mut c1 = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut c2 = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut buf = Vec::new();
        for _ in 0..20 {
            l.read(&mut buf).unwrap();
            if l.peers() == Some(2) {
                break;
            }
        }
        assert_eq!(l.peers(), Some(2));
        l.write_all(b"hi\n").unwrap();
        let mut got = [0u8; 3];
        c1.read_exact(&mut got).unwrap();
        c2.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hi\n");
        c1.write_all(b"from1\n").unwrap();
        for _ in 0..40 {
            l.read(&mut buf).unwrap();
            if buf.ends_with(b"\n") {
                break;
            }
        }
        assert_eq!(take_lines(&mut buf), vec!["from1".to_string()]);
    }
}
