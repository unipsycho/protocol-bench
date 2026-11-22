//! Server-side loops: Modbus TCP listener and RTU slave over any `Read + Write` line.
//! Both serve a shared [`RegisterBank`] and call an optional hook after each successful write so
//! the pack can mirror master writes into host points.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::bank::RegisterBank;
use crate::frame::{
    exception_pdu, mbap_frame, mbap_header, rtu_frame, rtu_request_len, rtu_split, Request,
};

/// Called after a write request succeeded (bank already updated).
pub type WriteHook = Arc<dyn Fn(&Request) + Send + Sync>;

/// Most simultaneous TCP masters a listener serves.
pub const MAX_CLIENTS: usize = 32;

#[derive(Clone)]
pub struct ServeCtx {
    pub bank: Arc<Mutex<RegisterBank>>,
    /// Unit id this slave answers to.
    pub unit: u8,
    pub stop: Arc<AtomicBool>,
    pub on_write: Option<WriteHook>,
    /// Connected TCP masters (informational).
    pub clients: Arc<AtomicUsize>,
}

impl ServeCtx {
    pub fn new(bank: Arc<Mutex<RegisterBank>>, unit: u8) -> Self {
        Self {
            bank,
            unit,
            stop: Arc::new(AtomicBool::new(false)),
            on_write: None,
            clients: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Run one request PDU against the bank; returns the response PDU.
    fn execute(&self, pdu: &[u8]) -> Vec<u8> {
        match Request::parse(pdu) {
            Ok(req) => {
                let result = self.bank.lock().map(|mut b| b.handle(&req));
                match result {
                    Ok(Ok(resp)) => {
                        if req.function() >= 5 {
                            if let Some(hook) = &self.on_write {
                                hook(&req);
                            }
                        }
                        req.response_pdu(&resp)
                    }
                    Ok(Err(code)) => exception_pdu(req.function(), code),
                    Err(_) => exception_pdu(req.function(), 4),
                }
            }
            Err(code) => exception_pdu(pdu.first().copied().unwrap_or(0), code),
        }
    }

    fn answers(&self, unit: u8) -> bool {
        unit == self.unit || unit == 0xFF
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

/// Serve one TCP connection until the peer closes or `ctx.stop` is set.
pub fn serve_tcp_stream_ctx(mut stream: TcpStream, ctx: &ServeCtx) {
    let idle = Duration::from_millis(200);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(idle));
    loop {
        if ctx.stopped() {
            return;
        }
        let mut first = [0u8; 1];
        match stream.read(&mut first) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e) if is_timeout(&e) => continue,
            Err(_) => return,
        }
        let mut h = [0u8; 7];
        h[0] = first[0];
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        if stream.read_exact(&mut h[1..]).is_err() {
            return;
        }
        let Ok((tid, rest, unit)) = mbap_header(&h) else {
            return;
        };
        let mut pdu = vec![0u8; rest];
        if stream.read_exact(&mut pdu).is_err() {
            return;
        }
        let _ = stream.set_read_timeout(Some(idle));
        if !ctx.answers(unit) {
            continue; // not our unit: stay silent like a real slave
        }
        let reply = ctx.execute(&pdu);
        if stream.write_all(&mbap_frame(tid, unit, &reply)).is_err() {
            return;
        }
    }
}

/// Accept connections on `listener` until `ctx.stop`. Each master gets its own thread.
pub fn run_tcp_listener(listener: TcpListener, ctx: ServeCtx) {
    let _ = listener.set_nonblocking(true);
    while !ctx.stopped() {
        match listener.accept() {
            Ok((stream, _)) => {
                if ctx.clients.load(Ordering::SeqCst) >= MAX_CLIENTS {
                    continue; // drop: too many masters
                }
                let _ = stream.set_nonblocking(false);
                ctx.clients.fetch_add(1, Ordering::SeqCst);
                let c = ctx.clone();
                std::thread::spawn(move || {
                    serve_tcp_stream_ctx(stream, &c);
                    c.clients.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(30));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// RTU slave loop. `line` must return `TimedOut` / `WouldBlock` when idle (short port timeout).
/// Frames are length-delimited by function code; a partial frame that goes quiet for
/// `resync` is discarded. Returns on `ctx.stop` or a hard I/O error.
pub fn serve_rtu<P: Read + Write>(line: &mut P, ctx: &ServeCtx, resync: Duration) -> Result<(), String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut last_rx = Instant::now();
    let mut chunk = [0u8; 256];
    while !ctx.stopped() {
        match line.read(&mut chunk) {
            Ok(0) => {}
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                last_rx = Instant::now();
            }
            Err(e) if is_timeout(&e) => {
                if !buf.is_empty() && last_rx.elapsed() >= resync {
                    buf.clear();
                }
            }
            Err(e) => return Err(e.to_string()),
        }
        while let Some(len) = rtu_request_len(&buf) {
            if buf.len() < len {
                break;
            }
            let frame: Vec<u8> = buf.drain(..len).collect();
            let Ok((unit, pdu)) = rtu_split(&frame) else {
                buf.clear(); // bad CRC: resync on the next quiet gap
                break;
            };
            if unit != 0 && !ctx.answers(unit) {
                continue;
            }
            let reply = ctx.execute(pdu);
            if unit == 0 {
                continue; // broadcast: execute, never answer
            }
            line.write_all(&rtu_frame(unit, &reply)).map_err(|e| e.to_string())?;
            let _ = line.flush();
        }
    }
    Ok(())
}

/// Bind 127.0.0.1:0 and serve connections on background threads. Tests only.
#[cfg(test)]
pub fn spawn_tcp_for_tests(bank: Arc<Mutex<RegisterBank>>, unit: u8) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let ctx = ServeCtx::new(bank, unit);
    std::thread::spawn(move || run_tcp_listener(listener, ctx));
    addr
}

/// Serve one accepted TCP stream (tests that hand-roll their own listener).
#[cfg(test)]
pub fn serve_tcp_stream(stream: TcpStream, bank: Arc<Mutex<RegisterBank>>, unit: u8) {
    serve_tcp_stream_ctx(stream, &ServeCtx::new(bank, unit));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Area;
    use std::collections::VecDeque;
    use std::io;

    /// Scripted RTU line: input chunks in, written bytes captured; sets `stop` when input is spent.
    struct MockLine {
        input: VecDeque<Vec<u8>>,
        out: Vec<u8>,
        stop: Arc<AtomicBool>,
    }

    impl Read for MockLine {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.input.pop_front() {
                Some(c) => {
                    buf[..c.len()].copy_from_slice(&c);
                    Ok(c.len())
                }
                None => {
                    self.stop.store(true, Ordering::SeqCst);
                    Err(io::Error::new(io::ErrorKind::TimedOut, "idle"))
                }
            }
        }
    }

    impl Write for MockLine {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.out.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn bank() -> Arc<Mutex<RegisterBank>> {
        let mut b = RegisterBank::default();
        b.define(Area::Holding, 0, 8);
        b.define(Area::Coil, 0, 8);
        b.set_regs(Area::Holding, 0, &[10, 20, 30]);
        Arc::new(Mutex::new(b))
    }

    fn run(chunks: Vec<Vec<u8>>, ctx: &ServeCtx) -> Vec<u8> {
        let mut line = MockLine { input: chunks.into(), out: Vec::new(), stop: ctx.stop.clone() };
        serve_rtu(&mut line, ctx, Duration::from_millis(50)).unwrap();
        line.out
    }

    #[test]
    fn rtu_slave_answers_reads_writes_and_split_frames() {
        let ctx = ServeCtx::new(bank(), 5);
        let read = rtu_frame(5, &Request::ReadRegs { area: Area::Holding, start: 0, count: 3 }.pdu());
        let write = rtu_frame(5, &Request::WriteRegs { start: 4, values: vec![7, 8] }.pdu());
        // The read arrives split across two chunks; the write arrives glued to a frame for another unit.
        let other = rtu_frame(9, &Request::ReadRegs { area: Area::Holding, start: 0, count: 1 }.pdu());
        let mut glued = write.clone();
        glued.extend_from_slice(&other);
        let out = run(vec![read[..3].to_vec(), read[3..].to_vec(), glued], &ctx);
        // Reply 1: read; reply 2: write echo; nothing for unit 9.
        let r1 = rtu_frame(5, &[0x03, 0x06, 0, 10, 0, 20, 0, 30]);
        let r2 = rtu_frame(5, &[0x10, 0, 4, 0, 2]);
        assert_eq!(out, [r1, r2].concat());
        assert_eq!(ctx.bank.lock().unwrap().regs(Area::Holding, 4, 2), Some(vec![7, 8]));
    }

    #[test]
    fn rtu_slave_exception_then_bad_crc_is_dropped_then_recovers() {
        let ctx = ServeCtx::new(bank(), 5);
        let bad_addr = rtu_frame(5, &Request::ReadRegs { area: Area::Holding, start: 100, count: 1 }.pdu());
        let mut corrupt = rtu_frame(5, &Request::ReadRegs { area: Area::Holding, start: 0, count: 1 }.pdu());
        let n = corrupt.len();
        corrupt[n - 1] ^= 0xFF;
        let ok = rtu_frame(5, &Request::ReadRegs { area: Area::Holding, start: 1, count: 1 }.pdu());
        let out = run(vec![bad_addr, corrupt, ok], &ctx);
        // Exception for the unmapped read, silence for the corrupt frame, normal reply after.
        let expect = [rtu_frame(5, &[0x83, 0x02]), rtu_frame(5, &[0x03, 0x02, 0, 20])].concat();
        assert_eq!(out, expect);
    }

    #[test]
    fn rtu_broadcast_write_applies_without_reply() {
        let ctx = ServeCtx::new(bank(), 5);
        let broadcast = rtu_frame(0, &Request::WriteReg { addr: 1, value: 99 }.pdu());
        let out = run(vec![broadcast], &ctx);
        assert!(out.is_empty());
        assert_eq!(ctx.bank.lock().unwrap().regs(Area::Holding, 1, 1), Some(vec![99]));
    }

    #[test]
    fn write_hook_fires_only_for_successful_writes() {
        let seen: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let mut ctx = ServeCtx::new(bank(), 5);
        ctx.on_write = Some(Arc::new(move |r| s2.lock().unwrap().push(r.clone())));
        let frames = vec![
            rtu_frame(5, &Request::ReadRegs { area: Area::Holding, start: 0, count: 1 }.pdu()),
            rtu_frame(5, &Request::WriteReg { addr: 2, value: 5 }.pdu()),
            rtu_frame(5, &Request::WriteReg { addr: 200, value: 5 }.pdu()), // unmapped -> exception
        ];
        let _ = run(frames, &ctx);
        assert_eq!(*seen.lock().unwrap(), vec![Request::WriteReg { addr: 2, value: 5 }]);
    }

    #[test]
    fn tcp_listener_stops_and_counts_clients() {
        let bank = bank();
        let ctx = ServeCtx::new(bank, 1);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let c = ctx.clone();
        let h = std::thread::spawn(move || run_tcp_listener(listener, c));
        let s = TcpStream::connect(addr).unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        while ctx.clients.load(Ordering::SeqCst) == 0 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(ctx.clients.load(Ordering::SeqCst), 1);
        ctx.stop.store(true, Ordering::SeqCst);
        h.join().unwrap();
        drop(s);
    }
}
