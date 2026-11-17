//! The Spacious window: app bar, breadcrumb bar, treemap or list, status line.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, FontId, CornerRadius, Id, Key, Layout, Modifiers, PointerButton, Rect, Sense, Stroke, StrokeKind,
    Ui, pos2, vec2,
};

use crate::live::{Live, Patch, apply_patches};
use crate::scan::{NONE, Tree};
use crate::stream::{self, Builder, ScanJob};
use crate::theme::*;
use crate::treemap::{self, Cell, HEADER_H, Item};
use crate::util::{Cat, expand, fmt_count, fmt_duration, fmt_size, fmt_time, home_dir, tilde};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Treemap,
    List,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortCol {
    Name,
    Size,
    Items,
    Modified,
}

#[derive(Clone, Debug)]
enum Action {
    ZoomTo(u32),
    Select(u32),
    Open(u32),
    Reveal(u32),
    Terminal(u32),
    CopyPath(u32),
    CopySize(u32),
    ScanFolder(u32),
    Props(u32),
    /// Open or close a folder in place.
    Toggle(u32),
    CloseAll,
    Up,
    /// Back to the scanned folder.
    Top,
    View(View),
    FocusSearch,
    Theme(ThemeKind),
    ScanPath(String),
    Rescan,
    Browse,
    PrivacySettings,
}

struct Toast {
    text: String,
    at: Instant,
}

struct Props {
    node: u32,
    top: Vec<u32>,
}

struct SearchJob {
    key: (String, u32, u64),
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<(Vec<u32>, usize)>,
}

struct SearchResult {
    key: (String, u32, u64),
    items: Vec<u32>,
    total: usize,
}

const MAX_RESULTS: usize = 20_000;

pub struct App {
    path_input: String,
    tree: Option<Arc<Tree>>,
    generation: u64,
    job: Option<ScanJob>,
    /// Fills the tree from the running scan's messages.
    builder: Option<Builder>,
    /// Bumped whenever the running scan adds to the tree, so views redraw.
    scan_rev: u64,
    /// Path to open once the running scan finishes (rescan keeps your place).
    goto_after_scan: Option<String>,
    root: u32,
    selected: Option<u32>,
    view: View,
    search: String,
    /// Categories switched off in the legend: their files are dimmed.
    hidden: [bool; Cat::ALL.len()],

    cells: Vec<Cell>,
    dim: Vec<bool>,
    cells_key: Option<(u64, u32, [i32; 4], String, [bool; Cat::ALL.len()], u64)>,
    /// Folders opened in place in the treemap.
    expanded: HashSet<u32>,
    expanded_gen: u64,

    list: Vec<u32>,
    list_key: Option<(u64, u32, SortCol, bool)>,
    sort: (SortCol, bool),

    search_job: Option<SearchJob>,
    search_result: Option<SearchResult>,

