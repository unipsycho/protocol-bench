//! Protocol Bench shell: menu bar, icon rail, docked panes, themes.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Layout, Pos2, Rect, Rounding, Sense, Stroke, Vec2,
};
use crate::engine_api::EngineHandle;
use pb_engine::HostSnapshot;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::art::{self, ChromeIcon};
use crate::brand_logo::AaLogoCache;
use crate::dnd::{self, DropCommit, PaneHit};
use crate::dock::DockPane;
use crate::layout::{LayoutNode, SplitDir, TabGroup};
use crate::http_panel::HttpPanelState;
use crate::las_panel::LasPanelState;
use crate::mqtt_panel::{self, MqttPanelState};
use crate::opcua_panel::{self, OpcUaPanelState};
use crate::panels::PointsPanelState;
use crate::files_panel::FilesPanelState;
use crate::modbus_panel::ModbusPanelState;
use crate::snmp_panel::SnmpPanelState;
use crate::streams_panel::StreamsPanelState;
use crate::traffic_panel::TrafficPanelState;
use crate::panels;
use crate::persist::{self, UiState};
use crate::project::{
    self, DockLayout, FilesProject, HttpProject, LasProject, ModbusProject, MqttProject, ProjectFile, ProjectSettings, SnmpProject,
    StreamsProject,
};
use pb_ipc::PointBindingWire;
use crate::theme::{
    self, apply_form_spacing, divider_bar, font_semi, font_ui, space, tint, AppTheme, ThemePalette,
    BAR_H, HEADER_H, MIN_PANE, RADIUS, RAIL_W, SEP, STATUS_H,
};

/// Product slogan from earlier shell About copy (stashed WIP).
const PRODUCT_SLOGAN: &str = "Your protocol hub for every stream.";
const COMPANY_URL: &str = "https://www.agileautomation.ca";

