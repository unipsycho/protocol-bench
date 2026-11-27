//! PEAK PCAN Ethernet gateway (PCAN-Ethernet Gateway DR, PCAN-Gateway FD, …) LAN frames.
//!
//! The gateway forwards CAN traffic to the PC over UDP or TCP, configured as *routes* in its web
//! page. Each CAN frame travels as one big-endian record (several may share a datagram or sit
//! back to back in a TCP stream):
//!
//! | Bytes | Field |
//! | --- | --- |
//! | 0–1 | length of this record, including these two bytes (36 classic, 40 with CRC) |
//! | 2–3 | message type: `0x80` classic CAN, `0x81` classic + CRC32, `0x90` CAN FD, `0x91` FD + CRC32 |
//! | 4–11 | tag (unused, 0) |
//! | 12–19 | timestamp µs, low word then high word |
//! | 20 | channel |
//! | 21 | DLC (FD: 0–15 DLC code) |
//! | 22–23 | flags |
//! | 24–27 | CAN id; bit 30 = RTR, bit 31 = 29-bit extended |
//! | 28– | data (8 bytes classic; DLC-sized for FD), then the optional CRC32 |
//!
//! We send classic frames (`0x80`) and read classic and CAN FD records; FD frames with more than
//! eight data bytes don't fit an 8-byte signal frame and are skipped. CRC32 values are not checked.
//! Bit rate and channel routing are set in the gateway itself.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use super::{CanBus, Frame};

pub const TYPE_CAN: u16 = 0x80;
pub const TYPE_CAN_CRC: u16 = 0x81;
pub const TYPE_FD: u16 = 0x90;
pub const TYPE_FD_CRC: u16 = 0x91;
const HEADER: usize = 28;
const ID_EXT: u32 = 0x8000_0000;
const ID_RTR: u32 = 0x4000_0000;

/// CAN FD DLC code → data length.
pub fn fd_len(dlc: u8) -> usize {
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64][dlc.min(15) as usize]
}

/// One classic CAN record (36 bytes).
pub fn encode(f: &Frame) -> [u8; 36] {
    let mut b = [0u8; 36];
    b[0..2].copy_from_slice(&36u16.to_be_bytes());
    b[2..4].copy_from_slice(&TYPE_CAN.to_be_bytes());
    b[21] = f.dlc.min(8);
    let id = f.id & if f.extended { 0x1FFF_FFFF } else { 0x7FF } | if f.extended { ID_EXT } else { 0 };
    b[24..28].copy_from_slice(&id.to_be_bytes());
    b[28..36].copy_from_slice(&f.data);
    b
}

/// What one record turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Frame { frame: Frame, channel: u8 },
    /// Valid record we don't turn into a frame (RTR, FD > 8 bytes, unknown type).
    Skipped(&'static str),
}

/// Split `buf` into records. Returns them plus how many bytes were used; a partial record at
/// the end is left for the next read (TCP). A nonsense length is an error.
pub fn decode(buf: &[u8]) -> io::Result<(Vec<Record>, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while buf.len() - at >= 4 {
        let len = u16::from_be_bytes([buf[at], buf[at + 1]]) as usize;
        if !(HEADER..=256).contains(&len) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad PCAN record length {len}")));
        }
        if buf.len() - at < len {
            break;
        }
        out.push(record(&buf[at..at + len]));
        at += len;
    }
    Ok((out, at))
}

fn record(r: &[u8]) -> Record {
    let kind = u16::from_be_bytes([r[2], r[3]]);
    let channel = r[20];
    let dlc = r[21];
    let raw_id = u32::from_be_bytes([r[24], r[25], r[26], r[27]]);
    let n = match kind {
        TYPE_CAN | TYPE_CAN_CRC => dlc.min(8) as usize,
        TYPE_FD | TYPE_FD_CRC => fd_len(dlc),
        _ => return Record::Skipped("unknown record type"),
    };
    if raw_id & ID_RTR != 0 {
        return Record::Skipped("remote frame");
    }
    if n > 8 {
        return Record::Skipped("CAN FD frame over 8 bytes");
    }
    if r.len() < HEADER + n {
        return Record::Skipped("short record");
    }
    let extended = raw_id & ID_EXT != 0;
    let mut data = [0u8; 8];
    data[..n].copy_from_slice(&r[HEADER..HEADER + n]);
    Record::Frame { frame: Frame { id: raw_id & if extended { 0x1FFF_FFFF } else { 0x7FF }, extended, dlc: n as u8, data }, channel }
}