    menu_target: Option<Item>,
    props: Option<Props>,
    toast: Option<Toast>,
    recent: Vec<String>,
    focus_search: bool,
    focus_path: bool,
    /// Watches the scanned folder and keeps the tree up to date.
    live: Option<Live>,
    pending: Vec<Patch>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<String>) -> Self {
        install(&cc.egui_ctx, load_theme());
        let recent = load_recent();
        let mut app = App {
            path_input: tilde(initial.as_deref().unwrap_or("/")),
            tree: None,
            generation: 0,
            job: None,
            builder: None,
            scan_rev: 0,
            goto_after_scan: None,
            root: 0,
            selected: None,
            view: View::Treemap,
            search: String::new(),
            hidden: [false; Cat::ALL.len()],
            cells: Vec::new(),
            dim: Vec::new(),
            cells_key: None,
            expanded: HashSet::new(),
            expanded_gen: 0,
            list: Vec::new(),
            list_key: None,
            sort: (SortCol::Size, true),
            search_job: None,
            search_result: None,
            menu_target: None,
            props: None,
            toast: None,
            recent,
            focus_search: false,
            focus_path: false,
            live: None,
            pending: Vec::new(),
        };
        // Scan the whole drive unless a folder was given.
        let start = initial.unwrap_or_else(|| "/".into());
        app.start_scan(&cc.egui_ctx, expand(&start));
        app
    }

    // ── Scanning ────────────────────────────────────────────────────────────

    fn start_scan(&mut self, ctx: &egui::Context, path: String) {
        let pb = PathBuf::from(&path);
        if !pb.is_dir() {
            self.toast_err(format!(
                "Can't scan {}: that folder doesn't exist. Check the path, or press ⌘O to choose one.",
                tilde(&path)
            ));
            return;
        }
        if let Some(j) = self.job.take() {
            j.cancel();
        }
        self.live = None;
        self.pending.clear();
        self.path_input = tilde(&path);
        self.recent.retain(|r| r != &path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(10);
        save_recent(&self.recent);

        // Show the map straight away and let the scan fill it in.
        let old = self.tree.replace(Arc::new(stream::new_tree(&path)));
        if let Some(old) = old {
            // Freeing millions of names takes a moment: do it off the UI thread.
            std::thread::spawn(move || drop(old));
        }
        self.builder = Some(Builder::new());
        self.generation += 1;
        self.root = 0;
        self.selected = None;
        self.props = None;
        self.menu_target = None;
        self.expanded.clear();
        self.expanded_gen += 1;
        self.search_result = None;
        let c = ctx.clone();
        self.job = Some(stream::start(path, move || c.request_repaint()));
    }

    fn rescan(&mut self, ctx: &egui::Context) {
        let Some(tree) = self.tree.clone() else {
            self.start_scan(ctx, expand(&self.path_input));
            return;
        };
        self.goto_after_scan = Some(tree.path(self.root));
        self.start_scan(ctx, tree.root_path.clone());
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        ctx.request_repaint_after(Duration::from_millis(100));
        let (Some(arc), Some(builder)) = (self.tree.as_mut(), self.builder.as_mut()) else { return };
        // A search thread may hold the tree for a moment: apply next frame.
        let Some(tree) = Arc::get_mut(arc) else { return };
        let done = job.done();
        // Apply what has arrived, within a few milliseconds so the window stays smooth.
        let budget = Instant::now() + Duration::from_millis(8);
        let mut applied = 0u32;
        let mut drained = false;
        loop {
            match job.try_recv() {
                Some(m) => {
                    builder.apply(tree, m);
                    applied += 1;
                    if applied % 256 == 0 && Instant::now() > budget {
                        break;
                    }
                }
                None => {
                    drained = true;
                    break;
                }
            }
        }
        if applied > 0 {
            self.scan_rev += 1;
        }
        if !(done && drained) {
            return;
        }

        // The scan has ended: sort, compact, and keep the user's place.
        let Some(mut arc) = self.tree.take() else { return };
        let Some(tree) = Arc::get_mut(&mut arc) else {
            self.tree = Some(arc);
            return;
        };
        let job = self.job.take().unwrap();
        self.builder = None;
        let places = remember(self, tree);
        stream::finish(tree, job.started.elapsed());
        let goto = self.goto_after_scan.take();
        restore(self, tree, places);
        if let Some(p) = goto
            && let Some(i) = tree.find(&p)
            && tree.node(i).is_dir
        {
            self.root = i;
        }
        let root_path = tree.root_path.clone();
        self.tree = Some(arc);
        self.generation += 1;
        if job.cancelled() {
            self.toast("Scan stopped. The map shows what was read so far.");
        } else {
            let c = ctx.clone();
            self.live = Live::start(root_path, move || c.request_repaint());
        }
    }

    // ── Live updates ────────────────────────────────────────────────────────

    fn poll_live(&mut self, ctx: &egui::Context) {
        if let Some(l) = &self.live {
            self.pending.extend(l.poll());
        }
        if self.pending.is_empty() || self.job.is_some() {
            return;
        }
        let Some(mut arc) = self.tree.take() else {
            self.pending.clear();
            return;
        };
        // A search thread may still hold the tree: try again shortly.
        let Some(tree) = Arc::get_mut(&mut arc) else {
            self.tree = Some(arc);
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        };

        // Indices move during edits: remember places by path, including what an open
        // right-click menu points at, so the menu stays as it is.
        let places = remember(self, tree);
        let requests = apply_patches(tree, std::mem::take(&mut self.pending));
        restore(self, tree, places);
        self.tree = Some(arc);
        self.generation += 1;
        if let Some(l) = &self.live {
            for r in requests {
                l.request(r);
            }
        }
    }

    // ── Search ──────────────────────────────────────────────────────────────

    fn poll_search(&mut self, ctx: &egui::Context) {
        let q = self.search.trim().to_lowercase();
        let Some(tree) = self.tree.clone() else { return };
        if q.is_empty() {
            self.search_job = None;
            self.search_result = None;
            return;
        }
        let key = (q.clone(), self.root, self.generation);
        if self.search_result.as_ref().is_some_and(|r| r.key == key) {
            return;
        }
        if let Some(job) = &self.search_job {
            if job.key == key {
                match job.rx.try_recv() {
                    Ok((items, total)) => {
                        self.search_result = Some(SearchResult { key, items, total });
                        self.search_job = None;
                    }
                    Err(_) => ctx.request_repaint_after(Duration::from_millis(30)),
                }
                return;
            }
            job.cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (c, root, ctx2) = (cancel.clone(), self.root, ctx.clone());
        std::thread::spawn(move || {
            if let Some(r) = search_tree(&tree, root, &q, &c) {
                let _ = tx.send(r);
                ctx2.request_repaint();
            }
        });
        self.search_job = Some(SearchJob { key, cancel, rx });
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn searching(&self) -> bool {
        !self.search.trim().is_empty()
    }

    // ── Actions ─────────────────────────────────────────────────────────────

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), at: Instant::now() });
    }
    /// Errors use the same toast and lead with what failed.
    fn toast_err(&mut self, text: impl Into<String>) {
        self.toast(text);
    }

    /// The panel menu (≡) and the right-click menu of empty areas: every command, with its
    /// shortcut.
    fn main_menu(&self, ui: &mut Ui) -> Option<Action> {
        ui.set_min_width(220.0);
        let mut a = None;
        let has_tree = self.tree.is_some() && self.job.is_none();
        if menu_item(ui, "Scan Folder…", "⌘O") {
            a = Some(Action::Browse);
        }
        if menu_item(ui, "Rescan", "⌘R") {
            a = Some(Action::Rescan);
        }
        ui.separator();
        if check_item(ui, "Treemap", "⌘1", self.view == View::Treemap) {
            a = Some(Action::View(View::Treemap));
        }
        if check_item(ui, "List", "⌘2", self.view == View::List) {
            a = Some(Action::View(View::List));
        }
        ui.separator();
        if menu_item_on(ui, "Up One Level", "⌫", has_tree && self.root != 0) {
            a = Some(Action::Up);
        }
        if menu_item_on(ui, "Back to Top", "Esc", has_tree && self.root != 0) {
            a = Some(Action::Top);
        }
        if menu_item_on(ui, "Close All Folders", "", has_tree && !self.expanded.is_empty()) {
            a = Some(Action::CloseAll);
        }
        if menu_item(ui, "Search Names", "⌘K") {
            a = Some(Action::FocusSearch);
        }
        if menu_item_on(ui, "Properties…", "⌘I", has_tree && self.selected.is_some())
            && let Some(sel) = self.selected
        {
            a = Some(Action::Props(sel));
        }
        ui.separator();
        ui.menu_button("Theme", |ui| {
            for k in ThemeKind::ALL {
                if check_item(ui, k.name(), "", tk().kind == k) {
                    a = Some(Action::Theme(k));
                }
            }
        });
        a
    }

    fn apply(&mut self, ctx: &egui::Context, a: Action) {
        // Commands that don't need a scanned folder.
        match a {
            Action::Browse => return self.browse(ctx),
            Action::Rescan => return self.rescan(ctx),
            Action::ScanPath(p) => return self.start_scan(ctx, p),
            Action::View(v) => {
                self.view = v;
                return;
            }
            Action::FocusSearch => {
                self.focus_search = true;
                return;
            }
            Action::Theme(k) => {
                set_theme(ctx, k);
                save_theme(k);
                return;
            }
            _ => {}
        }
        let Some(tree) = self.tree.clone() else { return };
        match a {
            Action::ZoomTo(i) => {
                if tree.node(i).is_dir {
                    self.root = i;
                }
            }
            Action::Select(i) => self.selected = Some(i),
            Action::Open(i) => run("open", &[&tree.path(i)], self),
            Action::Reveal(i) => run("open", &["-R", &tree.path(i)], self),
            Action::Terminal(i) => run("open", &["-a", "Terminal", &tree.path(i)], self),
            Action::CopyPath(i) => {
                ctx.copy_text(tree.path(i));
                self.toast("Path copied.");
            }
            Action::CopySize(i) => {
                ctx.copy_text(fmt_size(tree.node(i).size));
                self.toast("Size copied.");
            }
            Action::ScanFolder(i) => self.start_scan(ctx, tree.path(i)),
            Action::Props(i) => self.props = Some(Props { node: i, top: tree.top_files(i, 5) }),
            Action::Up => {
                if self.root != 0 {
                    let from = self.root;
                    self.root = tree.node(self.root).parent;
                    self.selected = Some(from);
                }
            }
            Action::Toggle(i) => {
                if !self.expanded.remove(&i) {
                    self.expanded.insert(i);
                }
                self.expanded_gen += 1;
            }
            Action::CloseAll => {
                self.expanded.clear();
                self.expanded_gen += 1;
            }
            Action::Top => self.root = 0,
            Action::Rescan
            | Action::Browse
            | Action::ScanPath(_)
            | Action::View(_)
            | Action::FocusSearch
            | Action::Theme(_) => {}
            Action::PrivacySettings => run(
                "open",
                &["x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"],
                self,
            ),
        }
    }

    fn browse(&mut self, ctx: &egui::Context) {
        let start = expand(&self.path_input);
        if let Some(p) = rfd::FileDialog::new().set_directory(&start).pick_folder() {
            self.start_scan(ctx, p.to_string_lossy().into_owned());
        }
    }

    // ── Keyboard ────────────────────────────────────────────────────────────

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let mut acts = Vec::new();
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::COMMAND, Key::O) {
                acts.push(Action::Browse);
            }
            if i.consume_key(Modifiers::COMMAND, Key::R) {
                acts.push(Action::Rescan);
            }
            if i.consume_key(Modifiers::COMMAND, Key::K) || i.consume_key(Modifiers::COMMAND, Key::F) {
                self.focus_search = true;
            }
            if i.consume_key(Modifiers::COMMAND, Key::L) {
                self.focus_path = true;
            }
            if i.consume_key(Modifiers::COMMAND, Key::Num1) {
                self.view = View::Treemap;
            }
            if i.consume_key(Modifiers::COMMAND, Key::Num2) {
                self.view = View::List;
            }
            if i.consume_key(Modifiers::COMMAND, Key::ArrowUp) || i.pointer.button_pressed(PointerButton::Extra1) {
                acts.push(Action::Up);
            }
            if !typing {
                if i.consume_key(Modifiers::NONE, Key::Backspace) {
                    acts.push(Action::Up);
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    if !self.search.is_empty() {
                        self.search.clear();
                    } else {
                        acts.push(Action::Top);
                    }
                }
                if let Some(s) = self.selected {
                    if i.consume_key(Modifiers::NONE, Key::Enter) {
                        acts.push(Action::ZoomTo(s));
                    }
                    if i.consume_key(Modifiers::COMMAND, Key::I) {
                        acts.push(Action::Props(s));
                    }
                }
            }
        });
        for a in acts {
            self.apply(ctx, a);
        }
    }

    // ── App bar ─────────────────────────────────────────────────────────────

    fn app_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut act = None;
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                let sid = Id::new("app.search");
                let sresp = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .id(sid)
                        .hint_text("Search names   ⌘K")
                        .desired_width(220.0),
                );
                if self.focus_search {
                    sresp.request_focus();
                    self.focus_search = false;
                }
                if sresp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.search.clear();
                    sresp.surrender_focus();
                }
                ui.add_space(12.0);
                let cur = if self.view == View::Treemap { 0 } else { 1 };
                if let Some(v) = segmented(ui, "app.view", &["Treemap", "List"], cur) {
                    self.view = if v == 0 { View::Treemap } else { View::List };
                }
                ui.add_space(12.0);
                let scanning = self.job.is_some();
                if button(ui, "app.scan", "Scan", true, !scanning).clicked() {
                    self.start_scan(&ctx, expand(&self.path_input));
                }
                if button(ui, "app.browse", "Browse…", false, true).clicked() {
                    act = Some(Action::Browse);
                }
                // Recent folders.
                let recent = self.recent.clone();
                let rb = icon_button(ui, "app.recent", "⌄", !recent.is_empty()).on_hover_text("Recent folders");
                egui::Popup::menu(&rb).show(|ui| {
                    ui.set_min_width(280.0);
                    for r in &recent {
                        if menu_item(ui, &tilde(r), "") {
                            act = Some(Action::ScanPath(r.clone()));
                        }
                    }
                });
                let pid = Id::new("app.path");
                let presp = ui.add(
                    egui::TextEdit::singleline(&mut self.path_input)
                        .id(pid)
                        .hint_text("Folder to scan, e.g. ~/Desktop")
                        .font(mono(FS_LG))
                        .desired_width(ui.available_width()),
                );
                if self.focus_path {
                    presp.request_focus();
                    self.focus_path = false;
                }
                if presp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.start_scan(&ctx, expand(&self.path_input));
                }
            });
        });
        if let Some(a) = act {
            self.apply(&ctx, a);
        }
    }

    // ── Breadcrumb and legend ───────────────────────────────────────────────

    fn crumb_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut act = None;
        // Right-click anywhere on the bar for the general commands.
        let bg = ui.interact(ui.max_rect(), Id::new("crumbs.bar"), Sense::click());
        bg.context_menu(|ui| {
            if let Some(a) = self.main_menu(ui) {
                act = Some(a);
            }
        });
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(6.0);
            // Panel menu and legend on the right.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(4.0);
                let m = flat_icon(ui, "crumbs.menu", "≡", true).on_hover_text("Menu");
                ui.add_space(4.0);
                let sep = ui.allocate_exact_size(vec2(1.0, 12.0), Sense::hover()).0;
                ui.painter().vline(sep.center().x, sep.y_range(), Stroke::new(1.0, tk().separator));
                egui::Popup::menu(&m).show(|ui| {
                    if let Some(a) = self.main_menu(ui) {
                        act = Some(a);
                    }
                });
                ui.add_space(8.0);
                for (k, cat) in Cat::ALL.iter().enumerate().rev() {
                    let off = self.hidden[k];
                    let galley = ui.painter().layout_no_wrap(cat.name().into(), ui_font(FS_SM), tk().text_dim);
                    let (r, _) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, TAB_H), Sense::hover());
                    let resp = ui
                        .interact(r, Id::new(("legend", cat.name())), Sense::click())
                        .on_hover_text(if off { "Show these files" } else { "Dim these files" });
                    if resp.clicked() {
                        self.hidden[k] = !off;
                    }
                    let sw = Rect::from_center_size(pos2(r.min.x + 6.0, r.center().y), vec2(9.0, 9.0));
                    let p = ui.painter();
                    if off {
                        p.rect_stroke(sw, CornerRadius::same(1), Stroke::new(1.0, cat.color()), StrokeKind::Inside);
                    } else {
                        p.rect_filled(sw, CornerRadius::same(1), cat.color());
                    }
                    let col = if off {
                        tk().disabled_fg
                    } else if resp.hovered() {
                        tk().tab_text_active
                    } else {
                        tk().text_dim
                    };
                    p.galley_with_override_text_color(pos2(r.min.x + 14.0, r.center().y - galley.size().y / 2.0), galley, col);
                    ui.add_space(4.0);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let Some(tree) = self.tree.clone() else {
                        ui.label(egui::RichText::new("No folder scanned").color(tk().text_faint).size(FS_MD));
                        return;
                    };
                    let up = flat_icon(ui, "crumbs.up", "‹", self.root != 0).on_hover_text("Up one level   ⌫");
                    if up.clicked() {
                        act = Some(Action::Up);
                    }
                    ui.add_space(4.0);
                    let chain = tree.ancestors(self.root);
                    for (k, &n) in chain.iter().enumerate() {
                        if k > 0 {
                            ui.label(egui::RichText::new("›").color(tk().text_faint));
                        }
                        let name = if n == 0 { tilde(&tree.root_path) } else { tree.node(n).name.to_string() };
                        let last = k + 1 == chain.len();
                        let text = egui::RichText::new(name).size(FS_MD).color(if last { tk().tab_text_active } else { tk().tab_text });
                        let text = if last { text.font(medium(FS_MD)) } else { text };
                        let r = ui.push_id(("crumb", k), |ui| ui.add(egui::Label::new(text).sense(Sense::click()))).inner;
                        if r.clicked() {
                            act = Some(Action::ZoomTo(n));
                        }
                        if r.hovered() && !last {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                    let n = tree.node(self.root);
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(fmt_size(n.size)).size(FS_MD).color(tk().text_dim));
                });
            });
        });
        if let Some(a) = act {
            self.apply(&ctx, a);
        }
    }

    // ── Status line ─────────────────────────────────────────────────────────

    fn status_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut act = None;
        let bg = ui.interact(ui.max_rect(), Id::new("status.bar"), Sense::click());
        bg.context_menu(|ui| {
            if let Some(a) = self.main_menu(ui) {
                act = Some(a);
            }
        });
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                // Right to left: live state, totals, then any warning.
                let sep = |ui: &mut Ui| {
                    ui.add_space(4.0);
                    let r = ui.allocate_exact_size(vec2(1.0, 12.0), Sense::hover()).0;
                    ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, tk().separator));
                    ui.add_space(4.0);
                };
                let dim = |t: String| egui::RichText::new(t).size(FS_MD).color(tk().text_dim);
                if let Some(job) = &self.job {
                    if button(ui, "status.cancel", "Cancel", false, true).clicked() {
                        job.cancel();
                    }
                    ui.add_space(6.0);
                    let p = &job.progress;
                    ui.label(dim(format!(
                        "{} files · {} folders · {} · {}",
                        fmt_count(p.files.load(Ordering::Relaxed)),
                        fmt_count(p.dirs.load(Ordering::Relaxed)),
                        fmt_size(p.bytes.load(Ordering::Relaxed)),
                        fmt_duration(job.started.elapsed())
                    )));
                } else if let Some(t) = &self.tree {
                    if self.live.is_some() {
                        let live = ui.label(dim("Live".into()));
                        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 3.0, tk().success);
                        live.on_hover_text("Watching for changes. Deleted, added and changed files update the map within a second.");
                        sep(ui);
                    }
                    let r = t.node(0);
                    ui.label(dim(format!(
                        "{} · {} files · {} folders · scanned in {}",
                        fmt_size(r.size),
                        fmt_count(r.files),
                        fmt_count(r.dirs as u64),
                        fmt_duration(t.elapsed)
                    )));
                    if t.unreadable > 0 {
                        sep(ui);
                        let w = ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(format!("{} folders could not be read", fmt_count(t.unreadable)))
                                        .size(FS_MD)
                                        .color(tk().warning),
                                )
                                .sense(Sense::click()),
                            )
                            .on_hover_text(
                                "macOS blocks some folders. Click to open Full Disk Access settings, add Spacious, then rescan.",
                            );
                        if w.clicked() {
                            act = Some(Action::PrivacySettings);
                        }
                    }
                }
                ui.add_space(12.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if let Some(t) = &self.toast {
                        if t.at.elapsed() > Duration::from_secs(4) {
                            self.toast = None;
                        } else {
                            // One-line toast: panel fill, hairline border. Errors keep the toast colour.
                            let r = egui::Frame::new()
                                .fill(tk().panel_bg)
                                .stroke(Stroke::new(1.0, tk().field_border))
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 2))
                                .show(ui, |ui| {
                                    ui.add(egui::Label::new(egui::RichText::new(&t.text).size(FS_MD).color(tk().text)).truncate())
                                })
                                .response
                                .interact(Sense::click());
                            if r.clicked() {
                                self.toast = None;
                            }
                            ctx.request_repaint_after(Duration::from_millis(500));
                            return;
                        }
                    }
                    if let Some(job) = &self.job {
                        let cur = job.progress.current.lock().map(|c| c.clone()).unwrap_or_default();
                        ui.add(
                            egui::Label::new(egui::RichText::new(format!("Scanning {}", tilde(&cur))).size(FS_MD).color(tk().text_dim))
                                .truncate(),
                        );
                    } else if let (Some(t), Some(s)) = (&self.tree, self.selected) {
                        let n = t.node(s);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("{} · {}", tilde(&t.path(s)), fmt_size(n.size))).size(FS_MD).color(tk().text_dim),
                            )
                            .truncate(),
                        );
                    }
                });
            });
        });
        if let Some(a) = act {
            self.apply(&ctx, a);
        }
    }

    // ── Empty and scanning states ───────────────────────────────────────────

    fn placeholder(&mut self, ui: &mut Ui, rect: Rect) {
        let resp = ui.interact(rect, Id::new("viewer.empty"), Sense::click());
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::ZERO, tk().inset);
        if let Some(job) = &self.job {
            let pr = &job.progress;
            let c = rect.center();
            p.text(c - vec2(0.0, 24.0), Align2::CENTER_CENTER, format!("Scanning {}", tilde(&job.root)), medium(13.0), tk().text);
            p.text(
                c,
                Align2::CENTER_CENTER,
                format!(
                    "{} files   {} folders   {}",
                    fmt_count(pr.files.load(Ordering::Relaxed)),
                    fmt_count(pr.dirs.load(Ordering::Relaxed)),
                    fmt_size(pr.bytes.load(Ordering::Relaxed))
                ),
                mono(FS_LG),
                tk().text_dim,
            );
            let cur = pr.current.lock().map(|c| c.clone()).unwrap_or_default();
            let cur = tilde(&cur);
            let max_chars = ((rect.width() - 40.0) / 6.5).max(10.0) as usize;
            let cur = if cur.chars().count() > max_chars {
                format!("…{}", cur.chars().rev().take(max_chars).collect::<Vec<_>>().into_iter().rev().collect::<String>())
            } else {
                cur
            };
            p.text(c + vec2(0.0, 20.0), Align2::CENTER_CENTER, cur, ui_font(FS_MD), tk().text_faint);
            // Thin amber bar while working. The total isn't known ahead, so it doesn't move;
            // the counters above do.
            let bar = Rect::from_min_size(pos2(rect.min.x, rect.min.y), vec2(rect.width(), 2.0));
            p.rect_filled(bar, CornerRadius::ZERO, tk().cache_busy);
        } else {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Choose a folder to scan. Type a path above and press Return, or press ⌘O.",
                ui_font(FS_LG),
                tk().text_faint,
            );
        }
        let mut act = None;
        resp.context_menu(|ui| {
            if let Some(a) = self.main_menu(ui) {
                act = Some(a);
            }
        });
        if let Some(a) = act {
            let ctx = ui.ctx().clone();
            self.apply(&ctx, a);
        }
    }

    // ── Treemap ─────────────────────────────────────────────────────────────

    fn treemap_view(&mut self, ui: &mut Ui, tree: &Arc<Tree>, rect: Rect) {
        let ctx = ui.ctx().clone();
        let resp = ui.interact(rect, Id::new("treemap.canvas"), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, tk().pasteboard);
        let area = rect.shrink(4.0);

        let q = self.search.trim().to_lowercase();
        let key = (
            self.generation.wrapping_mul(1_000_003).wrapping_add(self.scan_rev),
            self.root,
            [area.min.x as i32, area.min.y as i32, area.max.x as i32, area.max.y as i32],
            q.clone(),
            self.hidden,
            self.expanded_gen,
        );
        if self.cells_key.as_ref() != Some(&key) {
            treemap::layout(tree, self.root, area, &self.expanded, &mut self.cells);
            self.dim = self
                .cells
                .iter()
                .map(|c| match c.item {
                    Item::Node(i) => {
                        let n = tree.node(i);
                        if n.is_dir && c.open {
                            false
                        } else {
                            self.hidden[n.cat as usize] || (!q.is_empty() && !contains_ci(&n.name, &q))
                        }
                    }
                    Item::Rest { .. } => !q.is_empty() || self.hidden.iter().any(|h| *h),
                })
                .collect();
            self.cells_key = Some(key);
        }

        let hover = resp.hover_pos().and_then(|p| treemap::hit(&self.cells, p));

        for (k, c) in self.cells.iter().enumerate() {
            draw_cell(&painter, tree, c, self.dim[k]);
        }
        if tree.node(self.root).size == 0 {
            painter.text(area.center(), Align2::CENTER_CENTER, "This folder is empty.", ui_font(FS_LG), tk().text_faint);
        }

        // Selection and hover outlines on top.
        if let Some(s) = self.selected
            && let Some(c) = self.cells.iter().find(|c| c.item == Item::Node(s))
        {
            painter.rect_stroke(c.rect, CornerRadius::ZERO, Stroke::new(2.0, tk().accent), StrokeKind::Inside);
        }
        if let Some(h) = hover {
            let c = self.cells[h];
            painter.rect_stroke(c.rect, CornerRadius::ZERO, Stroke::new(1.0, tk().icon_active), StrokeKind::Inside);
        }

        let hovered_item = hover.map(|h| self.cells[h].item);
        let mut acts: Vec<Action> = Vec::new();
        let hovered_cell = hover.map(|h| self.cells[h]);
        if let Some(c) = hovered_cell
            && let Item::Node(i) = c.item
            && tree.node(i).is_dir
            && tree.node(i).n_children > 0
            && (!c.open || resp.hover_pos().is_some_and(|p| p.y < c.rect.min.y + HEADER_H))
        {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !resp.double_clicked() {
            match hovered_cell {
                Some(c) => match c.item {
                    Item::Node(i) => {
                        acts.push(Action::Select(i));
                        let n = tree.node(i);
                        if n.is_dir && n.n_children > 0 {
                            let on_header = resp.hover_pos().is_some_and(|p| p.y < c.rect.min.y + HEADER_H);
                            if c.open {
                                // Clicking an open folder's title bar closes it.
                                if on_header {
                                    acts.push(Action::Toggle(i));
                                }
                            } else if treemap::can_open(c.rect) {
                                acts.push(Action::Toggle(i));
                            } else {
                                // Too small to open in place: go into it instead.
                                acts.push(Action::ZoomTo(i));
                            }
                        }
                    }
                    Item::Rest { parent, .. } => acts.push(Action::Select(parent)),
                },
                None => self.selected = None,
            }
        }
        if resp.double_clicked() {
            match hovered_item {
                Some(Item::Node(i)) => {
                    let n = tree.node(i);
                    let target = if n.is_dir { i } else { n.parent };
                    if target != NONE {
                        acts.push(Action::ZoomTo(target));
                    }
                }
                Some(Item::Rest { parent, .. }) => acts.push(Action::ZoomTo(parent)),
                None => {}
            }
        }
        if resp.secondary_clicked() {
            self.menu_target = hovered_item;
            if let Some(Item::Node(i)) = hovered_item {
                self.selected = Some(i);
            }
        }
        if !resp.context_menu_opened()
            && let Some(item) = hovered_item
        {
            let resp = resp.clone().on_hover_ui_at_pointer(|ui| tooltip(ui, tree, item));
            let _ = resp;
        }
        let target = self.menu_target;
        let root = self.root;
        let open_state = match target {
            Some(Item::Node(i)) if tree.node(i).is_dir => Some(self.cells.iter().any(|c| c.item == Item::Node(i) && c.open)),
            _ => None,
        };
        let any_open = !self.expanded.is_empty();
        resp.context_menu(|ui| {
            if let Some(a) = item_menu(ui, tree, target, root, open_state, any_open) {
                acts.push(a);
            }
        });
        for a in acts {
            self.apply(&ctx, a);
        }
    }

    // ── List ────────────────────────────────────────────────────────────────

    fn list_rows(&mut self, tree: &Tree) -> (Vec<u32>, Option<usize>) {
        if self.searching() {
            return match &self.search_result {
                Some(r) if r.key.1 == self.root && r.key.2 == self.generation => (r.items.clone(), Some(r.total)),
                _ => (Vec::new(), None),
            };
        }
        let key = (self.generation.wrapping_mul(1_000_003).wrapping_add(self.scan_rev), self.root, self.sort.0, self.sort.1);
        if self.list_key != Some(key) {
            let mut v: Vec<u32> = tree.children(self.root).collect();
            sort_nodes(tree, &mut v, self.sort);
            self.list = v;
            self.list_key = Some(key);
        }
        (self.list.clone(), None)
    }

    fn list_view(&mut self, ui: &mut Ui, tree: &Arc<Tree>, rect: Rect) {
        let ctx = ui.ctx().clone();
        let searching = self.searching();
        let (mut rows, total) = self.list_rows(tree);
        if searching && self.sort != (SortCol::Size, true) {
            sort_nodes(tree, &mut rows, self.sort);
        }
        let max = rows.iter().map(|&i| tree.node(i).size).max().unwrap_or(1).max(1);

        ui.painter().rect_filled(rect, CornerRadius::ZERO, tk().panel_bg);
        // Right-click on the list's background for the general commands; rows sit on top.
        let mut bg_act = None;
        ui.interact(rect, Id::new("list.background"), Sense::click()).context_menu(|ui| {
            bg_act = self.main_menu(ui);
        });
        if let Some(a) = bg_act {
            self.apply(&ctx, a);
        }
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Min)));
        let ui = &mut ui;
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

        // Column layout.
        let w = rect.width();
        let col_size = 150.0;
        let col_items = 84.0;
        let col_mod = 124.0;
        let col_name = if searching { (w - col_size - col_items - col_mod) * 0.42 } else { w - col_size - col_items - col_mod };
        let cols = [
            (SortCol::Name, "Name", col_name),
            (SortCol::Size, "Size", col_size),
            (SortCol::Items, "Items", col_items),
            (SortCol::Modified, "Modified", col_mod),
        ];

        // Header.
        let (hr, _) = ui.allocate_exact_size(vec2(w, TAB_H), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(hr, CornerRadius::ZERO, tk().panel_header);
        let mut x = hr.min.x;
        for (col, label, cw) in cols {
            let r = Rect::from_min_size(pos2(x, hr.min.y), vec2(cw, hr.height()));
            let resp = ui.interact(r, Id::new(("list.header", label)), Sense::click());
            if resp.clicked() {
                self.sort = if self.sort.0 == col { (col, !self.sort.1) } else { (col, col != SortCol::Name) };
            }
            let active = self.sort.0 == col;
            let arrow = if active { if self.sort.1 { " ▾" } else { " ▴" } } else { "" };
            let col_txt = if active || resp.hovered() { tk().tab_text_active } else { tk().tab_text };
            p.text(pos2(r.min.x + 8.0, r.center().y), Align2::LEFT_CENTER, format!("{label}{arrow}"), medium(FS_MD), col_txt);
            if x > hr.min.x {
                p.vline(x, r.y_range().shrink(4.0), Stroke::new(1.0, tk().separator));
            }
            x += cw;
        }
        if searching {
            p.vline(x, hr.y_range().shrink(4.0), Stroke::new(1.0, tk().separator));
            p.text(pos2(x + 8.0, hr.center().y), Align2::LEFT_CENTER, "Folder", medium(FS_MD), tk().tab_text);
            if let Some(total) = total {
                let note = if total > rows.len() {
                    format!("Largest {} of {} matches", fmt_count(rows.len() as u64), fmt_count(total as u64))
                } else {
                    format!("{} matches", fmt_count(total as u64))
                };
                p.text(pos2(hr.max.x - 10.0, hr.center().y), Align2::RIGHT_CENTER, note, ui_font(FS_MD), tk().text_faint);
            }
        }
        p.hline(hr.x_range(), hr.max.y - 0.5, Stroke::new(1.0, tk().separator));

        if rows.is_empty() {
            let msg = if searching {
                if self.search_job.is_some() { "Searching…" } else { "No names match." }
            } else {
                "This folder is empty."
            };
            let r = ui.available_rect_before_wrap();
            ui.painter().text(pos2(r.min.x + 10.0, r.min.y + 14.0), Align2::LEFT_CENTER, msg, ui_font(FS_LG), tk().text_faint);
            return;
        }

        let mut acts = Vec::new();
        let root_path = tree.path(self.root);
        egui::ScrollArea::vertical().id_salt("list.scroll").auto_shrink([false, false]).show_rows(
            ui,
            ROW_H,
            rows.len(),
            |ui, range| {
                for k in range {
                    let i = rows[k];
                    let n = tree.node(i);
                    let (r, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                    let resp = ui.interact(r, Id::new(("list.row", tree.path(i))), Sense::click());
                    let selected = self.selected == Some(i);
                    let bg = if selected {
                        tk().row_selected
                    } else if resp.hovered() {
                        tk().hover
                    } else if k % 2 == 1 {
                        tk().row_alt
                    } else {
                        tk().row
                    };
                    let p = ui.painter();
                    p.rect_filled(r, CornerRadius::ZERO, bg);
                    let dimmed = self.hidden[n.cat as usize];
                    let fg = if dimmed { tk().disabled_fg } else { tk().text };

                    // Name with a swatch.
                    let mut x = r.min.x + 8.0;
                    let sw = Rect::from_center_size(pos2(x + 5.0, r.center().y), vec2(10.0, 10.0));
                    if n.is_dir {
                        p.rect_filled(sw, CornerRadius::same(1), folder_colors(n.cat).0);
                    } else {
                        p.rect_filled(sw, CornerRadius::same(1), n.cat.color());
                    }
                    x += 16.0;
                    let name_clip = Rect::from_min_max(pos2(x, r.min.y), pos2(r.min.x + col_name - 6.0, r.max.y));
                    let label = if n.is_dir { format!("{}/", n.name) } else { n.name.to_string() };
                    let fg_name = if selected {
                        // The selected name sits in its own cell.
                        let g = p.layout_no_wrap(label.clone(), ui_font(FS_LG), fg);
                        let cell = Rect::from_min_size(pos2(x - 3.0, r.min.y + 1.0), vec2(g.size().x + 6.0, r.height() - 2.0))
                            .intersect(name_clip.expand2(vec2(3.0, 0.0)));
                        p.rect_filled(cell, CornerRadius::same(2), tk().selected_name_bg);
                        tk().selected_name_fg
                    } else {
                        fg
                    };
                    p.with_clip_rect(name_clip).text(pos2(x, r.center().y), Align2::LEFT_CENTER, label, ui_font(FS_LG), fg_name);

                    // Size with a bar.
                    let sx = r.min.x + col_name;
                    let bar_w = (col_size - 80.0) * (n.size as f32 / max as f32);
                    let bar = Rect::from_min_size(pos2(sx + 72.0, r.center().y - 2.0), vec2(bar_w.max(1.0), 4.0));
                    p.rect_filled(bar, CornerRadius::same(1), tk().text_faint.gamma_multiply(0.6));
                    p.text(pos2(sx + 64.0, r.center().y), Align2::RIGHT_CENTER, fmt_size(n.size), mono(FS_MD), fg);

                    let ix = sx + col_size;
                    if n.is_dir {
                        p.text(pos2(ix + col_items - 8.0, r.center().y), Align2::RIGHT_CENTER, fmt_count(n.files), mono(FS_MD), tk().text_dim);
                    }
                    let mx = ix + col_items;
                    p.text(pos2(mx + 8.0, r.center().y), Align2::LEFT_CENTER, fmt_time(n.mtime), mono(FS_MD), tk().text_dim);
                    if searching {
                        let fx = mx + col_mod;
                        let parent = tree.path(n.parent);
                        let rel = parent.strip_prefix(&root_path).map(|s| format!(".{s}")).unwrap_or(parent);
                        let clip = Rect::from_min_max(pos2(fx, r.min.y), pos2(r.max.x - 6.0, r.max.y));
                        p.with_clip_rect(clip).text(pos2(fx + 8.0, r.center().y), Align2::LEFT_CENTER, rel, ui_font(FS_MD), tk().text_dim);
                    }
                    p.hline(r.x_range(), r.max.y - 0.5, Stroke::new(1.0, tk().row_line));

                    if resp.clicked() {
                        acts.push(Action::Select(i));
                    }
                    if resp.double_clicked() {
                        acts.push(if n.is_dir { Action::ZoomTo(i) } else { Action::Open(i) });
                    }
                    if resp.secondary_clicked() {
                        acts.push(Action::Select(i));
                    }
                    let root = self.root;
                    resp.context_menu(|ui| {
                        if let Some(a) = item_menu(ui, tree, Some(Item::Node(i)), root, None, false) {
                            acts.push(a);
                        }
                    });
                }
            },
        );
        for a in acts {
            self.apply(&ctx, a);
        }
    }

    // ── Properties ──────────────────────────────────────────────────────────

    fn properties(&mut self, ctx: &egui::Context) {
        let Some(tree) = self.tree.clone() else { return };
        let Some(props) = &self.props else { return };
        let i = props.node;
        let top = props.top.clone();
        let n = tree.node(i);
        let path = tree.path(i);
        let mut open = true;
        let mut acts = Vec::new();
        egui::Window::new(egui::RichText::new(&*n.name).font(semibold(13.0)))
            .id(Id::new("props.window"))
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("props.grid").num_columns(2).spacing(vec2(14.0, 4.0)).show(ui, |ui| {
                    let row = |ui: &mut Ui, k: &str, v: String| {
                        ui.label(egui::RichText::new(k).color(tk().text_dim));
                        ui.label(egui::RichText::new(v).font(mono(FS_MD)));
                        ui.end_row();
                    };
                    ui.label(egui::RichText::new("Path").color(tk().text_dim));
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(tilde(&path)).font(mono(FS_MD))).wrap());
                        if button(ui, "props.copy_path", "Copy", false, true).clicked() {
                            acts.push(Action::CopyPath(i));
                        }
                    });
                    ui.end_row();
                    row(ui, "Size on disk", format!("{} ({} bytes)", fmt_size(n.size), fmt_count(n.size)));
                    row(ui, "Kind", if n.is_dir { "Folder".into() } else { n.cat.name().into() });
                    row(ui, "Modified", fmt_time(n.mtime));
                    if n.is_dir {
                        row(ui, "Contains", format!("{} files, {} folders", fmt_count(n.files), fmt_count(n.dirs as u64)));
                        if let Some(c) = tree.children(i).next() {
                            let c = tree.node(c);
                            row(ui, "Largest item", format!("{}  {}", c.name, fmt_size(c.size)));
                        }
                        if n.unreadable {
                            ui.label(egui::RichText::new("Note").color(tk().text_dim));
                            ui.label(egui::RichText::new("This folder could not be read.").color(tk().warning));
                            ui.end_row();
                        }
                    }
                    if n.parent != NONE {
                        row(ui, "Share of parent", {
                            let ps = tree.node(n.parent).size.max(1);
                            format!("{:.1}%", n.size as f64 * 100.0 / ps as f64)
                        });
                    }
                });
                if n.is_dir && !top.is_empty() {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Largest files").font(medium(FS_MD)).color(tk().text_dim));
                    for t in &top {
                        let tn = tree.node(*t);
                        let full = tree.path(*t);
                        let rel = full.strip_prefix(&path).unwrap_or(&full).trim_start_matches('/').to_owned();
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(fmt_size(tn.size)).font(mono(FS_MD)).color(tk().text_dim));
                            let r = ui.add(egui::Label::new(egui::RichText::new(rel).size(FS_MD)).truncate().sense(Sense::click()));
                            if r.clicked() {
                                acts.push(Action::Reveal(*t));
                            }
                            r.on_hover_text("Reveal in Finder");
                        });
                    }
                }
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if button(ui, "props.reveal", "Reveal in Finder", false, true).clicked() {
                        acts.push(Action::Reveal(i));
                    }
                });
            });
        if !open {
            self.props = None;
        }
        for a in acts {
            self.apply(ctx, a);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_scan(&ctx);
        self.poll_live(&ctx);
        self.poll_search(&ctx);
        self.shortcuts(&ctx);

        let frame = |fill| egui::Frame::new().fill(fill).inner_margin(egui::Margin::ZERO);
        egui::Panel::top("app.bar").exact_size(APP_BAR_H).frame(frame(tk().header_bg)).show(ui, |ui| self.app_bar(ui));
        egui::Panel::top("app.crumbs").exact_size(TAB_H + GAP).frame(frame(tk().app_bg)).show(ui, |ui| {
            let r = ui.max_rect();
            let band = Rect::from_min_max(pos2(r.min.x, r.min.y + GAP), r.max);
            ui.painter().rect_filled(band, CornerRadius::ZERO, tk().panel_header);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(band).layout(Layout::left_to_right(Align::Center)));
            self.crumb_bar(&mut child);
        });
        egui::Panel::bottom("app.status").exact_size(30.0 + GAP).frame(frame(tk().app_bg)).show(ui, |ui| {
            let r = ui.max_rect();
            let band = Rect::from_min_max(r.min, pos2(r.max.x, r.max.y - 0.0));
            let band = Rect::from_min_max(pos2(band.min.x, band.min.y + GAP), band.max);
            ui.painter().rect_filled(band, CornerRadius::ZERO, tk().header_bg);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(band).layout(Layout::left_to_right(Align::Center)));
            self.status_bar(&mut child);
        });
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let full = ui.max_rect();
            ui.painter().rect_filled(full, CornerRadius::ZERO, tk().app_bg);
            let rect = Rect::from_min_max(pos2(full.min.x, full.min.y + GAP), full.max);
            match (true, self.tree.clone()) {
                (true, Some(tree)) => match self.view {
                    View::Treemap => self.treemap_view(ui, &tree, rect),
                    View::List => self.list_view(ui, &tree, rect),
                },
                _ => self.placeholder(ui, rect),
            }
        });
        self.properties(&ctx);
    }
}

