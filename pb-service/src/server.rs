//! Localhost TCP JSON IPC server (127.0.0.1 only).

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use pb_engine::{AggConfig, CalcConfig, EngineCommand, EngineId};
use pb_ipc::{
    ensure_token, read_json, write_endpoint, write_json, HelloParams, ImportBindingsParams,
    OpenProjectParams, PutProjectParams, Request, Response, SetAggsParams, SetCalcsParams,
    SubmitParams, IpcEndpoint, METHOD_GET_PROJECT, METHOD_HELLO, METHOD_IMPORT_BINDINGS,
    METHOD_OPEN_PROJECT, METHOD_PING, METHOD_STOP_ALL, METHOD_PUT_PROJECT, METHOD_SET_AGGS, METHOD_SET_CALCS,
    METHOD_SNAPSHOT, METHOD_STATUS, METHOD_SUBMIT,
};

use crate::runtime::Runtime;

pub fn serve(runtime: Arc<Mutex<Runtime>>, bind_port: u16) -> Result<(), String> {
    let token = ensure_token()?;
    let listener = TcpListener::bind(("127.0.0.1", bind_port)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    write_endpoint(&IpcEndpoint {
        host: "127.0.0.1".into(),
        port,
    })?;
    eprintln!("pb-service: IPC listening on 127.0.0.1:{port}");
    eprintln!("pb-service: token file ready (config dir ipc.token)");
    let _ = token; // validated on hello

    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let rt = Arc::clone(&runtime);
                thread::spawn(move || {
                    if let Err(err) = handle_client(stream, rt) {
                        eprintln!("pb-service: client ended: {err}");
                    }
                });
            }
            Err(err) => eprintln!("pb-service: accept error: {err}"),
        }
    }
    Ok(())
}

fn handle_client(stream: TcpStream, runtime: Arc<Mutex<Runtime>>) -> Result<(), String> {
    // Replies are small and latency-bound: don't let Nagle delay them.
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(120)))
        .ok();
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut writer = stream;
    let mut authed = false;
    let expected = pb_ipc::read_token()?;

    loop {
        let req: Request = match read_json(&mut reader) {
            Ok(r) => r,
            Err(err) => {
                if err.contains("UnexpectedEof") || err.contains("ended") || err.contains("0") {
                    return Ok(());
                }
                return Err(err);
            }
        };
        let resp = dispatch(&req, Arc::clone(&runtime), &mut authed, &expected);
        write_json(&mut writer, &resp)?;
        writer.flush().ok();
    }
}

fn dispatch(
    req: &Request,
    runtime: Arc<Mutex<Runtime>>,
    authed: &mut bool,
    expected_token: &str,
) -> Response {
    let id = req.id;
    if req.method == METHOD_HELLO {
        let params: HelloParams = match serde_json::from_value(req.params.clone()) {
            Ok(p) => p,
            Err(e) => return Response::err(id, e.to_string()),
        };
        if params.token != expected_token {
            return Response::err(id, "invalid IPC token");
        }
        *authed = true;
        return Response::ok(id, serde_json::json!({ "ok": true }));
    }
    if !*authed {
        return Response::err(id, "hello required");
    }

    let mut rt = match runtime.lock() {
        Ok(g) => g,
        Err(_) => return Response::err(id, "runtime lock poisoned"),
    };

    match req.method.as_str() {
        METHOD_PING => Response::ok(id, serde_json::json!({ "pong": true })),
        METHOD_STOP_ALL => {
            rt.stop_all_hubs();
            Response::ok(id, serde_json::json!({ "ok": true }))
        }
        METHOD_STATUS => match serde_json::to_value(rt.status()) {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, e.to_string()),
        },
        METHOD_SNAPSHOT => match serde_json::to_value(rt.snapshot()) {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, e.to_string()),
        },
        METHOD_SUBMIT => {
            let params: SubmitParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(id, e.to_string()),
            };
            rt.engine.submit(params.engine, params.command);
            Response::ok(id, serde_json::json!({ "ok": true }))
        }
        METHOD_SET_CALCS => {
            let params: SetCalcsParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(id, e.to_string()),
            };
            rt.engine.set_calcs(params.calcs);
            Response::ok(id, serde_json::json!({ "ok": true }))
        }
        METHOD_SET_AGGS => {
            let params: SetAggsParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(id, e.to_string()),
            };
            rt.engine.set_aggs(params.aggs);
            Response::ok(id, serde_json::json!({ "ok": true }))
        }
        METHOD_GET_PROJECT => Response::ok(id, rt.project_json.clone()),
        METHOD_PUT_PROJECT => {
            let params: PutProjectParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(id, e.to_string()),
            };
            let path = params
                .path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            let named = path.is_some();
            match rt.put_project(params.project, params.auto_run_hubs, path) {
                Ok(()) => {
                    if named {
                        let _ = pb_ipc::write_last_project(&rt.project_path);
                    }
                    Response::ok(id, serde_json::json!({ "ok": true }))
                }
                Err(e) => Response::err(id, e),
            }
        }
        METHOD_OPEN_PROJECT => {
            let params: OpenProjectParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(id, e.to_string()),
            };
            let path = if params.path.is_empty() {
                rt.project_path.clone()
            } else {
                PathBuf::from(params.path)
            };
            // Switching projects: stop what the old one started before the new one configures.
            rt.stop_all_hubs();
            match rt.load_project_file(&path, params.auto_run_hubs) {
                Ok(()) => {
                    // Remember it so a restarted service reopens this project, not the default.
                    let _ = pb_ipc::write_last_project(&path);
                    Response::ok(
                        id,
                        serde_json::json!({ "ok": true, "path": path.display().to_string() }),
                    )
                }
                Err(e) => Response::err(id, e),
            }
        }
        METHOD_IMPORT_BINDINGS => {
            let params: ImportBindingsParams = match serde_json::from_value(req.params.clone()) {
                Ok(p) => p,
                Err(e) => return Response::err(id, e.to_string()),
            };
            match rt.import_bindings(params.points, params.next_point_seq) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, e),
            }
        }
        other => Response::err(id, format!("unknown method: {other}")),
    }
}

// Silence unused imports if packs evolve.
#[allow(dead_code)]
fn _types(_: EngineId, _: EngineCommand, _: CalcConfig, _: AggConfig) {}