fn resolve(host: &str, port: u16) -> io::Result<SocketAddr> {
    (host, port).to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("cannot resolve {host}")))
}

/// UDP routes: we listen on `local_port` (the gateway's CAN → LAN route targets this PC and
/// port) and send to `host:port` (the gateway's LAN → CAN route).
pub struct PcanUdp {
    sock: UdpSocket,
    gateway: SocketAddr,
    queue: VecDeque<Frame>,
    pub skipped: u64,
}

impl PcanUdp {
    pub fn open(host: &str, port: u16, local_port: u16) -> io::Result<PcanUdp> {
        let gateway = resolve(host, port)?;
        let bind: SocketAddr = if gateway.is_ipv6() { ([0u16; 8], local_port).into() } else { ([0, 0, 0, 0], local_port).into() };
        let sock = UdpSocket::bind(bind).map_err(|e| io::Error::new(e.kind(), format!("listen on UDP {local_port}: {e}")))?;
        sock.set_read_timeout(Some(Duration::from_millis(50)))?;
        Ok(PcanUdp { sock, gateway, queue: VecDeque::new(), skipped: 0 })
    }

    pub fn local_port(&self) -> u16 {
        self.sock.local_addr().map(|a| a.port()).unwrap_or(0)
    }
}

impl CanBus for PcanUdp {
    fn read(&mut self) -> io::Result<Option<Frame>> {
        if let Some(f) = self.queue.pop_front() {
            return Ok(Some(f));
        }
        let mut buf = [0u8; 1500];
        let n = match self.sock.recv_from(&mut buf) {
            Ok((n, _)) => n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => return Ok(None),
            // Windows reports an ICMP "port unreachable" from an earlier send this way.
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => return Ok(None),
            Err(e) => return Err(e),
        };
        // A garbled datagram is dropped, not fatal.
        if let Ok((recs, _)) = decode(&buf[..n]) {
            for r in recs {
                match r {
                    Record::Frame { frame, .. } => self.queue.push_back(frame),
                    Record::Skipped(_) => self.skipped += 1,
                }
            }
        }
        Ok(self.queue.pop_front())
    }

    fn write(&mut self, f: &Frame) -> io::Result<()> {
        self.sock.send_to(&encode(f), self.gateway).map(|_| ())
    }
}

/// TCP route: one connection to the gateway carries both directions.
pub struct PcanTcp {
    stream: TcpStream,
    buf: Vec<u8>,
    queue: VecDeque<Frame>,
    pub skipped: u64,
}

impl PcanTcp {
    pub fn open(host: &str, port: u16) -> io::Result<PcanTcp> {
        let stream = TcpStream::connect_timeout(&resolve(host, port)?, Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_millis(50)))?;
        stream.set_nodelay(true)?;
        Ok(PcanTcp { stream, buf: Vec::new(), queue: VecDeque::new(), skipped: 0 })
    }
}

