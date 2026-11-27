//! Built-in virtual CAN bus: an in-process stand-in for Linux `vcan`.
//!
//! Every [`VirtualPort`] that joins the same bus name sees every frame written by the *other*
//! ports on that bus, never its own — the same rule SocketCAN uses. No kernel module, no root,
//! works on every platform. Only reaches ports inside the same process.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::{CanBus, Frame};

type Members = Vec<(u64, Sender<Frame>)>;

fn buses() -> &'static Mutex<(u64, HashMap<String, Members>)> {
    static B: OnceLock<Mutex<(u64, HashMap<String, Members>)>> = OnceLock::new();
    B.get_or_init(|| Mutex::new((0, HashMap::new())))
}

/// One member of a named virtual bus. Leaves the bus when dropped.
pub struct VirtualPort {
    bus: String,
    id: u64,
    rx: Receiver<Frame>,
    timeout: Duration,
}

impl VirtualPort {
    pub fn join(bus: &str) -> VirtualPort {
        let (tx, rx) = channel();
        let mut g = buses().lock().unwrap_or_else(|e| e.into_inner());
        g.0 += 1;
        let id = g.0;
        g.1.entry(bus.to_string()).or_default().push((id, tx));
        VirtualPort { bus: bus.to_string(), id, rx, timeout: Duration::from_millis(50) }
    }

    /// Ports currently on `bus` (this one included).
    pub fn members(bus: &str) -> usize {
        buses().lock().unwrap_or_else(|e| e.into_inner()).1.get(bus).map_or(0, |m| m.len())
    }
}

impl CanBus for VirtualPort {
    fn read(&mut self) -> io::Result<Option<Frame>> {
        match self.rx.recv_timeout(self.timeout) {
            Ok(f) => Ok(Some(f)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(io::ErrorKind::BrokenPipe, "virtual bus closed")),
        }
    }

    fn write(&mut self, f: &Frame) -> io::Result<()> {
        let g = buses().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(members) = g.1.get(&self.bus) {
            for (id, tx) in members {
                if *id != self.id {
                    let _ = tx.send(*f);
                }
            }
        }
        Ok(())
    }
}

impl Drop for VirtualPort {
    fn drop(&mut self) {
        let mut g = buses().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = g.1.get_mut(&self.bus) {
            m.retain(|(id, _)| *id != self.id);
            if m.is_empty() {
                g.1.remove(&self.bus);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_reach_other_ports_not_the_sender() {
        let mut a = VirtualPort::join("t-echo");
        let mut b = VirtualPort::join("t-echo");
        let mut c = VirtualPort::join("t-echo");
        let mut other = VirtualPort::join("t-other");
        let f = Frame { id: 0x123, extended: false, dlc: 2, data: [1, 2, 0, 0, 0, 0, 0, 0] };
        a.write(&f).unwrap();
        assert_eq!(b.read().unwrap(), Some(f));
        assert_eq!(c.read().unwrap(), Some(f));
        assert_eq!(a.read().unwrap(), None, "no echo to the sender");
        assert_eq!(other.read().unwrap(), None, "other buses are separate");
        assert_eq!(VirtualPort::members("t-echo"), 3);
        drop(c);
        assert_eq!(VirtualPort::members("t-echo"), 2);
        drop((a, b));
        assert_eq!(VirtualPort::members("t-echo"), 0);
    }
}