// ── Keeping places across tree edits ───────────────────────────────────────

/// Where the user is, by path, so it survives edits that move node indices.
struct Places {
    root: String,
    selected: Option<String>,
    props: Option<String>,
    open: Vec<String>,
    /// What an open right-click menu points at; small-item boxes keep their counts.
    menu: Option<(String, Option<(u32, u64)>)>,
}

fn remember(app: &App, tree: &Tree) -> Places {
    Places {
        root: tree.path(app.root),
        selected: app.selected.map(|s| tree.path(s)),
        props: app.props.as_ref().map(|p| tree.path(p.node)),
        open: app.expanded.iter().map(|&e| tree.path(e)).collect(),
        menu: app.menu_target.map(|m| match m {
            Item::Node(i) => (tree.path(i), None),
            Item::Rest { parent, count, size } => (tree.path(parent), Some((count, size))),
        }),
    }
}

fn restore(app: &mut App, tree: &Tree, p: Places) {
    let root = tree.nearest(&p.root);
    app.root = if tree.node(root).is_dir { root } else { tree.node(root).parent };
    app.selected = p.selected.and_then(|s| tree.find(&s));
    app.props = p.props.and_then(|s| tree.find(&s)).map(|n| Props { node: n, top: tree.top_files(n, 5) });
    app.expanded = p.open.iter().filter_map(|s| tree.find(s)).collect();
    app.expanded_gen += 1;
    app.menu_target = p.menu.and_then(|(path, rest)| {
        let i = tree.find(&path)?;
        Some(match rest {
            None => Item::Node(i),
            Some((count, size)) => Item::Rest { parent: i, count, size },
        })
    });
}