pub struct ProtocolBenchApp {
    engine: EngineHandle,
    ui_state: UiState,
    theme: AppTheme,
    theme_applied: Option<AppTheme>,
    pane_hits: Vec<PaneHit>,
    pending: Option<DropCommit>,
    about_open: bool,
    settings_open: bool,
    menu: Option<BarMenu>,
    status: String,
    /// True when localhost IPC to pb-service is up (Decision #8).
    service_connected: bool,
    /// Last service running report (hubs / packs).
    service_report: String,
    aa_logo: AaLogoCache,
    dirty: bool,
    last_save: Instant,
    mqtt: MqttPanelState,
    opcua: OpcUaPanelState,
    http: HttpPanelState,
    streams: StreamsPanelState,
    las: LasPanelState,
    snmp: SnmpPanelState,
    modbus: ModbusPanelState,
    files: FilesPanelState,
    points: PointsPanelState,
    traffic: TrafficPanelState,
    /// Project-level options (Project panel / project.json).
    project_settings: ProjectSettings,
    /// Last settings snapshot written to the project file.
    settings_saved: ProjectSettings,
    /// Current JSON project path (MQTT + hubs + points).
    project_path: PathBuf,
    /// Last MQTT snapshot written to the project file.
    mqtt_saved: MqttProject,
    /// Last HTTP clients snapshot written to the project file.
    http_saved: HttpProject,
    /// Last Streams / TCP endpoints snapshot written to the project file.
    streams_saved: StreamsProject,
    /// Last SNMP targets snapshot written to the project file.
    las_saved: LasProject,
    snmp_saved: SnmpProject,
    modbus_saved: ModbusProject,
    files_saved: FilesProject,
    /// Last hubs snapshot written to the project file.
    hubs_saved: crate::hubs::HubModel,
    /// Last dock layout written to project.json (PB-95).
    layout_saved: DockLayout,
    last_project_save: Instant,
    /// Native open/save dialog running off the UI thread (Hyprland-safe).
    file_pick: Option<PendingFilePick>,
    /// Open-project request waiting on the "this will stop" confirmation.
    pending_switch: Option<PendingSwitch>,
    /// Quit request waiting on confirmation (what keeps running in the service).
    pending_quit: Option<Vec<String>>,
    /// Set once the user confirmed quitting, so the close request is let through.
    quit_confirmed: bool,
    /// Hubs were stopped for quit: never autosave again (a save re-applies the project).
    stopped_all: bool,
    /// Live host snapshot for Calc/Agg/points — refreshed on poll, not mouse FPS (PB-64).
    live_host: HostSnapshot,
    live_points_fp: u64,
    last_live_poll: Instant,
    /// Last IPC status probe (chip + bottom report).
    last_service_poll: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FilePickKind {
    Open,
    SaveAs,
}

/// An Open / Open Recent request held back until the user confirms stopping live hubs.
struct PendingSwitch {
    /// Project file to open once confirmed.
    path: PathBuf,
    /// What the service is running now and will stop on switch.
    running: Vec<String>,
}

struct PendingFilePick {
    kind: FilePickKind,
    rx: Receiver<Option<PathBuf>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BarMenu {
    File,
    View,
    Help,
    Theme,
}

enum MenuAction {
    OpenProject,
    SaveProject,
    SaveAsProject,
    OpenRecent(PathBuf),
    ClearRecent,
    ResetLayout,
    Quit,
    About,
    Settings,
    Toggle(DockPane),
    ShowAll,
    Theme(AppTheme),
    Close,
}

struct Anchor {
    rect: Rect,
    clicked: bool,
}

impl ProtocolBenchApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::install_chrome_style(&cc.egui_ctx);
        let ui_state = persist::load();
        let theme = persist::theme_from_state(&ui_state);
        theme::apply_theme(&cc.egui_ctx, theme);
        let mut project_path = project::default_project_path().unwrap_or_else(|| {
            PathBuf::from("protocol-bench-project.json")
        });
        let mut mqtt = MqttPanelState::default();
        let mut opcua = OpcUaPanelState::default();
        let mut http = HttpPanelState::default();
        let mut streams = StreamsPanelState::default();
        let mut las = LasPanelState::default();
        let mut snmp = SnmpPanelState::default();
        let mut modbus = ModbusPanelState::default();
        let mut files = FilesPanelState::default();
        let mut mqtt_saved = MqttProject::default();
        let mut http_saved = HttpProject::default();
        let mut streams_saved = StreamsProject::default();
        let mut las_saved = LasProject::default();
        let mut snmp_saved = SnmpProject::default();
        let mut modbus_saved = ModbusProject::default();
        let mut files_saved = FilesProject::default();
        let mut project_settings = ProjectSettings::default();
        // Quiet offline: chip shows disconnected; left status stays calm (PB-101).
        let mut status = "Ready".into();
        let mut service_connected = false;
        let mut service_report = String::new();
        // Prefer live IPC; UI still opens when pb-service is down (auto-probe reconnects).
        let engine = match EngineHandle::connect() {
            Ok(h) => {
                service_connected = true;
                h
            }
            Err(err) => {
                eprintln!("protocol-bench: {err}");
                EngineHandle::disconnected()
            }
        };
        let mut ui_state = ui_state;
        let mut seed_layout = false;
        let mut auto_run = ui_state.auto_run_embedded_hubs;
        if service_connected {
            // The service owns the active project: attach to whatever it is running instead of
            // re-opening the default (that would swap out the project and its live hubs).
            // Only a service with nothing live gets an explicit open, so auto-run still fires.
            let probe = engine.status().ok();
            let mut attached = false;
            if let Some(st) = probe.as_ref() {
                if !st.project_path.is_empty() {
                    project_path = PathBuf::from(&st.project_path);
                }
                attached = !st.running_hubs().is_empty();
            }
            if !attached {
                let open_path = project_path.display().to_string();
                let _ = engine.open_project(&open_path, auto_run);
            }
            match engine.get_project_json() {
                Ok(value) => match serde_json::from_value::<ProjectFile>(value) {
                    Ok(file) => {
                        file.mqtt.apply_to_panel(&mut mqtt);
                        file.opcua.apply_to_panel(&mut opcua);
                        file.http.apply_to_panel(&mut http);
                        file.streams.apply_to_panel(&mut streams);
                        file.las.apply_to_panel(&mut las);
                        file.snmp.apply_to_panel(&mut snmp);
                        file.modbus.apply_to_panel(&mut modbus);
                        file.files.apply_to_panel(&mut files);
                        project_settings = file.settings.clone();
                        // PB-58: legacy project auto-run → app setting (ui.toml).
                        if file.settings.auto_run_embedded_broker && !ui_state.auto_run_embedded_hubs
                        {
                            ui_state.auto_run_embedded_hubs = true;
                        }
                        auto_run = ui_state.auto_run_embedded_hubs;
                        // PB-95: project dock layout, or seed from ui.toml once.
                        seed_layout = project::apply_or_seed_layout(&file, &mut ui_state);
                        if let Some(hubs) = file.hubs {
                            ui_state.hubs = hubs;
                            ui_state.hubs.sanitize();
                        }
                        if let Ok(mut store) = engine.points().lock() {
                            project::import_bindings_to_store(
                                &mut store,
                                &file.points,
                                file.next_point_seq,
                            );
                            project::migrate_panel_point_refs(&mut mqtt, &mut store);
                        }
                        project::migrate_client_binds_to_hubs(&mut mqtt, &mut ui_state.hubs);
                        // Persist rebuilt points[] when file was empty after IPC wipe.
                        seed_layout = seed_layout || file.points.is_empty();
                        mqtt_saved = MqttProject::from_panel(&mqtt);
                        http_saved = HttpProject::from_panel(&http);
                        streams_saved = StreamsProject::from_panel(&streams);
                        las_saved = LasProject::from_panel(&las);
                        snmp_saved = SnmpProject::from_panel(&snmp);
                        modbus_saved = ModbusProject::from_panel(&modbus);
                        files_saved = FilesProject::from_panel(&files);
                        // Repair orphan hub mqtt_connection ids before materializing points.
                        if mqtt_panel::apply_mqtt_session(&engine, &mut mqtt, &mut ui_state.hubs) {
                            seed_layout = true;
                        }
                        // IPC: local PointStore is a mirror — promote hub binds and push to service.
                        Self::push_hub_binds_to_service(&engine, &mut ui_state.hubs);
                        // Attached to live hubs: never auto-start over them.
                        let auto_start = auto_run && !attached;
                        mqtt_panel::maybe_auto_start_embedded(
                            &engine,
                            &mut mqtt,
                            &mut ui_state.hubs,
                            auto_start,
                        );
                        opcua_panel::apply_opcua_session(&engine, &mut opcua, &ui_state.hubs);
                        opcua_panel::maybe_auto_start_embedded(&engine, &mut opcua, auto_start);
                        status = format!("Connected · project {}", project_path.display());
                    }
                    Err(err) => status = format!("Project decode: {err}"),
                },
                Err(err) => status = format!("Get project: {err}"),
            }
            if let Ok(st) = engine.status() {
                service_report = Self::format_service_report(&st);
                status = format!("Service · {service_report}");
            }
        }
        let hubs_saved = ui_state.hubs.clone();
        let settings_saved = project_settings.clone();
        let layout_saved = DockLayout::from_ui(&ui_state);
        let live_host = engine.snapshot();
        let live_points_fp = Self::points_fingerprint(&live_host);
        let mut app = Self {
            engine,
            ui_state,
            theme,
            theme_applied: Some(theme),
            pane_hits: Vec::new(),
            pending: None,
            about_open: false,
            settings_open: std::env::var_os("PB_OPEN_SETTINGS").is_some(),
            menu: None,
            status,
            service_connected,
            service_report,
            aa_logo: AaLogoCache::default(),
            dirty: seed_layout,
            last_save: Instant::now(),
            mqtt,
            opcua,
            http,
            streams,
            las,
            snmp,
            modbus,
            files,
            points: PointsPanelState::default(),
            traffic: TrafficPanelState::default(),
            project_settings,
            settings_saved,
            project_path,
            mqtt_saved,
            http_saved,
            streams_saved,
            las_saved,
            snmp_saved,
            modbus_saved,
            files_saved,
            hubs_saved,
            layout_saved,
            last_project_save: Instant::now(),
            file_pick: None,
            pending_switch: None,
            pending_quit: None,
            quit_confirmed: false,
            stopped_all: false,
            live_host,
            live_points_fp,
            last_live_poll: Instant::now(),
            last_service_poll: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now),
        };
        if persist::note_recent_project(&mut app.ui_state, &app.project_path) {
            app.mark_dirty();
        }
        if seed_layout {
            let _ = app.save_project_now();
        }
        app
    }

    fn format_service_report(st: &pb_ipc::ServiceStatus) -> String {
        format!(
            "packs={:?} pts={} mqtt_hub={} opcua_hub={}",
            st.packs_loaded, st.point_count, st.mqtt_hub_running, st.opcua_hub_running
        )
    }

    /// Periodic IPC probe: chip + bottom rate without a click.
    fn poll_service_link(&mut self) {
        const SERVICE_POLL: Duration = Duration::from_millis(1500);
        if self.last_service_poll.elapsed() < SERVICE_POLL {
            return;
        }
        self.last_service_poll = Instant::now();
        match self.engine.probe_status() {
            Ok(st) => {
                let report = Self::format_service_report(&st);
                let was = self.service_connected;
                self.service_connected = true;
                // Only refresh left status when report changes (or we just reconnected).
                if !was || self.service_report != report {
                    self.service_report = report.clone();
                    self.status = format!("Service · {report}");
                }
            }
            Err(_) => {
                // Chip is enough when offline — do not spam the left status (PB-101).
                if self.service_connected {
                    self.service_connected = false;
                    self.service_report.clear();
                    if self.status.starts_with("Service ·")
                        || self.status.starts_with("Could not save project")
                        || self.status.starts_with("Service disconnected")
                    {
                        self.status = "Ready".into();
                    }
                }
            }
        }
    }

    /// Fingerprint of live point values/stamps (drives preview + table refresh).
    fn points_fingerprint(host: &HostSnapshot) -> u64 {
        let mut h = DefaultHasher::new();
        host.point_count.hash(&mut h);
        for p in &host.points {
            p.name.hash(&mut h);
            p.value.hash(&mut h);
            p.quality.hash(&mut h);
            p.updated_ms.hash(&mut h);
        }
        // MQTT traffic / connection (not host ticks — those advance every poll).
        for slot in &host.slots {
            slot.connected.hash(&mut h);
            slot.detail_json.hash(&mut h);
            for topic in &slot.topics {
                topic.hash(&mut h);
            }
        }
        h.finish()
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.ui_state.theme_id = self.theme.id().into();
    }

    fn autosave(&mut self) {
        if !self.dirty || self.last_save.elapsed() < Duration::from_millis(400) {
            return;
        }
        self.ui_state.theme_id = self.theme.id().into();
        match persist::save(&self.ui_state) {
            Ok(()) => self.dirty = false,
            Err(err) => self.status = format!("Could not save layout: {err}"),
        }
        self.last_save = Instant::now();
    }

    fn autosave_project(&mut self, _host: &pb_engine::HostSnapshot) {
        // Offline: chip covers it — never hammer IPC or flash save errors (PB-101).
        if !self.service_connected || self.stopped_all {
            return;
        }
        let current = MqttProject::from_panel(&self.mqtt);
        let mqtt_dirty = current != self.mqtt_saved;
        let http_dirty = HttpProject::from_panel(&self.http) != self.http_saved;
        let streams_dirty = StreamsProject::from_panel(&self.streams) != self.streams_saved;
        let las_dirty = LasProject::from_panel(&self.las) != self.las_saved;
        let snmp_dirty = SnmpProject::from_panel(&self.snmp) != self.snmp_saved;
        let modbus_dirty = ModbusProject::from_panel(&self.modbus) != self.modbus_saved;
        let files_dirty = FilesProject::from_panel(&self.files) != self.files_saved;
        let hubs_dirty = self.ui_state.hubs != self.hubs_saved;
        let settings_dirty = self.project_settings != self.settings_saved;
        let layout_dirty = DockLayout::from_ui(&self.ui_state) != self.layout_saved;
        // Save only when something actually changed — no timer flush when clean.
        if !(mqtt_dirty
            || http_dirty
            || streams_dirty
            || las_dirty
            || snmp_dirty
            || modbus_dirty
            || files_dirty
            || hubs_dirty
            || settings_dirty
            || layout_dirty)
        {
            return;
        }
        if self.last_project_save.elapsed() < Duration::from_millis(500) {
            return;
        }
        match self.save_project_now() {
            Ok(()) => {}
            Err(err) => {
                // One message if the link died mid-save; probe will quiet the bar.
                self.status = format!("Could not save project: {err}");
                self.service_connected = false;
            }
        }
    }

    /// Materialize Hub Input/agg binds into the local mirror and push to pb-service.
    /// Fixes empty Points panel after IPC: RemoteHandle.points() is not the service store.
    fn push_hub_binds_to_service(engine: &EngineHandle, hubs: &mut crate::hubs::HubModel) {
        let (wire, next) = match engine.points().lock() {
            Ok(mut store) => {
                let _ = project::materialize_hub_point_binds(hubs, &mut store);
                let wire: Vec<PointBindingWire> = store
                    .export_bindings()
                    .into_iter()
                    .map(|b| PointBindingWire {
                        id: b.id,
                        source: b.source,
                    })
                    .collect();
                (wire, store.next_seq())
            }
            Err(_) => return,
        };
        let _ = engine.import_bindings(wire, next);
    }

    fn save_project_now(&mut self) -> Result<(), String> {
        // Promote Input binds → point bus before reading bindings for project.json.
        // Without this, the empty IPC mirror wiped `points: []` on every hubs autosave.
        Self::push_hub_binds_to_service(&self.engine, &mut self.ui_state.hubs);
        let mqtt = MqttProject::from_panel(&self.mqtt);
        let layout = DockLayout::from_ui(&self.ui_state);
        let (points, next_point_seq) = self
            .engine
            .points()
            .lock()
            .map(|store| {
                (
                    project::bindings_from_store(&store),
                    store.next_seq(),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), 1));
        let http = HttpProject::from_panel(&self.http);
        let streams = StreamsProject::from_panel(&self.streams);
        let las = LasProject::from_panel(&self.las);
        let snmp = SnmpProject::from_panel(&self.snmp);
        let modbus = ModbusProject::from_panel(&self.modbus);
        let files = FilesProject::from_panel(&self.files);
        let file = ProjectFile {
            version: project::PROJECT_VERSION,
            points,
            next_point_seq,
            mqtt: mqtt.clone(),
            opcua: project::OpcUaProject::from_panel(&self.opcua),
            http: http.clone(),
            streams: streams.clone(),
            las: las.clone(),
            snmp: snmp.clone(),
            modbus: modbus.clone(),
            files: files.clone(),
            hubs: Some(self.ui_state.hubs.clone()),
            settings: self.project_settings.clone(),
            layout: Some(layout.clone()),
        };
        // Service owns project.json — persist only via IPC (pass path so Save as retargets).
        let value = serde_json::to_value(&file).map_err(|e| e.to_string())?;
        let path = Some(self.project_path.display().to_string());
        self.engine.put_project_json(
            value,
            self.ui_state.auto_run_embedded_hubs,
            path,
        )?;
        self.mqtt_saved = mqtt;
        self.http_saved = http;
        self.streams_saved = streams;
        self.las_saved = las;
        self.snmp_saved = snmp;
        self.modbus_saved = modbus;
        self.files_saved = files;
        self.hubs_saved = self.ui_state.hubs.clone();
        self.settings_saved = self.project_settings.clone();
        self.layout_saved = layout;
        self.last_project_save = Instant::now();
        if persist::note_recent_project(&mut self.ui_state, &self.project_path) {
            self.mark_dirty();
        }
        Ok(())
    }

    /// What the service is running right now (empty when idle or when the service is unreachable).
    /// Drives the "this will stop" / "this keeps running" confirmations.
    fn running_hubs(&self) -> Vec<String> {
        self.engine
            .probe_status()
            .map(|st| st.running_hubs())
            .unwrap_or_default()
    }

    /// Open a project, warning first when the service has live hubs the switch will stop.
    fn request_project_open(&mut self, path: PathBuf) {
        let running = self.running_hubs();
        if running.is_empty() {
            if let Err(err) = self.load_project_from_path(path) {
                self.status = format!("Open failed: {err}");
            }
        } else {
            self.pending_switch = Some(PendingSwitch { path, running });
        }
    }

    /// Quit: when the service still has hubs live, say they keep running after the UI exits.
    fn request_quit(&mut self, ctx: &egui::Context) {
        let running = self.running_hubs();
        if running.is_empty() {
            self.quit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.pending_quit = Some(running);
        }
    }

    /// Modal confirmations for project switch and quit.
    fn confirm_dialogs(&mut self, ctx: &egui::Context) {
        let p = self.theme.palette();
        if let Some(sw) = self.pending_switch.as_ref() {
            let path = sw.path.clone();
            let spec = ConfirmSpec {
                id: "pb_confirm_switch",
                title: "Switch project?",
                intro: "Opening this project stops what is running now.",
                items: &sw.running,
                detail: Some(format!("Open: {}", elide_middle(&path.display().to_string(), 58))),
                primary: "Stop and open",
                secondary: "Cancel",
                danger: None,
            };
            match confirm_modal(ctx, &p, &spec) {
                Some(Choice::Primary) => {
                    self.pending_switch = None;
                    if let Err(err) = self.load_project_from_path(path) {
                        self.status = format!("Open failed: {err}");
                    }
                }
                Some(Choice::Secondary) => {
                    self.pending_switch = None;
                    self.status = "Cancelled".into();
                }
                Some(Choice::Danger) | None => {}
            }
        }
        if let Some(running) = self.pending_quit.as_ref() {
            let spec = ConfirmSpec {
                id: "pb_confirm_quit",
                title: "Quit Protocol Bench?",
                intro: "The service keeps these running. Stop all disconnects clients too.",
                items: running,
                detail: None,
                primary: "Quit, keep running",
                secondary: "Stay",
                danger: Some("Stop all and quit"),
            };
            match confirm_modal(ctx, &p, &spec) {
                Some(Choice::Primary) => {
                    self.pending_quit = None;
                    self.quit_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(Choice::Danger) => {
                    // Flush edits first, then stop embedded hubs and disconnect every client.
                    let _ = self.save_project_now();
                    if let Err(err) = self.engine.stop_all_hubs() {
                        self.status = format!("Stop all failed: {err}");
                        self.pending_quit = None;
                    } else {
                        self.pending_quit = None;
                        self.stopped_all = true;
                        self.quit_confirmed = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Some(Choice::Secondary) => self.pending_quit = None,
                None => {}
            }
        }
    }

    fn load_project_from_path(&mut self, path: PathBuf) -> Result<(), String> {
        if !path.is_file() {
            let _ = persist::remove_recent_project(&mut self.ui_state, &path);
            self.mark_dirty();
            return Err(format!("File not found: {}", path.display()));
        }
        let auto_run = self.ui_state.auto_run_embedded_hubs;
        self.engine
            .open_project(&path.display().to_string(), auto_run)?;
        let value = self.engine.get_project_json()?;
        let file: ProjectFile = serde_json::from_value(value).map_err(|e| e.to_string())?;
        file.mqtt.apply_to_panel(&mut self.mqtt);
        file.opcua.apply_to_panel(&mut self.opcua);
        file.http.apply_to_panel(&mut self.http);
        file.streams.apply_to_panel(&mut self.streams);
        file.las.apply_to_panel(&mut self.las);
        file.snmp.apply_to_panel(&mut self.snmp);
        file.modbus.apply_to_panel(&mut self.modbus);
        file.files.apply_to_panel(&mut self.files);
        self.project_settings = file.settings.clone();
        if file.settings.auto_run_embedded_broker && !self.ui_state.auto_run_embedded_hubs {
            self.ui_state.auto_run_embedded_hubs = true;
            self.mark_dirty();
        }
        let seed_layout = project::apply_or_seed_layout(&file, &mut self.ui_state);
        if let Some(hubs) = file.hubs {
            self.ui_state.hubs = hubs;
            self.ui_state.hubs.sanitize();
        }
        if let Ok(mut store) = self.engine.points().lock() {
            project::import_bindings_to_store(&mut store, &file.points, file.next_point_seq);
            project::migrate_panel_point_refs(&mut self.mqtt, &mut store);
        }
        project::migrate_client_binds_to_hubs(&mut self.mqtt, &mut self.ui_state.hubs);
        Self::push_hub_binds_to_service(&self.engine, &mut self.ui_state.hubs);
        self.mqtt_saved = MqttProject::from_panel(&self.mqtt);
        self.http_saved = HttpProject::from_panel(&self.http);
        self.streams_saved = StreamsProject::from_panel(&self.streams);
        self.las_saved = LasProject::from_panel(&self.las);
        self.snmp_saved = SnmpProject::from_panel(&self.snmp);
        self.modbus_saved = ModbusProject::from_panel(&self.modbus);
        self.files_saved = FilesProject::from_panel(&self.files);
        self.hubs_saved = self.ui_state.hubs.clone();
        self.settings_saved = self.project_settings.clone();
        self.layout_saved = DockLayout::from_ui(&self.ui_state);
        self.project_path = path;
        opcua_panel::apply_opcua_session(&self.engine, &mut self.opcua, &self.ui_state.hubs);
        opcua_panel::maybe_auto_start_embedded(
            &self.engine,
            &mut self.opcua,
            self.ui_state.auto_run_embedded_hubs,
        );
        if mqtt_panel::apply_mqtt_session(&self.engine, &mut self.mqtt, &mut self.ui_state.hubs) {
            self.mark_dirty();
            Self::push_hub_binds_to_service(&self.engine, &mut self.ui_state.hubs);
        }
        mqtt_panel::maybe_auto_start_embedded(
            &self.engine,
            &mut self.mqtt,
            &mut self.ui_state.hubs,
            self.ui_state.auto_run_embedded_hubs,
        );
        if persist::note_recent_project(&mut self.ui_state, &self.project_path) {
            self.mark_dirty();
        }
        self.status = format!("Opened {}", self.project_path.display());
        if seed_layout {
            let _ = self.save_project_now();
        }
        Ok(())
    }

    /// Append `.json` when the save dialog left the extension off.
    /// `ipc.json` / `ipc.token` are how the UI finds the service — saving a project over one
    /// (the file dialog opens in that folder) leaves the UI permanently "disconnected".
    fn is_service_file(path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        matches!(name, "ipc.json" | "ipc.token")
            && path.parent().map(|d| d.to_path_buf()) == pb_ipc::config_dir()
    }

    fn ensure_project_ext(path: PathBuf) -> PathBuf {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if !ext.is_empty() => path,
            _ => path.with_extension("json"),
        }
    }

    /// Spawn a portal/GTK file dialog on a worker thread (never block the egui loop).
    fn begin_file_pick(&mut self, kind: FilePickKind) {
        if self.file_pick.is_some() {
            self.status = "A file dialog is already open".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        let start_dir = persist::dialog_start_dir(
            &self.ui_state,
            &self.project_path,
            pb_ipc::config_dir().as_deref(),
        )
        .or_else(|| directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()));
        let default_name = self
            .project_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project.json")
            .to_string();
        std::thread::spawn(move || {
            let mut dlg = rfd::FileDialog::new()
                .add_filter("Protocol Bench project", &["json"])
                .add_filter("All files", &["*"]);
            if let Some(dir) = start_dir {
                dlg = dlg.set_directory(dir);
            }
            let picked = match kind {
                FilePickKind::Open => dlg.set_title("Open project").pick_file(),
                FilePickKind::SaveAs => dlg
                    .set_title("Save project as")
                    .set_file_name(&default_name)
                    .save_file(),
            };
            let _ = tx.send(picked);
        });
        self.file_pick = Some(PendingFilePick { kind, rx });
        self.status = match kind {
            FilePickKind::Open => "Choose a project to open…".into(),
            FilePickKind::SaveAs => "Choose where to save the project…".into(),
        };
    }

    fn poll_file_pick(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.file_pick.as_ref() else {
            return;
        };
        // Keep frames alive while the portal dialog is up.
        ctx.request_repaint_after(Duration::from_millis(100));
        let kind = pending.kind;
        match pending.rx.try_recv() {
            Ok(Some(path)) => {
                self.file_pick = None;
                if Self::is_service_file(&path) {
                    self.status = "Not a project: ipc.json / ipc.token belong to the service".into();
                    return;
                }
                match kind {
                    FilePickKind::Open => self.request_project_open(path),
                    FilePickKind::SaveAs => {
                        self.project_path = Self::ensure_project_ext(path);
                        match self.save_project_now() {
                            Ok(()) => {
                                self.status =
                                    format!("Saved {}", self.project_path.display());
                            }
                            Err(err) => self.status = format!("Save failed: {err}"),
                        }
                    }
                }
            }
            Ok(None) => {
                self.file_pick = None;
                self.status = "Cancelled".into();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.file_pick = None;
                self.status = "File dialog failed".into();
            }
        }
    }

    fn reset_layout(&mut self) {
        self.ui_state.tree = LayoutNode::default_tree();
        self.ui_state.maximized = None;
        self.ui_state.hidden_panes.clear();
        self.mark_dirty();
        self.status = "Layout reset".into();
    }

    fn show_all(&mut self) {
        self.ui_state.hidden_panes.clear();
        self.ui_state.maximized = None;
        for pane in DockPane::ALL {
            self.ui_state
                .tree
                .ensure_pane(pane, DockPane::Project, crate::layout::TileEdge::East);
        }
        self.mark_dirty();
        self.status = "All panels shown".into();
    }

    fn apply_menu(&mut self, ctx: &egui::Context, action: MenuAction) {
        self.menu = None;
        match action {
            MenuAction::Close => {}
            MenuAction::OpenProject => self.begin_file_pick(FilePickKind::Open),
            MenuAction::SaveProject => match self.save_project_now() {
                Ok(()) => {
                    self.status = format!("Saved {}", self.project_path.display());
                }
                Err(err) => self.status = format!("Save failed: {err}"),
            },
            MenuAction::SaveAsProject => self.begin_file_pick(FilePickKind::SaveAs),
            MenuAction::OpenRecent(path) => self.request_project_open(path),
            MenuAction::ClearRecent => {
                if persist::clear_recent_projects(&mut self.ui_state) {
                    self.mark_dirty();
                }
                self.status = "Cleared recent projects".into();
            }
            MenuAction::ResetLayout => self.reset_layout(),
            MenuAction::Quit => self.request_quit(ctx),
            MenuAction::About => self.about_open = true,
            MenuAction::Settings => self.settings_open = true,
            MenuAction::Toggle(pane) => {
                if !self.ui_state.toggle_pane_visible(pane) {
                    self.status = "At least one panel stays open".into();
                } else {
                    self.mark_dirty();
                    let verb = if self.ui_state.pane_visible(pane) {
                        "Showing"
                    } else {
                        "Hidden"
                    };
                    self.status = format!("{verb} {}", pane.title());
                }
            }
            MenuAction::ShowAll => self.show_all(),
            MenuAction::Theme(next) => {
                if next != self.theme {
                    self.theme = next;
                    self.ui_state.theme_id = next.id().into();
                    self.mark_dirty();
                }
            }
        }
    }
}

impl eframe::App for ProtocolBenchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Screenshot hook: `PB_SHOT_OUT=<file.png>` saves this window's own pixels after a
        // few seconds (no screen capture, so other windows never appear) and exits.
        if let Some(out) = std::env::var_os("PB_SHOT_OUT") {
            static FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            ctx.request_repaint();
            if n == 240 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                if let Some(buf) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, rgba) {
                    let _ = buf.save(&out);
                }
                std::process::exit(0);
            }
        }
        // Screenshot hook: `PB_SHOT_SELECT=<hub id>[,<node id>]` selects a node once the
        // project has loaded (like `PB_SHOT_CALC_TYPES` for the calculation guide).
        static SHOT_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if let Some(spec) = std::env::var_os("PB_SHOT_SELECT").filter(|_| !SHOT_DONE.load(std::sync::atomic::Ordering::Relaxed)) {
            if self.service_connected && !self.ui_state.hubs.hubs.is_empty() {
                let mut ids = spec.to_string_lossy().split(',').filter_map(|s| s.trim().parse::<u64>().ok()).collect::<Vec<_>>().into_iter();
                if let Some(hub) = ids.next() {
                    self.ui_state.hubs.selected = Some(hub);
                    self.ui_state.hubs.selected_port = ids.next();
                    if let Some(w) = std::env::var("PB_SHOT_INSPECT").ok().and_then(|v| v.parse::<f32>().ok()) {
                        self.ui_state.hubs.inspector_width = w;
                    }
                    if std::env::var_os("PB_SHOT_MAX").is_some() {
                        self.ui_state.maximized = Some(DockPane::Hubs);
                    }
                    SHOT_DONE.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.menu.take().is_none() {
                if self.about_open {
                    self.about_open = false;
                } else {
                    self.settings_open = false;
                }
            }
        }

        if self.theme_applied != Some(self.theme) {
            theme::apply_theme(ctx, self.theme);
            self.theme_applied = Some(self.theme);
        }
        // PB-64: poll engine on a data timer — mouse frames reuse live_host.
        const LIVE_POLL: Duration = Duration::from_millis(150);
        if self.last_live_poll.elapsed() >= LIVE_POLL {
            self.last_live_poll = Instant::now();
            self.engine
                .set_calcs(crate::calc_panel::configs_from_hubs(&self.ui_state.hubs));
            self.engine
                .set_aggs(crate::agg_panel::configs_from_hubs(&self.ui_state.hubs));
            let snap = self.engine.snapshot();
            let fp = Self::points_fingerprint(&snap);
            if fp != self.live_points_fp {
                self.live_points_fp = fp;
                self.live_host = snap;
            } else {
                // Keep point values frozen; still track pack liveness lightly.
                self.live_host.alive = snap.alive;
                self.live_host.ticks = snap.ticks;
                self.live_host.uptime_ms = snap.uptime_ms;
            }
        }
        self.poll_service_link();
        let p = self.theme.palette();
        theme::paint_root_bg(ctx, &p);
        self.pane_hits.clear();
        let host_for_panes = self.live_host.clone();
        // Keep the UA address space in step with hub catalog edits even when the
        // OPC UA panel is not on screen (the pack ignores unchanged rows).
        if self.opcua.server_running {
            opcua_panel::sync_points_to_server(&self.engine, &host_for_panes, &self.ui_state.hubs);
        }

        self.app_bar(ctx, &p);
        self.status_bar(ctx, &p);
        self.pane_rail(ctx, &p);
        self.settings_drawer(ctx, &p, &host_for_panes);
        self.about_window(ctx, &p);
        self.poll_file_pick(ctx);
        // Window X / Alt+F4 / compositor close: hold it until the user has been told what keeps running.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_quit(ctx);
        }
        self.confirm_dialogs(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::same(SEP)),
            )
            .show(ctx, |ui| {
                let avail = ui.available_size();
                let shown = self
                    .ui_state
                    .tree
                    .without_hidden(&self.ui_state.hidden_panes);
                if let Some(max) = self
                    .ui_state
                    .maximized
                    .filter(|pane| self.ui_state.pane_visible(*pane))
                {
                    if let Some(area) = shown.as_ref().and_then(|tree| tree.area_containing(max)) {
                        self.render_node(ui, area, avail, &[], &host_for_panes, &p);
                    }
                } else if let Some(tree) = shown.as_ref() {
                    self.render_node(ui, tree, avail, &[], &host_for_panes, &p);
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("Show a panel from the left rail").color(p.muted),
                        );
                    });
                }
            });

        self.handle_drop(ctx, &p);
        self.autosave();
        self.autosave_project(&host_for_panes);
        // Next data poll only — do not chase mouse FPS for live values (PB-64).
        ctx.request_repaint_after(LIVE_POLL);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.ui_state.theme_id = self.theme.id().into();
        let _ = persist::save(&self.ui_state);
        let _ = self.save_project_now();
        self.engine.shutdown();
    }
}

