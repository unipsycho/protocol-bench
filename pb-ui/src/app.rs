//! Protocol Bench shell: menu bar, icon rail, docked panes, themes.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Layout, Pos2, Rect, Rounding, Sense, Stroke, Vec2,
};
use pb_engine::EngineHandle;

use crate::art::{self, ChromeIcon};
use crate::dnd::{self, DropCommit, PaneHit};
use crate::dock::DockPane;
use crate::layout::{LayoutNode, SplitDir, TabGroup};
use crate::panels;
use crate::persist::{self, UiState};
use crate::theme::{
    self, font_semi, font_ui, tint, AppTheme, ThemePalette, BAR_H, HEADER_H, MIN_PANE, RADIUS,
    RAIL_W, SEP, STATUS_H,
};

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
    dirty: bool,
    last_save: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BarMenu {
    File,
    View,
    Help,
    Theme,
}

enum MenuAction {
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
        Self {
            engine: EngineHandle::spawn(),
            ui_state,
            theme,
            theme_applied: Some(theme),
            pane_hits: Vec::new(),
            pending: None,
            about_open: false,
            settings_open: false,
            menu: None,
            status: "Ready".into(),
            dirty: false,
            last_save: Instant::now(),
        }
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
            MenuAction::ResetLayout => self.reset_layout(),
            MenuAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
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
        let host = self.engine.snapshot();
        let p = self.theme.palette();
        theme::paint_root_bg(ctx, &p);
        self.pane_hits.clear();

        self.app_bar(ctx, &p);
        self.status_bar(ctx, &p, &host);
        self.pane_rail(ctx, &p);
        self.settings_drawer(ctx, &p, &host);
        self.about_window(ctx, &p);

        let host_for_panes = host;
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
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.ui_state.theme_id = self.theme.id().into();
        let _ = persist::save(&self.ui_state);
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
        self.popup(ctx, anchor, BarMenu::File, p, action, |ui, action| {
            if menu_row(ui, "Reset layout", None, p).clicked() {
                *action = Some(MenuAction::ResetLayout);
            }
            menu_sep(ui, p);
            if menu_row(ui, "Quit", None, p).clicked() {
                *action = Some(MenuAction::Quit);
            }
        });
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
            let inner = menu_frame(p).show(ui, |ui| add(ui, action)).response;
            if inner.clicked_elsewhere() && !anchor.clicked {
                close = true;
            }
        });
        if close && action.is_none() {
            *action = Some(MenuAction::Close);
        }
    }

    fn status_bar(&self, ctx: &egui::Context, p: &ThemePalette, host: &pb_engine::HostSnapshot) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(STATUS_H)
            .frame(
                egui::Frame::none()
                    .fill(p.bar)
                    .inner_margin(egui::Margin::symmetric(14.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let cluster = 250.0;
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
                    engine_cluster(ui, host, p);
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
                ui.add_space(12.0);
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
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if art::chrome_button(ui, ChromeIcon::Close, "Close", false, p, 26.0).clicked() {
                            open = false;
                        }
                    });
                });
                let (band, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 84.0), Sense::hover());
                let mark = Rect::from_center_size(band.center(), Vec2::splat(84.0));
                art::paint_mark(ui.painter(), mark, p);
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(egui::RichText::new("Protocol").font(font_semi(24.0)).color(p.text));
                        ui.label(egui::RichText::new("Bench").font(font_semi(24.0)).color(p.accent));
                    });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).font(font_ui(13.0)).color(p.muted));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Agile Automation").font(font_ui(12.5)).color(p.muted));
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Serial, Modbus, MQTT, OPC UA, and file follow on one bench.")
                            .font(font_ui(13.0))
                            .color(p.text),
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
                        ("File follow", DockPane::Files),
                        ("Points", DockPane::Points),
                    ] {
                        about_chip(ui, label, pane.ink_core(), p);
                    }
                });
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Protocol I/O will run on the pb-engine thread. This build does not open a link.")
                        .font(font_ui(12.0))
                        .color(p.muted),
                );
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
            let hub_change = panels::paint(&mut body_ui, active, host, p, &mut self.ui_state.hubs);
            if hub_change.dirty {
                self.mark_dirty();
            }
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

fn menu_row(
    ui: &mut egui::Ui,
    label: &str,
    check: Option<bool>,
    p: &ThemePalette,
) -> egui::Response {
    let width = if check.is_some() { 220.0 } else { 180.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(5.0), tint(p.accent, 18));
    }
    let text_x = if check.is_some() { 32.0 } else { 12.0 };
    if check == Some(true) {
        art::paint_check(
            ui.painter(),
            Pos2::new(rect.min.x + 16.0, rect.center().y),
            p.accent,
        );
    }
    ui.painter().text(
        Pos2::new(rect.min.x + text_x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font_ui(13.0),
        p.text,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn menu_sep(ui: &mut egui::Ui, p: &ThemePalette) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(160.0), 9.0),
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

fn engine_cluster(ui: &mut egui::Ui, host: &pb_engine::HostSnapshot, p: &ThemePalette) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for slot in &host.slots {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        let ink = DockPane::from_engine(slot.id).ink(p.is_light);
        ui.painter().circle_filled(rect.center(), 3.5, ink);
        resp.on_hover_text(format!("{} · {}", slot.title, slot.note));
    }
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!(
            "{}  {}",
            host.thread_name,
            panels::format_uptime(host.uptime_ms)
        ))
        .font(theme::font_mono(11.5))
        .color(p.muted),
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
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let fill = if blocked {
        p.err
    } else if resp.hovered() || resp.dragged() {
        p.accent
    } else {
        p.line
    };
    ui.painter().rect_filled(rect, Rounding::ZERO, fill);
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
