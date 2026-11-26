//! WebSocket pack — JSON over `ws://` / `wss://`.
//!
//! A connection is either a **client** (dials a URL, optionally sends a subscribe message after
//! connecting) or a **server** (listens on host:port; every browser / dashboard that connects
//! gets the Output messages — handy for live web demos).
//!
//! * **Input** binds take a JSON path (`data.temp`, `items[0].value`, empty = whole message)
//!   from each received text message → `ws_N`. Optional scale / offset for numbers.
//! * **Output** binds send host points as JSON. `out_format = object` sends one object with
//!   every bound key (`{"ts":…, "pressure": 3400, …}`) each Output interval when something
//!   changed; `change` sends one `{"point","key","value","ts"}` message per change.

pub mod pack;