impl ProtocolBenchApp {
    fn app_bar(&mut self, ctx: &egui::Context, p: &ThemePalette) {
        let mut file = None;
        let mut view = None;
        let mut help = None;
        let mut theme_anchor = None;
        egui::TopBottomPanel::top("app_bar")
            .exact_height(BAR_H)
            .frame(
                egui::Frame::none()
                    .fill(p.bar)
                    .inner_margin(egui::Margin::symmetric(14.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (mark_rect, mark_resp) =
                        ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                    art::paint_mark(ui.painter(), mark_rect, p);
                    if mark_resp.clicked() {
                        self.about_open = true;
                        self.menu = None;
                    }
                    mark_resp
                        .on_hover_text("About Protocol Bench")
                        .on_hover_cursor(CursorIcon::PointingHand);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(
                            egui::RichText::new("Protocol")
                                .font(font_semi(20.0))
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new("Bench")
                                .font(font_semi(20.0))
                                .color(p.accent),
                        );
                    });
                    ui.add_space(10.0);
                    file = Some(self.menu_button(ui, "File", BarMenu::File, p));
                    view = Some(self.menu_button(ui, "View", BarMenu::View, p));
                    help = Some(self.menu_button(ui, "Help", BarMenu::Help, p));

                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if art::chrome_button(
                            ui,
                            ChromeIcon::Gear,
                            "Settings",
                            self.settings_open,
                            p,
                            34.0,
                        )
                        .clicked()
                        {
                            self.settings_open = !self.settings_open;
                            self.menu = None;
                        }
                        let theme_resp =
                            theme_chip(ui, self.theme, self.menu == Some(BarMenu::Theme), p)
                                .on_hover_text(format!("Theme — {}", self.theme.label()));
                        if theme_resp.clicked() {
                            self.toggle_menu(BarMenu::Theme);
                        }
                        theme_anchor = Some(Anchor {
                            rect: theme_resp.rect,
                            clicked: theme_resp.clicked(),
                        });
                    });
                });
                let clip = ui.clip_rect();
                ui.painter().line_segment(
                    [
                        Pos2::new(clip.min.x, clip.max.y - 0.5),
                        Pos2::new(clip.max.x, clip.max.y - 0.5),
                    ],
                    Stroke::new(1.5, p.accent),
                );
            });

        let mut action = None;
        if let Some(anchor) = file {
            self.file_menu(ctx, &anchor, p, &mut action);
        }
        if let Some(anchor) = view {
            self.view_menu(ctx, &anchor, p, &mut action);
        }
        if let Some(anchor) = help {
            self.help_menu(ctx, &anchor, p, &mut action);
        }
        if let Some(anchor) = theme_anchor {
            self.theme_menu(ctx, &anchor, p, &mut action);
        }
        if let Some(action) = action {
            self.apply_menu(ctx, action);
        }
    }

    fn menu_button(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        id: BarMenu,
        p: &ThemePalette,
    ) -> Anchor {
        let open = self.menu == Some(id);
        let resp = bar_text_button(ui, label, open, p);
        if resp.clicked() {
            self.toggle_menu(id);
        }
        Anchor {
            rect: resp.rect,
            clicked: resp.clicked(),
        }
    }

    fn toggle_menu(&mut self, id: BarMenu) {
        self.menu = if self.menu == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    fn file_menu(
        &self,
        ctx: &egui::Context,
        anchor: &Anchor,
        p: &ThemePalette,
        action: &mut Option<MenuAction>,
    ) {
        let recents = persist::existing_recent_projects(&self.ui_state);
        let has_recent_entries = !self.ui_state.recent_projects.is_empty();
        let current = self.project_path.clone();
        self.popup(ctx, anchor, BarMenu::File, p, action, |ui, action| {
            if menu_row(ui, "Open project…", None, p).clicked() {
                *action = Some(MenuAction::OpenProject);
            }
            let recent = menu_row_padded(ui, "Open recent", None, 18.0, p);
            submenu_arrow(ui.painter(), recent.rect, p);
            recent_submenu(
                ui.ctx(),
                recent.rect,
                recent.hovered(),
                &recents,
                has_recent_entries,
                &current,
                p,
                action,
            );
            menu_sep(ui, p);
            if menu_row(ui, "Save project", None, p).clicked() {
                *action = Some(MenuAction::SaveProject);
            }
            if menu_row(ui, "Save project as…", None, p).clicked() {
                *action = Some(MenuAction::SaveAsProject);
            }
            menu_sep(ui, p);
            if menu_row(ui, "Quit", None, p).clicked() {
                *action = Some(MenuAction::Quit);
            }
        });
        // Choosing anything closes the menu, so the submenu must not reopen on the next visit.
        if action.is_some() {
            ctx.data_mut(|d| d.insert_temp(recent_open_key(), false));
        }
    }

    fn view_menu(
        &self,
        ctx: &egui::Context,
        anchor: &Anchor,
        p: &ThemePalette,
        action: &mut Option<MenuAction>,
    ) {
        let visible: Vec<(DockPane, bool)> = DockPane::ALL
            .iter()
            .copied()
            .map(|pane| (pane, self.ui_state.pane_visible(pane)))
            .collect();
        self.popup(ctx, anchor, BarMenu::View, p, action, |ui, action| {
            for (pane, on) in visible {
                if menu_row(ui, pane.title(), Some(on), p).clicked() {
                    *action = Some(MenuAction::Toggle(pane));
                }
            }
            menu_sep(ui, p);
            if menu_row(ui, "Show all panels", None, p).clicked() {
                *action = Some(MenuAction::ShowAll);
            }
            if menu_row(ui, "Reset layout", None, p).clicked() {
                *action = Some(MenuAction::ResetLayout);
            }
        });
    }

    fn help_menu(
        &self,
        ctx: &egui::Context,
        anchor: &Anchor,
        p: &ThemePalette,
        action: &mut Option<MenuAction>,
    ) {
        self.popup(ctx, anchor, BarMenu::Help, p, action, |ui, action| {
            if menu_row(ui, "About Protocol Bench", None, p).clicked() {
                *action = Some(MenuAction::About);
            }
            if menu_row(ui, "Settings", None, p).clicked() {
                *action = Some(MenuAction::Settings);
            }
        });
    }

    fn theme_menu(
        &self,
        ctx: &egui::Context,
        anchor: &Anchor,
        p: &ThemePalette,
        action: &mut Option<MenuAction>,
    ) {
        let current = self.theme;
        self.popup(ctx, anchor, BarMenu::Theme, p, action, |ui, action| {
            ui.label(
                egui::RichText::new("THEME")
                    .font(font_ui(10.0))
                    .color(p.muted),
            );
            ui.add_space(4.0);
            for theme in AppTheme::ALL {
                if theme_row(ui, theme, theme == current, p).clicked() {
                    *action = Some(MenuAction::Theme(theme));
                }
            }
        });
    }

    fn popup(
        &self,
        ctx: &egui::Context,
        anchor: &Anchor,
        id: BarMenu,
        p: &ThemePalette,
        action: &mut Option<MenuAction>,
        add: impl FnOnce(&mut egui::Ui, &mut Option<MenuAction>),
    ) {
        if self.menu != Some(id) {
            return;
        }
        let mut x = anchor.rect.min.x;
        if id == BarMenu::Theme {
            x = (anchor.rect.max.x - 214.0).max(8.0);
        }
        let pos = Pos2::new(x, anchor.rect.max.y + 6.0);
        let mut close = false;
        egui::Area::new(Id::new(match id {
            BarMenu::File => "pb_menu_file",
            BarMenu::View => "pb_menu_view",
            BarMenu::Help => "pb_menu_help",
            BarMenu::Theme => "pb_menu_theme",
        }))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .interactable(true)
        .show(ctx, |ui| {
            // Separators fill the available width, which in an Area is the whole screen;
            // cap it to the menu's own width measured last frame (96 on the first frame).
            let wkey = Id::new(("pb_menu_w", id as u8));
            let prev: f32 = ctx.data(|d| d.get_temp(wkey)).unwrap_or(96.0);
            let inner = menu_frame(p)
                .show(ui, |ui| {
                    ui.set_max_width(prev);
                    add(ui, action);
                    let w = ui.min_rect().width();
                    ctx.data_mut(|d| d.insert_temp(wkey, w));
                    if (w - prev).abs() > 0.5 {
                        ctx.request_repaint();
                    }
                })
                .response;
            if inner.clicked_elsewhere() && !anchor.clicked {
                close = true;
            }
        });
        if close && action.is_none() {
            *action = Some(MenuAction::Close);
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context, p: &ThemePalette) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(STATUS_H)
            .frame(
                egui::Frame::none()
                    .fill(p.bar)
                    .inner_margin(egui::Margin::symmetric(14.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let cluster = 220.0;
                    let left = (ui.available_width() - cluster).max(40.0);
                    ui.add_sized(
                        Vec2::new(left, 18.0),
                        egui::Label::new(
                            egui::RichText::new(&self.status)
                                .font(font_ui(12.0))
                                .color(p.muted),
                        )
                        .truncate(),
                    );
                    // Click forces an immediate probe; auto-probe also runs ~1.5s.
                    if service_connection_chip(ui, self.service_connected, p).clicked() {
                        self.last_service_poll = Instant::now()
                            .checked_sub(Duration::from_secs(10))
                            .unwrap_or_else(Instant::now);
                        self.poll_service_link();
                    }
                });
                let clip = ui.clip_rect();
                ui.painter().line_segment(
                    [
                        Pos2::new(clip.min.x, clip.min.y + 0.5),
                        Pos2::new(clip.max.x, clip.min.y + 0.5),
                    ],
                    Stroke::new(1.0, p.line),
                );
            });
    }

    fn pane_rail(&mut self, ctx: &egui::Context, p: &ThemePalette) {
        egui::SidePanel::left("pane_rail")
            .exact_width(RAIL_W)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(p.bar)
                    .stroke(Stroke::new(1.0, p.line))
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 10.0,
                        bottom: 10.0,
                    }),
            )
            .show(ctx, |ui| {
                ui.with_layout(Layout::top_down(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    rail_caption(ui, "BENCH", p);
                    for pane in DockPane::ALL
                        .into_iter()
                        .filter(|pane| pane.engine().is_none())
                    {
                        self.rail_toggle(ui, pane, p);
                    }
                    ui.add_space(6.0);
                    rail_caption(ui, "LINKS", p);
                    for pane in DockPane::ALL
                        .into_iter()
                        .filter(|pane| pane.engine().is_some())
                    {
                        self.rail_toggle(ui, pane, p);
                    }
                });
            });
    }

    fn rail_toggle(&mut self, ui: &mut egui::Ui, pane: DockPane, p: &ThemePalette) {
        let on = self.ui_state.pane_visible(pane);
        if rail_button(ui, pane, on, p).clicked() {
            if !self.ui_state.toggle_pane_visible(pane) {
                self.status = "At least one panel stays open".into();
            } else {
                self.mark_dirty();
            }
        }
    }

    fn settings_drawer(
        &mut self,
        ctx: &egui::Context,
        p: &ThemePalette,
        host: &pb_engine::HostSnapshot,
    ) {
        if !self.settings_open {
            return;
        }
        egui::SidePanel::right("settings_drawer")
            .exact_width(320.0)
            .frame(
                egui::Frame::none()
                    .fill(p.bar)
                    .stroke(Stroke::new(1.0, p.line))
                    .inner_margin(egui::Margin::same(16.0)),
            )
            .show(ctx, |ui| {
                apply_form_spacing(ui);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Settings")
                            .font(font_semi(16.0))
                            .color(p.text),
                    );
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if art::chrome_button(
                            ui,
                            ChromeIcon::Close,
                            "Close settings",
                            false,
                            p,
                            26.0,
                        )
                        .clicked()
                        {
                            self.settings_open = false;
                        }
                    });
                });
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Theme is the palette chip on the menu bar.")
                        .font(font_ui(12.0))
                        .color(p.muted),
                );
                // PB-58 control — dropped accidentally in PB-96 settings rewrite.
                ui.add_space(space::SECTION - 2.0);
                ui.label(
                    egui::RichText::new("HUBS")
                        .font(font_ui(10.5))
                        .color(p.muted),
                );
                ui.add_space(space::AFTER_HEADING);
                let prev_auto = self.ui_state.auto_run_embedded_hubs;
                if ui
                    .checkbox(
                        &mut self.ui_state.auto_run_embedded_hubs,
                        "Auto-run embedded hubs",
                    )
                    .changed()
                {
                    self.mark_dirty();
                    // Turning on at runtime starts MQTT + OPC UA hubs when configured.
                    if self.ui_state.auto_run_embedded_hubs && !prev_auto {
                        mqtt_panel::maybe_auto_start_embedded(
                            &self.engine,
                            &mut self.mqtt,
                            &mut self.ui_state.hubs,
                            true,
                        );
                        opcua_panel::maybe_auto_start_embedded(
                            &self.engine,
                            &mut self.opcua,
                            true,
                        );
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "When enabled, start the MQTT broker and OPC UA server hubs on load \
(and when this is turned on) if they are configured in their panels.",
                    )
                    .font(font_ui(11.5))
                    .color(p.muted),
                );
                ui.add_space(16.0);
                if text_button(ui, "Reset layout", p).clicked() {
                    self.reset_layout();
                }
                ui.add_space(8.0);
                if text_button(ui, "Show all panels", p).clicked() {
                    self.show_all();
                }
                ui.add_space(8.0);
                if text_button(ui, "About", p).clicked() {
                    self.about_open = true;
                }
                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("ENGINE HOST")
                        .font(font_ui(10.5))
                        .color(p.muted),
                );
                ui.add_space(4.0);
                let alive = if host.alive { "up" } else { "stopped" };
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {} · {}",
                        host.thread_name,
                        alive,
                        panels::format_uptime(host.uptime_ms)
                    ))
                    .font(theme::font_mono(12.0))
                    .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Five slots. No sockets, brokers, or files.")
                        .font(font_ui(12.0))
                        .color(p.muted),
                );
            });
    }

    fn about_window(&mut self, ctx: &egui::Context, p: &ThemePalette) {
        if !self.about_open {
            return;
        }
        let screen = ctx.screen_rect();
        let scrim = egui::Area::new(Id::new("pb_about_scrim"))
            .order(egui::Order::Middle)
            .fixed_pos(screen.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, Rounding::ZERO, Color32::from_black_alpha(78));
                resp
            });
        if scrim.inner.clicked() {
            self.about_open = false;
            return;
        }

        let mut open = true;
        let frame = egui::Frame::none()
            .fill(p.pane)
            .stroke(Stroke::new(1.0, p.line))
            .rounding(Rounding::same(10.0))
            .inner_margin(egui::Margin::symmetric(24.0, 18.0));
        egui::Window::new("About Protocol Bench")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_min_width(420.0);
                // Close X overlays the corner so the wordmark stays centered.
                {
                    let area = ui.available_rect_before_wrap();
                    let btn = 26.0;
                    let btn_rect = Rect::from_min_size(
                        Pos2::new(area.right() - btn, area.top()),
                        Vec2::splat(btn),
                    );
                    let mut btn_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(btn_rect)
                            .layout(Layout::left_to_right(egui::Align::Center)),
                    );
                    if art::chrome_button(&mut btn_ui, ChromeIcon::Close, "Close", false, p, btn)
                        .clicked()
                    {
                        open = false;
                    }
                }
                ui.vertical_centered(|ui| {
                    let (band, _) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 84.0), Sense::hover());
                    let mark = Rect::from_center_size(band.center(), Vec2::splat(72.0));
                    art::paint_mark(ui.painter(), mark, p);
                    ui.add_space(8.0);
                    paint_about_wordmark(ui, p);
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(PRODUCT_SLOGAN)
                            .font(font_ui(13.0))
                            .color(p.text),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .font(font_ui(13.0))
                            .color(p.muted),
                    );
                });
                ui.add_space(14.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    for (label, pane) in [
                        ("Streams", DockPane::Streams),
                        ("Modbus", DockPane::Modbus),
                        ("MQTT", DockPane::Mqtt),
                        ("OPC UA", DockPane::OpcUa),
                        ("HTTP / REST", DockPane::Http),
                        ("LAS", DockPane::Las),
                        ("SNMP", DockPane::Snmp),
                        ("File follow", DockPane::Files),
                        ("Points", DockPane::Points),
                    ] {
                        about_chip(ui, label, pane.ink_core(), p);
                    }
                });
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(
                        "Protocol I/O will run in the always-on service. This build does not open a link.",
                    )
                    .font(font_ui(12.0))
                    .color(p.muted),
                );
                ui.add_space(18.0);
                // Company logo at the bottom of About.
                ui.vertical_centered(|ui| {
                    self.aa_logo
                        .show(ui, ui.available_width().min(280.0), 64.0);
                    ui.add_space(14.0);
                    let site = ui
                        .link(
                            egui::RichText::new("www.agileautomation.ca")
                                .font(font_ui(13.0))
                                .color(Color32::from_rgb(74, 144, 226))
                                .underline(),
                        )
                        .on_hover_text(COMPANY_URL)
                        .on_hover_cursor(CursorIcon::PointingHand);
                    if site.clicked() {
                        open_in_browser(COMPANY_URL);
                    }
                });
            });
        self.about_open = open;
    }

    fn render_node(
        &mut self,
        ui: &mut egui::Ui,
        node: &LayoutNode,
        size: Vec2,
        path: &[bool],
        host: &pb_engine::HostSnapshot,
        p: &ThemePalette,
    ) {
        let size = Vec2::new(size.x.max(1.0), size.y.max(1.0));
        match node {
            LayoutNode::Leaf(group) => self.paint_area(ui, group, size, host, p),
            LayoutNode::Split {
                dir,
                ratio,
                first,
                second,
            } => {
                let ratio = ratio.clamp(0.08, 0.92);
                let mut path_first = path.to_vec();
                path_first.push(false);
                let mut path_second = path.to_vec();
                path_second.push(true);
                let blocked = match dir {
                    SplitDir::Row => size.x < MIN_PANE * 2.0 + SEP,
                    SplitDir::Col => size.y < MIN_PANE * 2.0 + SEP,
                };
                match dir {
                    SplitDir::Col => {
                        let (h1, h2, usable) = split_axis(size.y, SEP, ratio);
                        let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
                        let mut col = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(outer)
                                .layout(Layout::top_down(egui::Align::Min)),
                        );
                        col.set_clip_rect(outer);
                        col.spacing_mut().item_spacing = Vec2::ZERO;
                        self.render_node(
                            &mut col,
                            first,
                            Vec2::new(size.x, h1),
                            &path_first,
                            host,
                            p,
                        );
                        if let Some(next) =
                            tile_splitter(&mut col, false, size.x, SEP, usable, ratio, blocked, p)
                        {
                            self.ui_state.tree.set_ratio_at(path, next);
                            self.mark_dirty();
                        }
                        self.render_node(
                            &mut col,
                            second,
                            Vec2::new(size.x, h2),
                            &path_second,
                            host,
                            p,
                        );
                    }
                    SplitDir::Row => {
                        let (w1, w2, usable) = split_axis(size.x, SEP, ratio);
                        let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
                        let mut row = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(outer)
                                .layout(Layout::left_to_right(egui::Align::Min)),
                        );
                        row.set_clip_rect(outer);
                        row.spacing_mut().item_spacing = Vec2::ZERO;
                        self.render_node(
                            &mut row,
                            first,
                            Vec2::new(w1, size.y),
                            &path_first,
                            host,
                            p,
                        );
                        if let Some(next) =
                            tile_splitter(&mut row, true, size.y, SEP, usable, ratio, blocked, p)
                        {
                            self.ui_state.tree.set_ratio_at(path, next);
                            self.mark_dirty();
                        }
                        self.render_node(
                            &mut row,
                            second,
                            Vec2::new(w2, size.y),
                            &path_second,
                            host,
                            p,
                        );
                    }
                }
            }
        }
    }

    fn paint_area(
        &mut self,
        ui: &mut egui::Ui,
        group: &TabGroup,
        size: Vec2,
        host: &pb_engine::HostSnapshot,
        p: &ThemePalette,
    ) {
        let active = group.active_pane();
        let maximized = self
            .ui_state
            .maximized
            .is_some_and(|pane| group.contains(pane));
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(egui::Align::Min)),
        );
        let clip = rect.intersect(ui.clip_rect());
        child.set_clip_rect(clip);
        let painter = child.painter();
        let ink = active.ink(p.is_light);
        painter.rect_filled(rect, Rounding::same(RADIUS), p.pane);
        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_H));
        painter.rect_filled(
            header,
            Rounding {
                nw: RADIUS,
                ne: RADIUS,
                sw: 0.0,
                se: 0.0,
            },
            p.header_top,
        );
        painter.line_segment(
            [
                Pos2::new(header.min.x + 8.0, header.max.y),
                Pos2::new(header.max.x - 8.0, header.max.y),
            ],
            Stroke::new(1.0, p.line),
        );
        painter.rect_stroke(rect, Rounding::same(RADIUS), Stroke::new(1.0, p.line));

        if group.tabs.len() < 2 {
            let cap = Rect::from_center_size(
                Pos2::new(header.min.x + 11.0, header.center().y),
                Vec2::new(3.0, 16.0),
            );
            painter.rect_filled(cap, Rounding::same(1.5), ink);
            let max_w = 44.0;
            let drag_w = (header.width() - max_w).max(48.0);
            let drag_rect = Rect::from_min_size(header.min, Vec2::new(drag_w, header.height()));
            {
                let title_painter = painter.with_clip_rect(drag_rect.shrink2(Vec2::new(4.0, 0.0)));
                paint_grip(
                    &title_painter,
                    Pos2::new(header.min.x + 20.0, header.center().y - 7.0),
                    p.muted,
                );
                let icon = Rect::from_center_size(
                    Pos2::new(header.min.x + 46.0, header.center().y),
                    Vec2::splat(16.0),
                );
                art::paint_pane_icon(&title_painter, icon, active, ink);
                title_painter.text(
                    Pos2::new(header.min.x + 60.0, header.center().y),
                    Align2::LEFT_CENTER,
                    active.title(),
                    font_semi(15.0),
                    p.text,
                );
            }
            let drag = child
                .interact(
                    drag_rect,
                    Id::new(("pb_pane_drag", active.id())),
                    Sense::drag(),
                )
                .on_hover_text("Drag the title — top adds a tab, edges dock, center swaps")
                .on_hover_cursor(CursorIcon::Grab);
            if drag.dragged() {
                if dnd::arm_drag(child.ctx(), active, drag.drag_delta(), rect.size()) {
                    egui::DragAndDrop::set_payload(child.ctx(), active);
                }
            } else if !egui::DragAndDrop::has_payload_of_type::<DockPane>(child.ctx()) {
                dnd::clear_drag_arm(child.ctx(), active);
            }
        } else {
            // Several panes share this area. Each chip is its own drag handle.
            if paint_tab_strip(
                &mut child,
                &mut self.ui_state,
                group,
                header,
                rect.size(),
                p,
            ) {
                self.mark_dirty();
            }
        }

        let mut toggle_max = false;
        // Header-right only. A layout on the pane ui would drop the button into the body.
        let btn_zone = Rect::from_min_max(
            Pos2::new(header.max.x - 38.0, header.min.y),
            Pos2::new(header.max.x - 6.0, header.max.y),
        );
        let mut btn_ui = child.new_child(
            egui::UiBuilder::new()
                .max_rect(btn_zone)
                .layout(Layout::right_to_left(egui::Align::Center)),
        );
        btn_ui.set_clip_rect(btn_zone.intersect(clip));
        let icon = if maximized {
            ChromeIcon::Restore
        } else {
            ChromeIcon::Maximize
        };
        let tip = if maximized { "Restore" } else { "Maximize" };
        if art::chrome_button(&mut btn_ui, icon, tip, maximized, p, 26.0).clicked() {
            toggle_max = true;
        }
        if toggle_max {
            self.ui_state.maximized = if maximized { None } else { Some(active) };
            self.mark_dirty();
        }

        let body = Rect::from_min_max(
            Pos2::new(rect.min.x + 12.0, header.max.y + 10.0),
            Pos2::new(rect.max.x - 12.0, rect.max.y - 12.0),
        );
        if body.height() > 8.0 && body.width() > 8.0 {
            let mut body_ui = child.new_child(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(Layout::top_down(egui::Align::Min)),
            );
            body_ui.set_clip_rect(body.expand(2.0).intersect(clip));
            let hub_change = panels::paint(
                &mut body_ui,
                active,
                host,
                p,
                &mut self.ui_state.hubs,
                &self.engine,
                &mut self.mqtt,
                &mut self.opcua,
                &mut self.http,
                &mut self.streams,
                &mut self.las,
                &mut self.snmp,
                &mut self.modbus,
                &mut self.files,
                &mut self.points,
                &mut self.traffic,
                &mut self.project_settings,
                &self.project_path,
            );
            if hub_change.dirty {
                self.mark_dirty();
                // Allocate / refresh point ids for new Input binds, then push to service.
                Self::push_hub_binds_to_service(&self.engine, &mut self.ui_state.hubs);
                // Hub Input/Output MQTT binds feed the pack session.
                let _ = mqtt_panel::apply_mqtt_session(
                    &self.engine,
                    &mut self.mqtt,
                    &mut self.ui_state.hubs,
                );
                opcua_panel::apply_opcua_session(
                    &self.engine,
                    &mut self.opcua,
                    &self.ui_state.hubs,
                );
            }
            // Auto-run toggle lives in Settings (ui.toml), not Project panel.
            if let Some(pane) = hub_change.open {
                self.ui_state.reveal_pane(pane);
                self.ui_state.tree.set_active(pane);
                if self.ui_state.maximized.is_some() {
                    self.ui_state.maximized = Some(pane);
                }
                self.status = format!("Opened {}", pane.title());
                self.mark_dirty();
            }
        }
        self.pane_hits
            .push(PaneHit::area(active, group.tabs.clone(), rect));
    }

    fn handle_drop(&mut self, ctx: &egui::Context, p: &ThemePalette) {
        let dragging = egui::DragAndDrop::payload::<DockPane>(ctx).map(|pane| *pane);
        let released = ctx.input(|i| i.pointer.any_released());
        if dragging.is_none() {
            if released {
                if let Some(commit) = self.pending.take() {
                    self.apply_commit(ctx, commit);
                } else if self.status.starts_with("Dragging") {
                    self.status = "Ready".into();
                }
            } else if !egui::DragAndDrop::has_any_payload(ctx) {
                self.pending = None;
            }
            return;
        }

        let pane = dragging.unwrap();
        let Some(pos) = ctx
            .pointer_interact_pos()
            .or_else(|| ctx.pointer_latest_pos())
        else {
            return;
        };
        let src = ctx.data(|data| {
            data.get_temp::<Vec2>(Id::new("pb_drag_src_size"))
                .unwrap_or(Vec2::new(280.0, 180.0))
        });
        dnd::paint_ghost(ctx, pane, pos, src, p);

        let Some(hit) = self
            .pane_hits
            .iter()
            .filter(|hit| hit.rect.contains(pos))
            .min_by(|a, b| {
                a.rect
                    .area()
                    .partial_cmp(&b.rect.area())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
        else {
            self.pending = None;
            self.status = format!("Dragging {} — release to cancel", pane.title());
            return;
        };

        let landing = dnd::landing_at(pane, &hit, pos);
        dnd::paint_landing(ctx, hit.rect, &landing, p);
        if let Some(commit) = landing.commit {
            self.status = format!("Dragging {} — {}", pane.title(), landing.label);
            self.pending = Some(commit);
        } else {
            self.pending = None;
            self.status = format!("Dragging {}", pane.title());
        }
        if released {
            if let Some(commit) = self.pending.take() {
                self.apply_commit(ctx, commit);
            }
        }
    }

    fn apply_commit(&mut self, ctx: &egui::Context, commit: DropCommit) {
        let _ = egui::DragAndDrop::take_payload::<DockPane>(ctx);
        match commit {
            DropCommit::Swap { a, b } => {
                self.ui_state.tree.swap_panes(a, b);
                self.status = format!("Swapped {} with {}", a.title(), b.title());
                self.mark_dirty();
            }
            DropCommit::Tab { moving, target } => {
                if self.ui_state.tree.tab_onto(moving, target) {
                    self.status = format!("Tabbed {} onto {}", moving.title(), target.title());
                    self.mark_dirty();
                } else {
                    self.status = "Tab blocked".into();
                }
            }
            DropCommit::Dock {
                moving,
                target,
                edge,
            } => {
                if self.ui_state.tree.dock_into(moving, target, edge) {
                    self.status = format!(
                        "Docked {} {} {}",
                        moving.title(),
                        edge.phrase(),
                        target.title()
                    );
                    self.mark_dirty();
                } else {
                    self.status = "Dock blocked".into();
                }
            }
        }
    }
}

/// Tab chips along the header. Returns true when the selected tab changes.
fn paint_tab_strip(
    ui: &mut egui::Ui,
    state: &mut UiState,
    group: &TabGroup,
    header: Rect,
    source: Vec2,
    p: &ThemePalette,
) -> bool {
    let count = group.tabs.len();
    let right = header.max.x - 44.0;
    let left = header.min.x + 8.0;
    let gap = 4.0;
    let avail = (right - left - gap * (count.saturating_sub(1) as f32)).max(1.0);
    let width = (avail / count as f32).clamp(1.0, 156.0);
    let active = group.active.min(count - 1);
    let mut changed = false;
    let mut x = left;
    for (index, pane) in group.tabs.iter().copied().enumerate() {
        let on = index == active;
        let bottom = if on {
            header.max.y + 1.0
        } else {
            header.max.y - 5.0
        };
        let tab = Rect::from_min_max(
            Pos2::new(x, header.min.y + 7.0),
            Pos2::new(x + width, bottom),
        );
        x += width + gap;
        let ink = pane.ink(p.is_light);
        let fill = if on { p.pane } else { p.control };
        let rounding = Rounding {
            nw: 6.0,
            ne: 6.0,
            sw: 0.0,
            se: 0.0,
        };
        let painter = ui
            .painter()
            .with_clip_rect(tab.expand2(Vec2::new(0.0, 1.0)));
        painter.rect_filled(tab, rounding, fill);
        painter.rect_filled(
            Rect::from_min_max(tab.min, Pos2::new(tab.max.x, tab.min.y + 2.0)),
            rounding,
            if on { ink } else { tint(ink, 80) },
        );
        if width >= 26.0 {
            let icon = Rect::from_center_size(
                Pos2::new(tab.min.x + 13.0, tab.center().y + 1.0),
                Vec2::splat(13.0),
            );
            art::paint_pane_icon(&painter, icon, pane, if on { ink } else { p.muted });
        }
        if width >= 68.0 {
            let text_clip = Rect::from_min_max(
                Pos2::new(tab.min.x + 24.0, tab.min.y),
                Pos2::new(tab.max.x - 6.0, tab.max.y),
            );
            painter.with_clip_rect(text_clip).text(
                Pos2::new(text_clip.min.x, tab.center().y + 1.0),
                Align2::LEFT_CENTER,
                pane.title(),
                font_semi(13.0),
                if on { p.text } else { p.muted },
            );
        }
        let resp = ui
            .interact(
                tab,
                Id::new(("pb_tab_drag", pane.id())),
                Sense::click_and_drag(),
            )
            .on_hover_text("Drag this tab onto a strip to stack it, or onto an edge to dock it")
            .on_hover_cursor(CursorIcon::Grab);
        if resp.clicked() && !on && state.tree.set_active(pane) {
            changed = true;
        }
        if resp.dragged() {
            if dnd::arm_drag(ui.ctx(), pane, resp.drag_delta(), source) {
                egui::DragAndDrop::set_payload(ui.ctx(), pane);
            }
        } else if !egui::DragAndDrop::has_payload_of_type::<DockPane>(ui.ctx()) {
            dnd::clear_drag_arm(ui.ctx(), pane);
        }
    }
    changed
}

fn paint_grip(painter: &egui::Painter, origin: Pos2, color: Color32) {
    for row in 0..4 {
        for col in 0..2 {
            painter.circle_filled(
                Pos2::new(origin.x + col as f32 * 3.5, origin.y + row as f32 * 3.6),
                1.05,
                color,
            );
        }
    }
}

fn bar_text_button(ui: &mut egui::Ui, label: &str, open: bool, p: &ThemePalette) -> egui::Response {
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(label.to_owned(), font_ui(13.5), p.text));
    let size = galley.size() + Vec2::new(20.0, 10.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if open || resp.hovered() {
        ui.painter().rect_filled(
            rect,
            Rounding::same(6.0),
            tint(p.accent, if open { 40 } else { 18 }),
        );
    }
    let color = if open { p.accent } else { p.text };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        font_ui(13.5),
        color,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn theme_chip(ui: &mut egui::Ui, theme: AppTheme, open: bool, p: &ThemePalette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(72.0, 32.0), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(6.0), p.control);
    painter.rect_stroke(
        rect,
        Rounding::same(6.0),
        Stroke::new(
            if open { 1.5 } else { 1.0 },
            if open || resp.hovered() {
                p.accent
            } else {
                p.line
            },
        ),
    );
    paint_swatches(
        painter,
        Rect::from_center_size(rect.center(), Vec2::new(52.0, 14.0)),
        &theme.palette().swatch_colors(),
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn theme_row(
    ui: &mut egui::Ui,
    theme: AppTheme,
    selected: bool,
    p: &ThemePalette,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(200.0, 30.0), Sense::click());
    let painter = ui.painter();
    if selected || resp.hovered() {
        painter.rect_filled(
            rect,
            Rounding::same(5.0),
            tint(p.accent, if selected { 32 } else { 16 }),
        );
    }
    let sw = Rect::from_min_size(
        Pos2::new(rect.min.x + 8.0, rect.center().y - 6.0),
        Vec2::new(52.0, 12.0),
    );
    paint_swatches(painter, sw, &theme.palette().swatch_colors());
    painter.text(
        Pos2::new(sw.max.x + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        theme.label(),
        font_ui(12.5),
        if selected { p.accent } else { p.text },
    );
    resp
}

fn paint_swatches(painter: &egui::Painter, rect: Rect, colors: &[Color32]) {
    if colors.is_empty() || rect.width() < 2.0 {
        return;
    }
    let units = (colors.len() + 1) as f32;
    let unit = rect.width() / units;
    let mut x = rect.min.x;
    for (i, color) in colors.iter().enumerate() {
        let w = if i == 0 { unit * 2.0 } else { unit };
        let cell = Rect::from_min_size(
            Pos2::new(x, rect.min.y),
            Vec2::new((w - 1.0).max(1.0), rect.height()),
        );
        let round = if i == 0 {
            Rounding {
                nw: 2.0,
                ne: 0.0,
                sw: 2.0,
                se: 0.0,
            }
        } else if i + 1 == colors.len() {
            Rounding {
                nw: 0.0,
                ne: 2.0,
                sw: 0.0,
                se: 2.0,
            }
        } else {
            Rounding::ZERO
        };
        painter.rect_filled(cell, round, *color);
        x += w;
    }
}

fn menu_frame(p: &ThemePalette) -> egui::Frame {
    egui::Frame::none()
        .fill(p.bar)
        .stroke(Stroke::new(1.0, p.line))
        .rounding(Rounding::same(8.0))
        .inner_margin(egui::Margin::same(6.0))
}

/// True when both paths name the same file (symlinks / relative paths resolved).
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn recent_open_key() -> Id {
    Id::new("pb_file_recent_open")
}

/// Time (egui seconds) the pointer was last inside the parent row, the submenu, or the gap
/// between them. The submenu stays open for a short grace period after that so a slow or
/// diagonal move toward it does not close it.
fn recent_hover_time_key() -> Id {
    Id::new("pb_file_recent_hover_time")
}

const RECENT_CLOSE_GRACE: f64 = 0.35;

/// Small right-pointing triangle marking a row that opens a submenu.
fn submenu_arrow(painter: &egui::Painter, row: Rect, p: &ThemePalette) {
    let c = Pos2::new(row.max.x - 12.0, row.center().y);
    let pts = vec![
        Pos2::new(c.x - 2.5, c.y - 4.0),
        Pos2::new(c.x + 2.5, c.y),
        Pos2::new(c.x - 2.5, c.y + 4.0),
    ];
    painter.add(egui::Shape::convex_polygon(pts, p.muted, Stroke::NONE));
}

/// Hover submenu listing recent projects, opened to the right of its parent row. It stays open
/// while the pointer is over either the parent row or the submenu itself.
#[allow(clippy::too_many_arguments)]
fn recent_submenu(
    ctx: &egui::Context,
    row: Rect,
    row_hovered: bool,
    recents: &[PathBuf],
    has_entries: bool,
    current: &Path,
    p: &ThemePalette,
    action: &mut Option<MenuAction>,
) {
    let was_open = ctx.data(|d| d.get_temp::<bool>(recent_open_key())).unwrap_or(false);
    if !row_hovered && !was_open {
        return;
    }
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let now = ctx.input(|i| i.time);
    let inner = egui::Area::new(Id::new("pb_menu_file_recent"))
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::new(row.max.x + 4.0, row.min.y - 8.0))
        .interactable(true)
        .show(ctx, |ui| {
            menu_frame(p).show(ui, |ui| {
                if recents.is_empty() {
                    let _ = menu_row(ui, "(no recent projects)", None, p);
                }
                // Two projects can share a file name (e.g. two folders each holding project.json):
                // show the full path for those so the entries can be told apart.
                let stem_of = |path: &Path| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("project")
                        .to_string()
                };
                for path in recents {
                    let stem = stem_of(path);
                    let clashes = recents.iter().filter(|other| stem_of(other) == stem).count() > 1;
                    let name = if clashes {
                        format!("{stem} — {}", path.parent().map_or(String::new(), |d| d.display().to_string()))
                    } else {
                        stem
                    };
                    let is_current = same_file(path, current);
                    let resp = menu_row(ui, &name, is_current.then_some(true), p);
                    resp.clone().on_hover_text(path.display().to_string());
                    if resp.clicked() && !is_current {
                        *action = Some(MenuAction::OpenRecent(path.clone()));
                    }
                }
                if has_entries {
                    menu_sep(ui, p);
                    if menu_row(ui, "Clear recent projects", None, p).clicked() {
                        *action = Some(MenuAction::ClearRecent);
                    }
                }
            });
        });
    let sub = inner.response.rect;
    // Bridge across the gap (parent frame padding + offset) between the row and the submenu.
    let bridge = Rect::from_min_max(
        Pos2::new(row.min.x, row.min.y),
        Pos2::new(sub.min.x + 1.0, row.max.y),
    );
    let in_zone = row_hovered
        || pointer.is_some_and(|pos| sub.contains(pos) || bridge.contains(pos));
    if in_zone {
        ctx.data_mut(|d| d.insert_temp(recent_hover_time_key(), now));
    }
    let last = ctx.data(|d| d.get_temp::<f64>(recent_hover_time_key())).unwrap_or(now);
    let open = in_zone || now - last < RECENT_CLOSE_GRACE;
    if open && !in_zone {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    ctx.data_mut(|d| d.insert_temp(recent_open_key(), open));
}

fn menu_row(
    ui: &mut egui::Ui,
    label: &str,
    check: Option<bool>,
    p: &ThemePalette,
) -> egui::Response {
    menu_row_padded(ui, label, check, 0.0, p)
}

/// A row with `extra_right` px kept free on the right (room for a submenu arrow).
fn menu_row_padded(
    ui: &mut egui::Ui,
    label: &str,
    check: Option<bool>,
    extra_right: f32,
    p: &ThemePalette,
) -> egui::Response {
    // Content-sized rows — File/View were hard-coded too wide (PB-101).
    let text_x = if check.is_some() { 32.0 } else { 12.0 };
    let pad_right = 14.0 + extra_right;
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), font_ui(13.0), p.text));
    let width = (text_x + galley.size().x + pad_right).max(96.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(5.0), tint(p.accent, 18));
    }
    if check == Some(true) {
        art::paint_check(
            ui.painter(),
            Pos2::new(rect.min.x + 16.0, rect.center().y),
            p.accent,
        );
    }
    ui.painter().galley(
        Pos2::new(rect.min.x + text_x, rect.center().y - galley.size().y * 0.5),
        galley,
        p.text,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn menu_sep(ui: &mut egui::Ui, p: &ThemePalette) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(96.0), 9.0),
        Sense::hover(),
    );
    let y = rect.center().y;
    ui.painter().line_segment(
        [
            Pos2::new(rect.min.x + 8.0, y),
            Pos2::new(rect.max.x - 8.0, y),
        ],
        Stroke::new(1.0, p.line),
    );
}