// ── Drawing helpers ─────────────────────────────────────────────────────────

/// Colours for a folder, from the file type that fills most of it: (body, title bar, open body).
fn folder_colors(cat: Cat) -> (Color32, Color32, Color32) {
    let c = cat.color();
    (
        c.lerp_to_gamma(tk().row, 0.30),
        c.lerp_to_gamma(tk().panel_header, 0.62),
        c.lerp_to_gamma(tk().inset, 0.84),
    )
}

/// Inner padding of treemap labels.
const CELL_PAD: f32 = 5.0;

/// `text` shortened with "…" to fit `max_w`, or None when not even two letters fit.
fn elide(p: &egui::Painter, text: &str, font: &FontId, max_w: f32) -> Option<String> {
    if max_w < 12.0 {
        return None;
    }
    let width = |s: &str| p.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE).size().x;
    if width(text) <= max_w {
        return Some(text.to_owned());
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().collect::<String>() + "…";
        if width(&s) <= max_w { lo = mid } else { hi = mid - 1 }
    }
    (lo >= 2).then(|| chars[..lo].iter().collect::<String>() + "…")
}

/// Twirl: a small triangle, right when closed, down when open.
fn twirl(p: &egui::Painter, c: egui::Pos2, open: bool, color: Color32) {
    let pts = if open {
        vec![c + vec2(-3.5, -2.0), c + vec2(3.5, -2.0), c + vec2(0.0, 2.5)]
    } else {
        vec![c + vec2(-2.0, -3.5), c + vec2(2.5, 0.0), c + vec2(-2.0, 3.5)]
    };
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// A folder's title bar: twirl, name, and size on the right when there's room.
fn header(p: &egui::Painter, head: Rect, name: &str, size: u64, open: bool, fg: Color32, fg2: Color32) {
    let clip = p.with_clip_rect(head.intersect(p.clip_rect()));
    let cy = head.center().y;
    if head.width() < 18.0 {
        return;
    }
    twirl(&clip, pos2(head.min.x + 9.0, cy), open, fg2);
    let name_x = head.min.x + 17.0;
    let right = head.max.x - CELL_PAD;
    let name_font = semibold(FS_SM);
    let size_font = ui_font(FS_SM);
    let size_txt = fmt_size(size);
    let size_w = p.layout_no_wrap(size_txt.clone(), size_font.clone(), fg2).size().x;
    // The size shows only when the name still gets a fair share.
    let show_size = right - name_x - size_w - 10.0 >= 48.0;
    let name_max = if show_size { right - name_x - size_w - 10.0 } else { right - name_x };
    if let Some(n) = elide(p, name, &name_font, name_max) {
        clip.text(pos2(name_x, cy), Align2::LEFT_CENTER, n, name_font, fg);
    }
    if show_size {
        clip.text(pos2(right, cy), Align2::RIGHT_CENTER, size_txt, size_font, fg2);
    }
}

/// Name and size at the top-left of a cell, each shortened to fit.
fn corner_label(p: &egui::Painter, r: Rect, name: &str, size: Option<u64>, fg: Color32, fg2: Color32) {
    if r.width() < 28.0 || r.height() < 16.0 {
        return;
    }
    let clip = p.with_clip_rect(r.intersect(p.clip_rect()));
    let max_w = r.width() - 2.0 * CELL_PAD;
    let font = ui_font(FS_SM);
    if let Some(n) = elide(p, name, &font, max_w) {
        clip.text(r.min + vec2(CELL_PAD, 4.0), Align2::LEFT_TOP, n, font.clone(), fg);
    }
    if let Some(s) = size
        && r.height() >= 32.0
        && let Some(t) = elide(p, &fmt_size(s), &font, max_w)
    {
        clip.text(r.min + vec2(CELL_PAD, 18.0), Align2::LEFT_TOP, t, font, fg2);
    }
}

/// Size and contents of a closed folder, centred in its body, in the first form that fits.
fn centre_label(p: &egui::Painter, body: Rect, n: &crate::scan::Node, fg: Color32, fg2: Color32) {
    if body.height() < 36.0 {
        return;
    }
    let max_w = body.width() - 2.0 * CELL_PAD;
    let big = body.width() > 220.0 && body.height() > 90.0;
    let size_font = if big { semibold(16.0) } else { semibold(12.5) };
    let sub_font = ui_font(FS_MD);
    let size_txt = fmt_size(n.size);
    if p.layout_no_wrap(size_txt.clone(), size_font.clone(), fg).size().x > max_w {
        return;
    }
    let forms = [
        (n.dirs > 0).then(|| format!("{} files in {} folders", fmt_count(n.files), fmt_count(n.dirs as u64))),
        Some(format!("{} files", fmt_count(n.files))),
    ];
    let sub = forms
        .into_iter()
        .flatten()
        .find(|t| p.layout_no_wrap(t.clone(), sub_font.clone(), fg2).size().x <= max_w);
    let gap = if big { 11.0 } else { 9.0 };
    let clip = p.with_clip_rect(body.intersect(p.clip_rect()));
    match sub {
        Some(t) if body.height() >= 52.0 => {
            clip.text(body.center() - vec2(0.0, gap), Align2::CENTER_CENTER, size_txt, size_font, fg);
            clip.text(body.center() + vec2(0.0, gap + 1.0), Align2::CENTER_CENTER, t, sub_font, fg2);
        }
        _ => {
            clip.text(body.center(), Align2::CENTER_CENTER, size_txt, size_font, fg);
        }
    }
}

fn draw_cell(p: &egui::Painter, tree: &Tree, c: &Cell, dim: bool) {
    // A 1 px gap between cells instead of borders.
    let r = c.rect.shrink(0.5);
    if r.width() < 0.5 || r.height() < 0.5 {
        return;
    }
    // Dark ink on the coloured fills reads in every theme.
    let ink = Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 225);
    let ink2 = Color32::from_rgba_unmultiplied(0x10, 0x10, 0x10, 150);
    match c.item {
        Item::Node(i) => {
            let n = tree.node(i);
            if n.is_dir {
                let openable = n.n_children > 0 && treemap::can_open(c.rect);
                let (body_fill, bar_fill, open_fill) = folder_colors(n.cat);
                let fade = |c: Color32| if dim { c.gamma_multiply(0.2) } else { c };
                p.rect_filled(r, CornerRadius::ZERO, fade(if c.open { open_fill } else { body_fill }));
                let (fg, fg2) = if dim { (tk().text_faint, tk().text_faint) } else { (tk().text, tk().text_dim) };
                if c.open || openable {
                    let head = Rect::from_min_size(r.min, vec2(r.width(), HEADER_H));
                    p.rect_filled(head, CornerRadius::ZERO, fade(bar_fill));
                    header(p, head, &n.name, n.size, c.open, fg, fg2);
                    if !c.open && !dim {
                        centre_label(p, Rect::from_min_max(pos2(r.min.x, head.max.y), r.max), n, ink, ink2);
                    }
                } else if !dim {
                    corner_label(p, r, &n.name, Some(n.size), ink, ink2);
                }
            } else {
                let base = n.cat.color().lerp_to_gamma(tk().row, 0.22);
                p.rect_filled(r, CornerRadius::ZERO, if dim { base.gamma_multiply(0.15) } else { base });
                if !dim {
                    corner_label(p, r, &n.name, Some(n.size), ink, ink2);
                }
            }
        }
        Item::Rest { parent, count, size } => {
            // Leftovers take a quiet tint of the folder they belong to.
            let fill = tree.node(parent).cat.color().lerp_to_gamma(tk().row, 0.72);
            p.rect_filled(r, CornerRadius::ZERO, if dim { fill.gamma_multiply(0.35) } else { fill });
            corner_label(p, r, &format!("{} small items", fmt_count(count as u64)), Some(size), tk().text_dim, tk().text_faint);
        }
    }
}