impl CanBus for PcanTcp {
    fn read(&mut self) -> io::Result<Option<Frame>> {
        if let Some(f) = self.queue.pop_front() {
            return Ok(Some(f));
        }
        let mut chunk = [0u8; 4096];
        let n = match self.stream.read(&mut chunk) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "gateway closed the connection")),
            Ok(n) => n,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => return Ok(None),
            Err(e) => return Err(e),
        };
        self.buf.extend_from_slice(&chunk[..n]);
        // Out of step in a byte stream can't be recovered: reconnect.
        let (recs, used) = decode(&self.buf)?;
        self.buf.drain(..used);
        for r in recs {
            match r {
                Record::Frame { frame, .. } => self.queue.push_back(frame),
                Record::Skipped(_) => self.skipped += 1,
            }
        }
        Ok(self.queue.pop_front())
    }

    fn write(&mut self, f: &Frame) -> io::Result<()> {
        self.stream.write_all(&encode(f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: u16, dlc: u8, id: u32, data: &[u8], crc: bool) -> Vec<u8> {
        let mut b = vec![0u8; HEADER];
        b[2..4].copy_from_slice(&kind.to_be_bytes());
        b[20] = 1;
        b[21] = dlc;
        b[24..28].copy_from_slice(&id.to_be_bytes());
        b.extend_from_slice(data);
        if crc {
            b.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        }
        let len = b.len() as u16;
        b[0..2].copy_from_slice(&len.to_be_bytes());
        b
    }

    #[test]
    fn classic_layout_matches_the_gateway_format() {
        let f = Frame { id: 0x18FEF100, extended: true, dlc: 8, data: [1, 2, 3, 4, 5, 6, 7, 8] };
        let b = encode(&f);
        assert_eq!(&b[0..4], &[0x00, 0x24, 0x00, 0x80], "length 36, type 0x80");
        assert_eq!(b[21], 8);
        assert_eq!(&b[24..28], &[0x98, 0xFE, 0xF1, 0x00], "id with the extended bit");
        assert_eq!(&b[28..36], &f.data);
        let (recs, used) = decode(&b).unwrap();
        assert_eq!((recs, used), (vec![Record::Frame { frame: f, channel: 0 }], 36));
        let s = Frame { id: 0x123, extended: false, dlc: 2, data: [0xA, 0xB, 0, 0, 0, 0, 0, 0] };
        assert_eq!(&encode(&s)[24..28], &[0, 0, 0x01, 0x23]);
    }

    #[test]
    fn several_records_crc_fd_and_partials() {
        let mut buf = rec(TYPE_CAN_CRC, 2, 0x100, &[9, 8, 0, 0, 0, 0, 0, 0], true);
        buf.extend(rec(TYPE_FD, 8, 0x8000_0200, &[1, 2, 3, 4, 5, 6, 7, 8], false));
        buf.extend(rec(TYPE_FD, 9, 0x300, &[0; 12], false));
        buf.extend(rec(TYPE_CAN, 0, 0x4000_0123, &[0; 8], false));
        let tail = rec(TYPE_CAN, 1, 0x7FF, &[0x55, 0, 0, 0, 0, 0, 0, 0], false);
        buf.extend_from_slice(&tail[..10]);
        let (recs, used) = decode(&buf).unwrap();
        assert_eq!(used, buf.len() - 10, "partial record left over");
        assert_eq!(recs[0], Record::Frame { frame: Frame { id: 0x100, extended: false, dlc: 2, data: [9, 8, 0, 0, 0, 0, 0, 0] }, channel: 1 });
        assert_eq!(recs[1], Record::Frame { frame: Frame { id: 0x200, extended: true, dlc: 8, data: [1, 2, 3, 4, 5, 6, 7, 8] }, channel: 1 });
        assert_eq!(recs[2], Record::Skipped("CAN FD frame over 8 bytes"));
        assert_eq!(recs[3], Record::Skipped("remote frame"));
        assert!(decode(&[0, 3, 0, 0x80]).is_err(), "nonsense length");
    }

    #[test]
    fn udp_round_trip_against_a_fake_gateway() {
        let gw = UdpSocket::bind("127.0.0.1:0").unwrap();
        gw.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut bus = PcanUdp::open("127.0.0.1", gw.local_addr().unwrap().port(), 0).unwrap();
        let f = Frame { id: 0x0CF00400, extended: true, dlc: 8, data: [0xFF, 0xFF, 0xFF, 0xE0, 0x2E, 0xFF, 0xFF, 0xFF] };
        bus.write(&f).unwrap();
        let mut b = [0u8; 64];
        let (n, from) = gw.recv_from(&mut b).unwrap();
        assert_eq!(&b[..n], &encode(&f));
        // Two frames in one datagram back to us.
        let g = Frame { id: 0x101, extended: false, dlc: 1, data: [7, 0, 0, 0, 0, 0, 0, 0] };
        let mut dg = encode(&f).to_vec();
        dg.extend_from_slice(&encode(&g));
        gw.send_to(&dg, ("127.0.0.1", from.port())).unwrap();
        let mut got = Vec::new();
        for _ in 0..40 {
            if let Some(x) = bus.read().unwrap() {
                got.push(x);
            }
            if got.len() == 2 {
                break;
            }
        }
        assert_eq!(got, vec![f, g]);
    }

    #[test]
    fn tcp_stream_split_across_reads() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let f = Frame { id: 0x200, extended: false, dlc: 2, data: [3, 4, 0, 0, 0, 0, 0, 0] };
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let b = encode(&f);
            s.write_all(&b[..7]).unwrap();
            std::thread::sleep(Duration::from_millis(120));
            s.write_all(&b[7..]).unwrap();
            let mut r = [0u8; 36];
            s.read_exact(&mut r).unwrap();
            r
        });
        let mut bus = PcanTcp::open("127.0.0.1", port).unwrap();
        let mut got = None;
        for _ in 0..40 {
            if let Some(x) = bus.read().unwrap() {
                got = Some(x);
                break;
            }
        }
        assert_eq!(got, Some(f));
        bus.write(&f).unwrap();
        assert_eq!(t.join().unwrap(), encode(&f));
    }
}