fn rail_caption(ui: &mut egui::Ui, text: &str, p: &ThemePalette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 14.0), Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font_ui(9.0),
        p.muted,
    );
}

fn rail_button(ui: &mut egui::Ui, pane: DockPane, on: bool, p: &ThemePalette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    let ink = pane.ink(p.is_light);
    let painter = ui.painter();
    if on {
        painter.rect_filled(
            rect,
            Rounding::same(8.0),
            tint(ink, if p.is_light { 40 } else { 52 }),
        );
        painter.rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0, ink));
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.min.x + 3.0, rect.center().y - 7.0),
                Vec2::new(2.0, 14.0),
            ),
            Rounding::same(1.0),
            ink,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, Rounding::same(8.0), tint(ink, 22));
    }
    let color = if on {
        ink
    } else if resp.hovered() {
        p.text
    } else {
        p.muted
    };
    art::paint_pane_icon(
        painter,
        Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
        pane,
        color,
    );
    resp.on_hover_text(pane.title())
        .on_hover_cursor(CursorIcon::PointingHand)
}

fn service_connection_chip(ui: &mut egui::Ui, connected: bool, p: &ThemePalette) -> egui::Response {
    let (label, ink) = if connected {
        ("Service: Connected", p.ok)
    } else {
        ("Service: Disconnected", p.err)
    };
    let galley = ui.fonts(|fonts| {
        fonts.layout_no_wrap(label.to_owned(), font_ui(12.0), p.text)
    });
    let size = Vec2::new(galley.size().x + 22.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter()
            .rect_filled(rect, Rounding::same(4.0), tint(ink, if p.is_light { 24 } else { 36 }));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let dot = Pos2::new(rect.min.x + 8.0, rect.center().y);
    ui.painter().circle_filled(dot, 3.5, ink);
    ui.painter()
        .galley(Pos2::new(rect.min.x + 16.0, rect.center().y - galley.size().y * 0.5), galley, p.text);
    resp.on_hover_text("Backend service IPC — click to re-check")
}

fn paint_about_wordmark(ui: &mut egui::Ui, p: &ThemePalette) {
    // Centered "Protocol" + accent "Bench" on the full row width.
    let gap = 4.0;
    let protocol = ui.fonts(|f| f.layout_no_wrap("Protocol".to_owned(), font_semi(24.0), p.text));
    let bench = ui.fonts(|f| f.layout_no_wrap("Bench".to_owned(), font_semi(24.0), p.accent));
    let width = protocol.size().x + gap + bench.size().x;
    let height = protocol.size().y.max(bench.size().y);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let origin = Pos2::new(rect.center().x - width * 0.5, rect.center().y - height * 0.5);
    let painter = ui.painter();
    painter.galley(origin, protocol, p.text);
    painter.galley(
        Pos2::new(origin.x + width - bench.size().x, origin.y),
        bench,
        p.accent,
    );
}

fn text_button(ui: &mut egui::Ui, label: &str, p: &ThemePalette) -> egui::Response {
    ui.add_sized(
        Vec2::new(ui.available_width(), 34.0),
        egui::Button::new(egui::RichText::new(label).font(font_ui(13.0)).color(p.text))
            .fill(p.control)
            .rounding(Rounding::same(6.0))
            .stroke(Stroke::new(1.0, p.line)),
    )
}

fn tile_splitter(
    ui: &mut egui::Ui,
    vertical_bar: bool,
    cross: f32,
    gap: f32,
    usable: f32,
    ratio: f32,
    blocked: bool,
    p: &ThemePalette,
) -> Option<f32> {
    let size = if vertical_bar {
        Vec2::new(gap, cross)
    } else {
        Vec2::new(cross, gap)
    };
    // The hit area stays the full gap so the panels do not move. Only the bar is thin.
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let fill = if blocked {
        p.err
    } else if resp.hovered() || resp.dragged() {
        p.accent
    } else {
        // About 40% transparent. The 2px bar and the gap around it stay as they are.
        tint(p.line, 153)
    };
    ui.painter()
        .rect_filled(divider_bar(rect, vertical_bar), Rounding::same(1.0), fill);
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(if vertical_bar {
            CursorIcon::ResizeHorizontal
        } else {
            CursorIcon::ResizeVertical
        });
    }
    if resp.double_clicked() {
        return Some(0.5);
    }
    if resp.dragged() && !blocked {
        let delta = if vertical_bar {
            resp.drag_delta().x / usable
        } else {
            resp.drag_delta().y / usable
        };
        return Some((ratio + delta).clamp(0.08, 0.92));
    }
    None
}

fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

fn split_axis(total: f32, gap: f32, ratio: f32) -> (f32, f32, f32) {
    let usable = (total - gap).max(2.0);
    let first = (usable * ratio).clamp(1.0, usable - 1.0);
    (first, usable - first, usable)
}

fn about_chip(ui: &mut egui::Ui, label: &str, ink: Color32, p: &ThemePalette) {
    let galley = ui.fonts(|fonts| fonts.layout_no_wrap(label.to_owned(), font_ui(12.0), ink));
    let size = galley.size() + Vec2::new(16.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let alpha = if p.is_light { 28 } else { 40 };
    ui.painter()
        .rect_filled(rect, Rounding::same(5.0), tint(ink, alpha));
    ui.painter()
        .rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0, ink));
    ui.painter()
        .galley(rect.min + Vec2::new(8.0, 4.0), galley, ink);
}


/// Content of one [`confirm_modal`].
struct ConfirmSpec<'a> {
    /// Stable egui id; one per dialog so their layers never collide.
    id: &'static str,
    title: &'a str,
    /// One-line explanation under the title.
    intro: &'a str,
    /// Live things listed in the boxed body (capped, the rest shown as "+ N more").
    items: &'a [String],
    /// Optional muted line under the list (e.g. the project about to open).
    detail: Option<String>,
    /// Main action (orange, rightmost).
    primary: &'a str,
    /// Dismiss action (leftmost); Esc and a scrim click map here too.
    secondary: &'a str,
    /// Optional destructive third choice (red), e.g. "Stop all and quit".
    danger: Option<&'a str>,
}