fn tooltip(ui: &mut Ui, tree: &Tree, item: Item) {
    ui.set_max_width(420.0);
    match item {
        Item::Node(i) => {
            let n = tree.node(i);
            ui.label(egui::RichText::new(&*n.name).font(medium(FS_LG)).color(tk().tab_text_active));
            let pct = if n.parent != NONE {
                format!("   {:.1}% of folder", n.size as f64 * 100.0 / tree.node(n.parent).size.max(1) as f64)
            } else {
                String::new()
            };
            ui.label(egui::RichText::new(format!("{}{}", fmt_size(n.size), pct)).font(mono(FS_MD)));
            if n.is_dir {
                ui.label(
                    egui::RichText::new(format!("{} files, {} folders", fmt_count(n.files), fmt_count(n.dirs as u64)))
                        .color(tk().text_dim)
                        .size(FS_MD),
                );
            } else {
                ui.label(egui::RichText::new(n.cat.name()).color(tk().text_dim).size(FS_MD));
            }
            let m = fmt_time(n.mtime);
            if !m.is_empty() {
                ui.label(egui::RichText::new(format!("Modified {m}")).color(tk().text_dim).size(FS_MD));
            }
            ui.label(egui::RichText::new(tilde(&tree.path(i))).color(tk().text_faint).size(FS_SM));
        }
        Item::Rest { parent, count, size } => {
            ui.label(egui::RichText::new(format!("{} small items", fmt_count(count as u64))).font(medium(FS_LG)));
            ui.label(egui::RichText::new(fmt_size(size)).font(mono(FS_MD)));
            ui.label(egui::RichText::new(format!("in {}. Double-click to zoom in.", tree.node(parent).name)).color(tk().text_dim).size(FS_MD));
        }
    }
}

