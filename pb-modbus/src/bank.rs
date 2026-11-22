//! Sparse register bank + request handler. Backs the server (phase 4) and the loopback tests.
//! A request is served only when **every** addressed point exists, else exception 2.

use std::collections::HashMap;

use crate::codec::Area;
use crate::frame::{Request, Response, EX_ILLEGAL_ADDRESS};

#[derive(Debug, Default, Clone)]
pub struct RegisterBank {
    pub coils: HashMap<u16, bool>,
    pub discrete: HashMap<u16, bool>,
    pub input: HashMap<u16, u16>,
    pub holding: HashMap<u16, u16>,
}

impl RegisterBank {
    /// Define `count` zeroed addresses so they are served.
    pub fn define(&mut self, area: Area, start: u16, count: u16) {
        for a in start..start.saturating_add(count) {
            match area {
                Area::Coil => {
                    self.coils.entry(a).or_insert(false);
                }
                Area::Discrete => {
                    self.discrete.entry(a).or_insert(false);
                }
                Area::Input => {
                    self.input.entry(a).or_insert(0);
                }
                Area::Holding => {
                    self.holding.entry(a).or_insert(0);
                }
            }
        }
    }

    pub fn set_regs(&mut self, area: Area, start: u16, values: &[u16]) {
        for (i, v) in values.iter().enumerate() {
            let a = start.wrapping_add(i as u16);
            match area {
                Area::Input => self.input.insert(a, *v),
                _ => self.holding.insert(a, *v),
            };
        }
    }

    pub fn regs(&self, area: Area, start: u16, count: u16) -> Option<Vec<u16>> {
        let map = if area == Area::Input { &self.input } else { &self.holding };
        (0..count).map(|i| map.get(&start.wrapping_add(i)).copied()).collect()
    }

    pub fn bits(&self, area: Area, start: u16, count: u16) -> Option<Vec<bool>> {
        let map = if area == Area::Discrete { &self.discrete } else { &self.coils };
        (0..count).map(|i| map.get(&start.wrapping_add(i)).copied()).collect()
    }

    /// Execute one request. `Err(code)` becomes an exception response.
    pub fn handle(&mut self, req: &Request) -> Result<Response, u8> {
        match req {
            Request::ReadBits { area, start, count } => {
                self.bits(*area, *start, *count).map(Response::Bits).ok_or(EX_ILLEGAL_ADDRESS)
            }
            Request::ReadRegs { area, start, count } => {
                self.regs(*area, *start, *count).map(Response::Regs).ok_or(EX_ILLEGAL_ADDRESS)
            }
            Request::WriteCoil { addr, on } => {
                let slot = self.coils.get_mut(addr).ok_or(EX_ILLEGAL_ADDRESS)?;
                *slot = *on;
                Ok(Response::Written)
            }
            Request::WriteReg { addr, value } => {
                let slot = self.holding.get_mut(addr).ok_or(EX_ILLEGAL_ADDRESS)?;
                *slot = *value;
                Ok(Response::Written)
            }
            Request::WriteCoils { start, values } => {
                if (0..values.len()).any(|i| !self.coils.contains_key(&start.wrapping_add(i as u16))) {
                    return Err(EX_ILLEGAL_ADDRESS);
                }
                for (i, v) in values.iter().enumerate() {
                    self.coils.insert(start.wrapping_add(i as u16), *v);
                }
                Ok(Response::Written)
            }
            Request::WriteRegs { start, values } => {
                if (0..values.len()).any(|i| !self.holding.contains_key(&start.wrapping_add(i as u16))) {
                    return Err(EX_ILLEGAL_ADDRESS);
                }
                self.set_regs(Area::Holding, *start, values);
                Ok(Response::Written)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_only_defined_addresses() {
        let mut b = RegisterBank::default();
        b.define(Area::Holding, 10, 4);
        b.set_regs(Area::Holding, 10, &[1, 2, 3, 4]);
        assert_eq!(
            b.handle(&Request::ReadRegs { area: Area::Holding, start: 11, count: 2 }),
            Ok(Response::Regs(vec![2, 3]))
        );
        assert_eq!(
            b.handle(&Request::ReadRegs { area: Area::Holding, start: 13, count: 2 }),
            Err(EX_ILLEGAL_ADDRESS)
        );
        // Failed multi-write changes nothing.
        assert_eq!(
            b.handle(&Request::WriteRegs { start: 13, values: vec![9, 9] }),
            Err(EX_ILLEGAL_ADDRESS)
        );
        assert_eq!(b.regs(Area::Holding, 13, 1), Some(vec![4]));
        assert_eq!(b.handle(&Request::WriteReg { addr: 13, value: 7 }), Ok(Response::Written));
        assert_eq!(b.regs(Area::Holding, 13, 1), Some(vec![7]));
    }

    #[test]
    fn coils_read_and_write() {
        let mut b = RegisterBank::default();
        b.define(Area::Coil, 0, 3);
        b.handle(&Request::WriteCoil { addr: 1, on: true }).unwrap();
        assert_eq!(
            b.handle(&Request::ReadBits { area: Area::Coil, start: 0, count: 3 }),
            Ok(Response::Bits(vec![false, true, false]))
        );
        assert_eq!(b.handle(&Request::WriteCoil { addr: 9, on: true }), Err(EX_ILLEGAL_ADDRESS));
    }
}