/// Which button closed a [`confirm_modal`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Primary,
    Secondary,
    Danger,
}

/// Themed confirmation modal: dimmed scrim, warning badge, running list, two buttons.
/// Secondary is also Esc / scrim click; `None` = still open.
///
/// Size is computed (not measured) so the window lands on whole points: a content-sized window
/// centred on an odd screen edge puts its 1px border on half pixels, which shows as a fuzzy,
/// jagged outline.
fn confirm_modal(ctx: &egui::Context, p: &ThemePalette, spec: &ConfirmSpec) -> Option<Choice> {
    const W: f32 = 520.0;
    const PAD: f32 = 24.0;
    const ROW: f32 = 22.0;
    const MAX_ROWS: usize = 6;

    let shown = spec.items.len().min(MAX_ROWS);
    let extra = spec.items.len() - shown;
    let rows = shown + usize::from(extra > 0);
    let list_h = if rows == 0 { 0.0 } else { ROW * rows as f32 + 16.0 };
    let detail_h = if spec.detail.is_some() { 38.0 } else { 0.0 };
    let h = PAD + 44.0 + 16.0 + list_h + detail_h + 22.0 + 34.0 + PAD;
    let size = Vec2::new(W, h.ceil());

    let screen = ctx.screen_rect();
    let pos = Pos2::new(
        (screen.center().x - size.x * 0.5).round(),
        (screen.center().y - size.y * 0.5).round(),
    );

    let mut decision = None;
    let scrim = egui::Area::new(Id::new((spec.id, "scrim")))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .interactable(true)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter()
                .rect_filled(rect, Rounding::ZERO, Color32::from_black_alpha(96));
            resp
        });
    if scrim.inner.clicked() {
        decision = Some(Choice::Secondary);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        decision = Some(Choice::Secondary);
    }

    let frame = egui::Frame::none()
        .fill(p.pane)
        .stroke(Stroke::new(1.0, p.line))
        .rounding(Rounding::same(10.0))
        .shadow(egui::epaint::Shadow {
            offset: Vec2::new(0.0, 8.0),
            blur: 24.0,
            spread: 0.0,
            color: Color32::from_black_alpha(90),
        })
        .inner_margin(egui::Margin::same(PAD));
    egui::Window::new(spec.id)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .fixed_size(size - Vec2::splat(PAD * 2.0))
        .frame(frame)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let inner_w = W - PAD * 2.0;

            // Header: badge + title / intro.
            let (head, _) = ui.allocate_exact_size(Vec2::new(inner_w, 44.0), Sense::hover());
            art::paint_warning_badge(
                ui.painter(),
                Rect::from_min_size(head.min, Vec2::splat(44.0)),
                p.warn,
            );
            ui.painter().text(
                Pos2::new(head.min.x + 58.0, head.min.y + 13.0),
                Align2::LEFT_CENTER,
                spec.title,
                font_semi(16.0),
                p.text,
            );
            ui.painter().text(
                Pos2::new(head.min.x + 58.0, head.min.y + 33.0),
                Align2::LEFT_CENTER,
                spec.intro,
                font_ui(12.5),
                p.muted,
            );
            ui.add_space(16.0);

            // What is running.
            if rows > 0 {
                let (list, _) = ui.allocate_exact_size(Vec2::new(inner_w, list_h), Sense::hover());
                ui.painter().rect_filled(list, Rounding::same(6.0), p.control);
                ui.painter()
                    .rect_stroke(list, Rounding::same(6.0), Stroke::new(1.0, p.line));
                let clip = Rect::from_min_max(
                    Pos2::new(list.min.x + 30.0, list.min.y),
                    Pos2::new(list.max.x - 10.0, list.max.y),
                );
                for (i, item) in spec.items.iter().take(shown).enumerate() {
                    let y = list.min.y + 8.0 + ROW * i as f32 + ROW * 0.5;
                    ui.painter()
                        .circle_filled(Pos2::new(list.min.x + 16.0, y), 3.0, p.live);
                    ui.painter().with_clip_rect(clip).text(
                        Pos2::new(clip.min.x, y),
                        Align2::LEFT_CENTER,
                        item,
                        font_ui(12.5),
                        p.text,
                    );
                }
                if extra > 0 {
                    let y = list.min.y + 8.0 + ROW * shown as f32 + ROW * 0.5;
                    ui.painter().text(
                        Pos2::new(clip.min.x, y),
                        Align2::LEFT_CENTER,
                        format!("+ {extra} more"),
                        font_ui(12.0),
                        p.muted,
                    );
                }
            }
            if let Some(detail) = &spec.detail {
                let (row, _) = ui.allocate_exact_size(Vec2::new(inner_w, 38.0), Sense::hover());
                ui.painter().with_clip_rect(row).text(
                    Pos2::new(row.min.x, row.center().y + 6.0),
                    Align2::LEFT_CENTER,
                    detail,
                    font_ui(11.5),
                    p.muted,
                );
            }
            ui.add_space(22.0);

            // Buttons, right-aligned: secondary then primary.
            let (bar, _) = ui.allocate_exact_size(Vec2::new(inner_w, 34.0), Sense::hover());
            let mut bar_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar)
                    .layout(Layout::right_to_left(egui::Align::Center)),
            );
            bar_ui.spacing_mut().item_spacing.x = 10.0;
            let on_accent = if p.is_light { Color32::WHITE } else { p.bg };
            let primary = egui::Button::new(
                egui::RichText::new(spec.primary)
                    .font(font_semi(13.0))
                    .color(on_accent),
            )
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(7.0))
            .min_size(Vec2::new(130.0, 34.0));
            if bar_ui.add(primary).clicked() {
                decision = Some(Choice::Primary);
            }
            if let Some(label) = spec.danger {
                let danger = egui::Button::new(
                    egui::RichText::new(label)
                        .font(font_semi(13.0))
                        .color(Color32::WHITE),
                )
                .fill(p.err)
                .stroke(Stroke::NONE)
                .rounding(Rounding::same(7.0))
                .min_size(Vec2::new(140.0, 34.0));
                if bar_ui.add(danger).clicked() {
                    decision = Some(Choice::Danger);
                }
            }
            let secondary = egui::Button::new(
                egui::RichText::new(spec.secondary)
                    .font(font_ui(13.0))
                    .color(p.text),
            )
            .fill(p.control)
            .stroke(Stroke::new(1.0, p.line))
            .rounding(Rounding::same(7.0))
            .min_size(Vec2::new(96.0, 34.0));
            if bar_ui.add(secondary).clicked() {
                decision = Some(Choice::Secondary);
            }
        });
    decision
}

/// Shorten `text` to `max` chars keeping the head and the tail (file name stays visible).
fn elide_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let tail = max * 2 / 3;
    let head = max - tail - 1;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}