fn menu_item(ui: &mut Ui, label: &str, shortcut: &str) -> bool {
    menu_item_on(ui, label, shortcut, true)
}

/// Menu item that can be disabled (drawn in disabled-fg, no hover).
fn menu_item_on(ui: &mut Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let b = egui::Button::new(label).shortcut_text(egui::RichText::new(shortcut).color(tk().text_faint));
    let clicked = ui.add_enabled(enabled, b).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Menu item for a choice that is on or off; the current one sits in the selection fill.
fn check_item(ui: &mut Ui, label: &str, shortcut: &str, on: bool) -> bool {
    let b = egui::Button::new(label).selected(on).shortcut_text(egui::RichText::new(shortcut).color(tk().text_faint));
    let clicked = ui.add(b).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// `open`: whether a treemap folder is open in place (None outside the treemap).
fn item_menu(ui: &mut Ui, tree: &Tree, target: Option<Item>, root: u32, open: Option<bool>, any_open: bool) -> Option<Action> {
    ui.set_min_width(200.0);
    let mut a = None;
    match target {
        Some(Item::Node(i)) => {
            let n = tree.node(i);
            if n.is_dir {
                if let Some(o) = open
                    && n.n_children > 0
                    && menu_item(ui, if o { "Close Folder" } else { "Open Folder" }, "Click")
                {
                    a = Some(Action::Toggle(i));
                }
                if menu_item(ui, "Zoom In", "↩") {
                    a = Some(Action::ZoomTo(i));
                }
                if menu_item(ui, "Reveal in Finder", "") {
                    a = Some(Action::Reveal(i));
                }
                if menu_item(ui, "Open in Terminal", "") {
                    a = Some(Action::Terminal(i));
                }
                if menu_item(ui, "Scan This Folder", "") {
                    a = Some(Action::ScanFolder(i));
                }
            } else {
                if menu_item(ui, "Open", "") {
                    a = Some(Action::Open(i));
                }
                if menu_item(ui, "Reveal in Finder", "") {
                    a = Some(Action::Reveal(i));
                }
            }
            ui.separator();
            if menu_item(ui, "Copy Path", "") {
                a = Some(Action::CopyPath(i));
            }
            if menu_item(ui, "Copy Size", "") {
                a = Some(Action::CopySize(i));
            }
            ui.separator();
            if menu_item(ui, "Properties…", "⌘I") {
                a = Some(Action::Props(i));
            }
        }
        Some(Item::Rest { parent, .. }) => {
            if menu_item(ui, "Zoom In", "") {
                a = Some(Action::ZoomTo(parent));
            }
        }
        None => {}
    }
    if target.is_some() {
        ui.separator();
    }
    if any_open && menu_item(ui, "Close All Folders", "") {
        a = Some(Action::CloseAll);
    }
    if root != 0 && menu_item(ui, "Up One Level", "⌫") {
        a = Some(Action::Up);
    }
    if menu_item(ui, "Rescan", "⌘R") {
        a = Some(Action::Rescan);
    }
    if menu_item(ui, "Scan Folder…", "⌘O") {
        a = Some(Action::Browse);
    }
    a
}

fn sort_nodes(tree: &Tree, v: &mut [u32], (col, desc): (SortCol, bool)) {
    let n = |i: &u32| tree.node(*i);
    match col {
        SortCol::Size => v.sort_by_key(|i| n(i).size),
        SortCol::Name => v.sort_by_cached_key(|i| n(i).name.to_lowercase()),
        SortCol::Items => v.sort_by_key(|i| (n(i).files, n(i).size)),
        SortCol::Modified => v.sort_by_key(|i| n(i).mtime),
    }
    if desc {
        v.reverse();
    }
}

/// Case-insensitive substring test; `needle` is already lowercase.
fn contains_ci(hay: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        let h = hay.as_bytes();
        let nd = needle.as_bytes();
        if nd.len() > h.len() {
            return false;
        }
        h.windows(nd.len()).any(|w| w.iter().zip(nd).all(|(a, b)| a.to_ascii_lowercase() == *b))
    } else {
        hay.to_lowercase().contains(needle)
    }
}

/// All names under `root` containing `q`, largest first, capped at MAX_RESULTS.
fn search_tree(tree: &Tree, root: u32, q: &str, cancel: &AtomicBool) -> Option<(Vec<u32>, usize)> {
    let mut hits = Vec::new();
    let mut stack: Vec<u32> = tree.children(root).collect();
    let mut steps = 0u32;
    while let Some(i) = stack.pop() {
        steps += 1;
        if steps % 65536 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let n = tree.node(i);
        if contains_ci(&n.name, q) {
            hits.push(i);
        }
        if n.is_dir {
            stack.extend(tree.children(i));
        }
    }
    let total = hits.len();
    hits.sort_unstable_by(|a, b| tree.node(*b).size.cmp(&tree.node(*a).size));
    hits.truncate(MAX_RESULTS);
    Some((hits, total))
}

fn run(cmd: &str, args: &[&str], app: &mut App) {
    if let Err(e) = Command::new(cmd).args(args).spawn() {
        app.toast_err(format!("Can't run {cmd}: {e}."));
    }
}

fn theme_file() -> PathBuf {
    PathBuf::from(home_dir()).join("Library/Application Support/Spacious/theme.txt")
}

fn load_theme() -> ThemeKind {
    std::fs::read_to_string(theme_file()).ok().and_then(|s| ThemeKind::from_name(&s)).unwrap_or(ThemeKind::Dark)
}

fn save_theme(k: ThemeKind) {
    let f = theme_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, k.name());
}

fn recent_file() -> PathBuf {
    PathBuf::from(home_dir()).join("Library/Application Support/Spacious/recent.txt")
}

fn load_recent() -> Vec<String> {
    std::fs::read_to_string(recent_file())
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn save_recent(r: &[String]) {
    let f = recent_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, r.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci() {
        assert!(contains_ci("Hello.TXT", "o.t"));
        assert!(!contains_ci("abc", "abcd"));
        assert!(contains_ci("Ärger.md", "ärg"));
    }
}
