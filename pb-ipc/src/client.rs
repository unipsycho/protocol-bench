//! UI-side localhost TCP client (no in-process engine).

use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pb_engine::{
    AggConfig, AggRuntime, CalcConfig, CalcRuntime, EngineCommand, EngineId, HostSnapshot,
};
use pb_points::PointStore;

use crate::frame::{read_json, write_json};
use crate::paths::{self, IpcEndpoint};
use crate::proto::*;

/// Drop-in style handle used by the UI instead of `EngineHandle::spawn`.
pub struct RemoteHandle {
    stream: Mutex<TcpStream>,
    next_id: AtomicU64,
    points: Arc<Mutex<PointStore>>,
    calc_rt: Arc<Mutex<CalcRuntime>>,
    agg_rt: Arc<Mutex<AggRuntime>>,
    pub last_status: Mutex<ServiceStatus>,
}

impl RemoteHandle {
    /// Connect using `ipc.json` + `ipc.token` from the config dir.
    pub fn connect() -> Result<Self, String> {
        let ep = paths::read_endpoint()?;
        let token = paths::read_token()?;
        Self::connect_to(&ep, &token)
    }

    pub fn connect_to(ep: &IpcEndpoint, token: &str) -> Result<Self, String> {
        let addr = format!("{}:{}", ep.host, ep.port);
        let stream = TcpStream::connect(&addr).map_err(|e| {
            format!(
                "cannot connect to Protocol Bench service at {addr}: {e}\n\
                 Start it first: PB_MODULES=./modules cargo run -p pb-service"
            )
        })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .ok();
        let handle = Self {
            stream: Mutex::new(stream),
            next_id: AtomicU64::new(1),
            points: Arc::new(Mutex::new(PointStore::new())),
            calc_rt: Arc::new(Mutex::new(CalcRuntime::default())),
            agg_rt: Arc::new(Mutex::new(AggRuntime::default())),
            last_status: Mutex::new(ServiceStatus::default()),
        };
        handle.call(
            METHOD_HELLO,
            serde_json::to_value(HelloParams {
                token: token.into(),
                client: "pb-ui".into(),
            })
            .unwrap_or(serde_json::Value::Null),
        )?;
        Ok(handle)
    }

    fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = Request {
            id,
            method: method.into(),
            params,
        };
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| "IPC stream lock poisoned".to_string())?;
        write_json(&mut *stream, &req)?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        // Keep owning stream locked while reading the matching response.
        let resp: Response = read_json(&mut reader)?;
        if resp.id != id {
            return Err(format!(
                "IPC id mismatch: sent {id}, got {}",
                resp.id
            ));
        }
        if let Some(err) = resp.error {
            return Err(err);
        }
        Ok(resp.result.unwrap_or(serde_json::Value::Null))
    }

    pub fn ping(&self) -> Result<(), String> {
        self.call(METHOD_PING, serde_json::Value::Null).map(|_| ())
    }

    pub fn status(&self) -> Result<ServiceStatus, String> {
        let v = self.call(METHOD_STATUS, serde_json::Value::Null)?;
        let st: ServiceStatus = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if let Ok(mut g) = self.last_status.lock() {
            *g = st.clone();
        }
        Ok(st)
    }

    pub fn snapshot(&self) -> HostSnapshot {
        match self.call(METHOD_SNAPSHOT, serde_json::Value::Null) {
            Ok(v) => serde_json::from_value(v).unwrap_or_else(|_| empty_snap()),
            Err(_) => empty_snap(),
        }
    }

    pub fn submit(&self, id: EngineId, command: EngineCommand) {
        let _ = self.call(
            METHOD_SUBMIT,
            serde_json::to_value(SubmitParams {
                engine: id,
                command,
            })
            .unwrap_or(serde_json::Value::Null),
        );
    }

    pub fn set_calcs(&self, next: Vec<CalcConfig>) {
        let _ = self.call(
            METHOD_SET_CALCS,
            serde_json::to_value(SetCalcsParams { calcs: next })
                .unwrap_or(serde_json::Value::Null),
        );
    }

    pub fn set_aggs(&self, next: Vec<AggConfig>) {
        let _ = self.call(
            METHOD_SET_AGGS,
            serde_json::to_value(SetAggsParams { aggs: next }).unwrap_or(serde_json::Value::Null),
        );
    }

    pub fn points(&self) -> Arc<Mutex<PointStore>> {
        Arc::clone(&self.points)
    }

    pub fn calc_runtime(&self) -> Arc<Mutex<CalcRuntime>> {
        Arc::clone(&self.calc_rt)
    }

    pub fn agg_runtime(&self) -> Arc<Mutex<AggRuntime>> {
        Arc::clone(&self.agg_rt)
    }

    pub fn get_project_json(&self) -> Result<serde_json::Value, String> {
        self.call(METHOD_GET_PROJECT, serde_json::Value::Null)
    }

    pub fn put_project_json(
        &self,
        project: serde_json::Value,
        auto_run_hubs: bool,
    ) -> Result<(), String> {
        self.call(
            METHOD_PUT_PROJECT,
            serde_json::to_value(PutProjectParams {
                project,
                auto_run_hubs,
            })
            .map_err(|e| e.to_string())?,
        )
        .map(|_| ())
    }

    pub fn open_project(&self, path: &str, auto_run_hubs: bool) -> Result<(), String> {
        self.call(
            METHOD_OPEN_PROJECT,
            serde_json::to_value(OpenProjectParams {
                path: path.into(),
                auto_run_hubs,
            })
            .map_err(|e| e.to_string())?,
        )
        .map(|_| ())
    }

    pub fn import_bindings(
        &self,
        points: Vec<PointBindingWire>,
        next_point_seq: u64,
    ) -> Result<(), String> {
        self.call(
            METHOD_IMPORT_BINDINGS,
            serde_json::to_value(ImportBindingsParams {
                points,
                next_point_seq,
            })
            .map_err(|e| e.to_string())?,
        )
        .map(|_| ())
    }

    /// No-op join — service outlives the UI.
    pub fn shutdown(&mut self) {}
}

fn empty_snap() -> HostSnapshot {
    HostSnapshot {
        thread_name: "disconnected".into(),
        alive: false,
        ticks: 0,
        uptime_ms: 0,
        slots: Vec::new(),
        modules_dir: String::new(),
        packs_loaded: Vec::new(),
        point_count: 0,
        points: Vec::new(),
    }
}

impl Drop for RemoteHandle {
    fn drop(&mut self) {
        // Leave the service running; just close the TCP socket.
        if let Ok(mut s) = self.stream.lock() {
            let _ = s.flush();
        }
    }
}
