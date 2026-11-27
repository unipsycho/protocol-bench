//! Dashboard pack — hosts a browser dashboard and feeds it live points.
//!
//! A connection listens on `host:port` and serves, on the one port:
//!
//! * `GET /` and the app's static files (the egui/wasm app built by `scripts/build-dashboard.sh`
//!   into `dist/`, embedded in this library),
//! * `GET /api/health`, `GET /api/dashboards`, `GET|PUT|DELETE /api/dashboards/<name>` (saved
//!   layouts, files in the connection's `data_dir`), `GET /api/history?points=a,b&since=&max=`,
//! * a WebSocket at `/ws`: a [`ServerMsg::Snap`] on connect, then batched [`ServerMsg::Chg`].
//!
//! Which points a browser may see is exactly the connection's **Output** binds.
//! Phase 1 (foundation): serving + data feed. Layout storage, history and control writes follow
//! the plan in `docs/dashboard-plan.md`.

pub mod pack;
pub mod store;
