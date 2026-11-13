//! The browser: tabs, modes, commands, and the two text rows around the page.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as TButton, MouseEvent,
    MouseEventKind,
};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::{cursor, queue, terminal};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use zebkit::offscreen::{Modifiers, MouseButton, MouseEvent as ZMouse, WheelPhase};
use zebkit::page::{Dialog, DialogPolicy, DialogType};
use zebkit::{LoadEvent, View};

use crate::config::{Action, Binding, Config, Scope, Stroke};
use crate::edit::{self, Edited};
use crate::history::{self, Completion, History, Saver};
use crate::input::{self, Edit, TextInput};
use crate::keys;
use crate::render::{Hop, Layout, Renderer};

pub const HINTS_JS: &str = include_str!("hints.js");
pub const FIND_JS: &str = include_str!("find.js");
const EDITABLE_JS: &str = concat!(editable!(), "(document.activeElement)");
/// Defines `__zwebCaret` (caret.js) and starts a one-character selection; `false` if no text is visible.
pub const VISUAL_START_JS: &str = concat!(include_str!("caret.js"), ";__zwebCaret.start()");
/// The selection's text; the selection collapses.
pub const YANK_JS: &str = "__zwebCaret.text()";
/// Removes the Visual selection, so the next `v` does not start there.
const COLLAPSE_JS: &str = "window.__zwebCaret?.collapse()";
/// Largest OSC 52 payload a yank writes, in base64 bytes.
const YANK_MAX: usize = 100_000;
const HOME: &str = "https://duckduckgo.com/";
/// Cell rows per scroll step (j, k, the wheel).
const STEP_ROWS: f32 = 3.0;
/// Code keys of hints.js (`__zwebHopKeys`): hop's letters, or digits for
/// filtered hints. Index: `hint_filter as usize`.
const HINT_KEYS: [&str; 2] = ["", "1234567890"];

// ---- Messages: every callback queues one; `drain` handles them in order. ----

pub enum HintReply {
    Targets(Result<String, String>),
    /// The hint overlay for the typed letters (hints.js `show`).
    Image(String),
}

pub enum Msg {
    Term(Event),
    Load {
        tab: u64,
        event: LoadEvent,
        url: String,
    },
    Title {
        tab: u64,
        title: String,
    },
    Caret {
        tab: u64,
        on: bool,
    },
    Popup {
        tab: u64,
        rect: Option<(i32, i32, i32, i32)>,
    },
    WindowOpen {
        view: View,
    },
    Dialog {
        tab: u64,
        dialog: Dialog,
    },
    Hints {
        tab: u64,
        reply: HintReply,
    },
    /// Hints read ahead (hints.js `__zwebHopPrepare()`).
    Prepared {
        tab: u64,
        filter: bool,
        json: String,
    },
    Found {
        tab: u64,
        generation: u64,
        query: String,
        result: Result<bool, String>,
    },
    Focused {
        ok: bool,
    },
    /// The field's text for the external editor (edit.rs `READ_JS`).
    EditText {
        tab: u64,
        json: Result<String, String>,
    },
    EditDone {
        tab: u64,
        path: PathBuf,
        edited: Edited,
    },
    /// The edited text went back into the field, or the field was gone.
    EditWritten {
        path: PathBuf,
        ok: bool,
    },
    /// `__zwebCaret.start()` of Visual session `session` found text, or not.
    VisualStarted {
        session: u64,
        ok: bool,
    },
    Yanked {
        tab: u64,
        text: Option<String>,
    },
    Extents {
        tab: u64,
        json: Option<String>,
    },
    /// Read the page for hints ahead, if nothing happened since `gen`.
    Prepare {
        gen: u64,
    },
    Tick,
}

thread_local! {
    static QUEUE: RefCell<VecDeque<Msg>> = RefCell::default();
    static SCHEDULED: Cell<bool> = const { Cell::new(false) };
    pub static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Queues a message for the app (engine thread only).
pub fn send(msg: Msg) {
    QUEUE.with(|q| q.borrow_mut().push_back(msg));
    if !SCHEDULED.with(|s| s.replace(true)) {
        zebkit::after(Duration::ZERO, drain);
    }
}

fn drain() {
    while let Some(msg) = QUEUE.with(|q| q.borrow_mut().pop_front()) {
        APP.with(|a| {
            if let Some(app) = a.borrow_mut().as_mut() {
                app.handle(msg);
            }
        });
    }
    SCHEDULED.with(|s| s.set(false));
}

// ---- State ----

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Browse,
    Insert,
    Hint,
    Visual,
}

#[derive(Clone, PartialEq)]
enum PromptKind {
    Open,
    OpenTab,
    Find,
    Dialog(u64, DialogType),
}

struct Prompt {
    kind: PromptKind,
    label: String,
    input: TextInput,
    not_found: bool,
    keep_open: bool,
    hint: String,
    /// The Tab/Shift-Tab session, from the text typed before the first Tab.
    completion: Option<Completion>,
    /// The best history match for the typed text (Open prompts only).
    suggestion: Option<String>,
}

impl Prompt {
    fn new(kind: PromptKind, label: &str, text: &str) -> Self {
        Self {
            kind,
            label: label.into(),
            input: TextInput::new(text),
            not_found: false,
            keep_open: false,
            hint: String::new(),
            completion: None,
            suggestion: None,
        }
    }

    fn completes(&self) -> bool {
        matches!(self.kind, PromptKind::Open | PromptKind::OpenTab)
    }

    /// Replaces the input with the next (or previous) history match.
    fn complete(&mut self, history: &History, back: bool, now: u64) {
        if !self.completes() {
            return;
        }
        let text = &self.input.text;
        let completion = self
            .completion
            .get_or_insert_with(|| Completion::new(history, text, now));
        if let Some(url) = completion.step(back) {
            self.input = TextInput::new(url);
            self.suggestion = None;
        }
    }

    /// After an edit: a new completion session and a new best match.
    fn suggest(&mut self, history: &History, now: u64) {
        self.completion = None;
        self.suggestion = None;
        if self.completes() && !self.input.text.trim().is_empty() {
            self.suggestion = history
                .top(&self.input.text, now)
                .first()
                .map(|e| e.url.clone());
        }
    }

    /// The text, and the dim best match drawn after it.
    fn body(&self, room: usize) -> (String, String) {
        if room == 0 {
            return Default::default();
        }
        let (before, after) = self.input.text.split_at(self.input.cursor);
        let prefix = format!(" {}{before}█", self.label);
        if prefix
            .chars()
            .map(display_char)
            .map(|c| c.width().unwrap_or(0))
            .sum::<usize>()
            > room
        {
            return (clip_start(&prefix, room), String::new());
        }
        let hint = if self.not_found {
            "  not found"
        } else if self.input.text.is_empty() {
            &self.hint
        } else {
            ""
        };
        let text = clip(&format!("{prefix}{after}{hint}"), room);
        let dim = match self.suggestion.as_deref() {
            Some(url) if after.is_empty() => match url.strip_prefix(self.input.text.as_str()) {
                Some(rest) => clip(rest, room - text.width()),
                None => clip(&format!("  → {url}"), room - text.width()),
            },
            _ => String::new(),
        };
        (text, dim)
    }
}

struct Tab {
    id: u64,
    view: View,
    title: String,
    url: String,
    loading: bool,
    popup: bool,
    /// A title that came before its page's commit: applied to history on commit.
    pending_title: Option<String>,
    /// The page shown is the one loading (its load committed).
    committed: bool,
}

struct Hint {
    key: String,
    x: f32,
    y: f32,
    href: Option<String>,
    edit: bool,
    text: String,
}

/// The hints whose text contains `text` (any case) and whose code starts
/// with `typed`, by index.
fn filter_hints(hints: &[Hint], text: &str, typed: &str) -> Vec<usize> {
    let text = text.to_lowercase();
    (0..hints.len())
        .filter(|&i| {
            hints[i].key.starts_with(typed) && hints[i].text.to_lowercase().contains(&text)
        })
        .collect()
}

/// Hints read ahead for a tab: its id, hints.js's id, the overlay, the hints.
type Prepared = (u64, String, Hop, Vec<Hint>);

/// hints.js's result, a JSON string of JSON: its id, overlay and hints.
fn parse_hints(json: &str) -> (Option<String>, Option<Hop>, Vec<Hint>) {
    let mut v: serde_json::Value = serde_json::from_str::<String>(json)
        .ok()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let hop = parse_hop(&v);
    let list = if v.is_array() {
        v.take()
    } else {
        v["list"].take()
    };
    let hints = list
        .as_array()
        .map(|l| {
            l.iter()
                .map(|h| Hint {
                    key: h["k"].as_str().unwrap_or("").into(),
                    x: h["x"].as_f64().unwrap_or(0.0) as f32,
                    y: h["y"].as_f64().unwrap_or(0.0) as f32,
                    href: h["href"].as_str().map(Into::into),
                    edit: h["edit"].as_bool().unwrap_or(false),
                    text: h["t"].as_str().unwrap_or("").into(),
                })
                .collect()
        })
        .unwrap_or_default();
    (v["id"].as_str().map(String::from), hop, hints)
}

/// The overlay in a hints.js result: {png, cells, dim}.
fn parse_hop(v: &serde_json::Value) -> Option<Hop> {
    let png = v["png"].as_str()?.to_string();
    let num = |n: &serde_json::Value| n.as_f64().unwrap_or(0.0).max(0.0) as u32;
    let cells = v["cells"].as_array()?.iter().filter_map(|c| {
        let c = c.as_array().filter(|c| c.len() == 8)?;
        Some(std::array::from_fn(|i| num(&c[i])))
    });
    let mut dim = [255, 255, 255, 191];
    if let Some(d) = v["dim"].as_array().filter(|d| d.len() == 4) {
        dim = std::array::from_fn(|i| num(&d[i]).min(255) as u8);
    }
    Some(Hop {
        png,
        cells: cells.collect(),
        dim,
    })
}

pub struct Options {
    pub scale: Option<u32>,
    /// Images as flat placeholders of the same size (key I toggles).
    pub placeholders: bool,
    /// The terminal takes keyboard enhancement flags (pushed at start).
    pub enhanced: bool,
}

/// Images keep their boxes (the layout does not move) and still load (so
/// showing them is at once), but are painted one flat colour: flat tiles
/// are tiny to send and repeat, so the terminal reuses them.
const PLACEHOLDER_JS: &str = "(on=>{let s=document.getElementById('__zweb_img');\
    if(!on){s&&s.remove();return}if(s)return;s=document.createElement('style');s.id='__zweb_img';\
    s.textContent='img,video,canvas,picture,svg image{object-fit:none!important;object-position:-99999px -99999px!important;\
    background:#d9dce1!important;background-image:none!important}video,canvas{visibility:hidden!important}';\
    const add=()=>(document.head||document.documentElement).appendChild(s);\
    if(document.documentElement)add();else new MutationObserver((m,o)=>{if(document.documentElement){o.disconnect();add()}})\
    .observe(document,{childList:true})})";

pub struct App {
    tabs: Vec<Tab>,
    active: usize,
    active_id: Rc<Cell<u64>>,
    /// The id of the tab that was active before the current one.
    previous_id: Option<u64>,
    /// URLs of closed tabs, oldest first.
    closed: VecDeque<String>,
    next_id: u64,
    mode: Mode,
    prompt: Option<Prompt>,
    hints: Vec<Hint>,
    /// Hints read ahead, for each of HINT_KEYS.
    prepared: [Option<Prepared>; 2],
    hint_typed: String,
    /// Filtered hints (`g f`): digits are codes, other characters `hint_text`.
    hint_filter: bool,
    hint_text: String,
    /// Filtered hints: the overlay with every code, shown in part.
    hint_hop: Option<Hop>,
    /// The dim layer's colour of the hints shown.
    hint_dim: Option<[u8; 4]>,
    hint_new_tab: bool,
    /// Bumped by each `v`: a start reply counts only for the newest one.
    visual_session: u64,
    /// Hints were shown in this session: read them ahead from now on.
    hints_used: bool,
    find: String,
    find_generation: u64,
    message: String,
    help: bool,
    help_lines: Vec<String>,
    help_offset: usize,
    bindings: Config,
    pending_key: Option<Stroke>,
    zoom: f64,
    scale: u32,
    /// CSS pixels per cell row. The page is a whole number of rows tall and
    /// scrolls by whole rows, so moved rows are the same pixels, reused.
    row_css: f32,
    /// The active page's scrollY, innerHeight and scrollHeight, as last
    /// read and moved by predicted scrolls since.
    extents: Option<(f32, f32, f32)>,
    /// Bumped by every action; a hint read ahead waits for 200 ms of none.
    prep_gen: u64,
    opts: Options,
    renderer: Arc<Renderer>,
    last_click: (Instant, f32, f32, u32),
    mouse: (f32, f32),
    last_stats: u64,
    history: History,
    saver: Saver,
    /// A `g e` edit runs, from its read to its write-back: no second one.
    editing: bool,
}

impl App {
    pub fn new(renderer: Arc<Renderer>, opts: Options, bindings: Config) -> App {
        let message = format!(
            "{} or {} for help",
            bindings.labels(Scope::Browse, Action::Help),
            bindings.labels(Scope::Global, Action::Help)
        );
        let help_lines = bindings.help_lines();
        App {
            tabs: Vec::new(),
            active: 0,
            active_id: Rc::new(Cell::new(0)),
            previous_id: None,
            closed: VecDeque::new(),
            next_id: 1,
            mode: Mode::Browse,
            prompt: None,
            hints: Vec::new(),
            prepared: [None, None],
            hint_typed: String::new(),
            hint_filter: false,
            hint_text: String::new(),
            hint_hop: None,
            hint_dim: None,
            hint_new_tab: false,
            visual_session: 0,
            hints_used: false,
            find: String::new(),
            find_generation: 0,
            message,
            help: false,
            help_lines,
            help_offset: 0,
            bindings,
            pending_key: None,
            zoom: 1.0,
            scale: 1,
            row_css: 16.0,
            extents: None,
            prep_gen: 0,
            opts,
            renderer,
            last_click: (Instant::now() - Duration::from_secs(10), 0.0, 0.0, 0),
            mouse: (0.0, 0.0),
            last_stats: 0,
            history: load_history(),
            saver: Saver::spawn(crate::data_dir().join("history.tsv")),
            editing: false,
        }
    }

    // ---- Layout ----

    /// Recomputes the page's size from the terminal's and applies it to every tab.
    pub fn relayout(&mut self) {
        let (cols, rows) = terminal::size().unwrap_or((100, 30));
        let px = terminal::window_size().ok();
        let page_rows = rows.saturating_sub(2).max(1);
        let (mut cell_w, mut cell_h) = (8.0, 16.0);
        if let Some(px) = px.filter(|p| p.width > 0 && p.height > 0) {
            cell_w = px.width as f64 / cols as f64;
            cell_h = px.height as f64 / rows as f64;
        }
        // A retina terminal reports device pixels: count 2 per CSS pixel.
        let dpr = if cell_w >= 14.0 { 2.0 } else { 1.0 };
        let css_w = (cols as f64 * cell_w / (dpr * self.zoom)).round().max(64.0) as u32;
        let row_css = (cell_h / (dpr * self.zoom)).round().max(1.0);
        self.row_css = row_css as f32;
        let css_h = (page_rows as f64 * row_css) as u32;
        self.scale =
            self.opts
                .scale
                .unwrap_or(if self.renderer.gfx == crate::render::Gfx::KittyShm {
                    dpr as u32
                } else {
                    1
                });
        {
            let mut l = self.renderer.layout.lock().unwrap();
            *l = Layout {
                top: 1,
                cols,
                rows: page_rows,
                css_w,
                css_h,
                hidden: self.help,
                popup: l.popup,
            };
        }
        for t in &self.tabs {
            t.view.set_size(css_w, css_h, self.scale as f32);
        }
        self.renderer.clear_all();
        // The hint overlay was removed with every image, and its places are old.
        self.leave_hint_or_visual();
        if let Some(t) = self.tabs.get(self.active) {
            t.view.request_frame();
        }
        self.draw();
    }

    fn layout(&self) -> Layout {
        *self.renderer.layout.lock().unwrap()
    }

    /// Terminal cell to the page's CSS pixels; `None` outside the page.
    fn to_css(&self, col: u16, row: u16) -> Option<(f32, f32)> {
        let l = self.layout();
        if row < l.top || row >= l.top + l.rows {
            return None;
        }
        let x = (col as f32 + 0.5) * l.css_w as f32 / l.cols as f32;
        let y = (row - l.top) as f32 * l.css_h as f32 / l.rows as f32
            + 0.5 * l.css_h as f32 / l.rows as f32;
        Some((x, y))
    }

    // ---- Tabs ----

    pub fn open_tab(&mut self, url: Option<&str>, view: Option<View>) {
        let l = self.layout();
        let view = match view {
            Some(v) => {
                v.set_size(l.css_w, l.css_h, self.scale as f32);
                v
            }
            None => View::new_offscreen(None, l.css_w, l.css_h, self.scale as f32)
                .expect("off-screen view"),
        };
        let id = self.next_id;
        self.next_id += 1;
        view.set_frame_rate(
            std::env::var("ZWEB_FPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(240.0),
        );
        view.set_max_frames_in_flight(2);
        view.set_dialog_policy(DialogPolicy::Hold);
        // Copy buttons (navigator.clipboard.writeText): allowed, as Chrome
        // allows them after a click. Reading the clipboard stays denied.
        let _ = view.set_permission(
            "clipboard-write",
            zebkit::PermissionState::Granted,
            Some("*"),
        );
        // The engine has no Badging service: a call to setAppBadge makes
        // Chromium kill the page's renderer (Substack pages do this).
        view.add_script(
            "delete Navigator.prototype.setAppBadge;delete Navigator.prototype.clearAppBadge",
        );
        if self.opts.placeholders {
            view.add_script(&format!("{PLACEHOLDER_JS}(true)"));
        }
        let (renderer, active) = (self.renderer.clone(), self.active_id.clone());
        let debug = std::env::var_os("ZWEB_DEBUG").is_some();
        view.on_frame(move |frame| {
            if debug {
                eprintln!(
                    "frame tab={id} seq={} layer={:?} {}x{} damage={:?}",
                    frame.sequence(),
                    frame.layer(),
                    frame.width(),
                    frame.height(),
                    frame
                        .damage()
                        .iter()
                        .map(|r| (r.x, r.y, r.width, r.height))
                        .collect::<Vec<_>>()
                );
            }
            if active.get() == id {
                if crate::trace::on() {
                    let now = crate::trace::ticks_us();
                    crate::trace!(
                        "frame {} recv drawn {:.2} ms ago, ready {:.2} ms ago",
                        frame.sequence(),
                        (now - frame.drawn_us()) as f64 / 1000.0,
                        (now - frame.ready_us()) as f64 / 1000.0
                    );
                }
                renderer.submit(frame);
            }
        });
        view.on_load(move |event, url| {
            send(Msg::Load {
                tab: id,
                event,
                url: url.into(),
            })
        });
        view.on_title(move |title| {
            send(Msg::Title {
                tab: id,
                title: title.into(),
            })
        });
        view.on_caret(move |rect| {
            send(Msg::Caret {
                tab: id,
                on: rect.is_some(),
            })
        });
        view.on_popup(move |rect| {
            send(Msg::Popup {
                tab: id,
                rect: rect.map(|r| (r.x, r.y, r.width, r.height)),
            })
        });
        view.on_window_open(|view, _url| send(Msg::WindowOpen { view }));
        view.on_dialog(move |d| {
            send(Msg::Dialog {
                tab: id,
                dialog: d.clone(),
            })
        });
        if let Some(url) = url {
            view.load_url(url);
        }
        self.tabs.push(Tab {
            id,
            view,
            title: String::new(),
            url: url.unwrap_or("").into(),
            loading: url.is_some(),
            popup: false,
            pending_title: None,
            committed: false,
        });
        self.switch(self.tabs.len() - 1);
    }

    fn switch(&mut self, index: usize) {
        if self.tabs.is_empty() {
            return;
        }
        if self.prompt.is_some() {
            self.prompt_done(false);
        }
        let index = index.min(self.tabs.len() - 1);
        if let Some(old) = self.tabs.get(self.active) {
            old.view.set_focus(false);
            if self.mode == Mode::Visual {
                old.view.evaluate(COLLAPSE_JS, |_| {});
            }
        }
        self.active = index;
        self.find_generation += 1;
        self.mode = Mode::Browse;
        self.clear_hints();
        let tab = &self.tabs[index];
        let old_id = self.active_id.get();
        if old_id != tab.id && self.index_of(old_id).is_some() {
            self.previous_id = Some(old_id);
        }
        self.active_id.set(tab.id);
        tab.view.set_focus(true);
        self.renderer.layout.lock().unwrap().popup = None;
        self.renderer.clear_popup();
        self.renderer.forget();
        tab.view.request_frame();
        self.extents = None;
        self.query_extents();
        self.schedule_prepare();
        self.draw();
    }

    fn close_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return self.quit();
        }
        let tab = self.tabs.remove(self.active);
        push_closed(&mut self.closed, tab.url.clone());
        tab.view.close();
        self.switch(self.active.min(self.tabs.len() - 1));
    }

    fn reopen_tab(&mut self) {
        match self.closed.pop_back() {
            Some(url) => self.open_tab(Some(&url), None),
            None => {
                self.message = "no closed tab".into();
                self.draw();
            }
        }
    }

    fn alternate_tab(&mut self) {
        let index = self.previous_id.and_then(|id| self.index_of(id));
        if let Some(index) = index.filter(|&i| i != self.active) {
            self.switch(index);
        }
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub(crate) fn view(&self) -> View {
        self.tab().view.clone()
    }

    /// For the self-test: the app is in Visual mode.
    pub(crate) fn in_visual(&self) -> bool {
        self.mode == Mode::Visual
    }

    /// For the self-test: the active tab has not finished its load.
    pub(crate) fn loading(&self) -> bool {
        self.tab().loading
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn quit(&mut self) {
        self.saver.finish(&self.history);
        zebkit::quit();
    }

    // ---- Messages ----

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Term(event) => self.terminal_event(event),
            Msg::Load { tab, event, url } => self.load_event(tab, event, url),
            Msg::Title { tab, title } => self.title_changed(tab, title),
            Msg::Caret { tab, on } => self.caret_changed(tab, on),
            Msg::Popup { tab, rect } => self.popup_changed(tab, rect),
            Msg::WindowOpen { view } => {
                self.open_tab(None, Some(view));
                self.message = "opened a new tab".into();
            }
            Msg::Dialog { tab, dialog } => self.dialog_opened(tab, dialog),
            Msg::Hints { tab, reply } => self.hint_reply_received(tab, reply),
            Msg::Found {
                tab,
                generation,
                query,
                result,
            } => self.found(tab, generation, &query, result),
            Msg::Prepare { gen } => self.prepare_hints(gen),
            Msg::Prepared { tab, filter, json } => self.hints_prepared(tab, filter, &json),
            Msg::Extents { tab, json } => self.extents_received(tab, json),
            Msg::Focused { ok } => self.focused(ok),
            Msg::EditText { tab, json } => self.edit_text(tab, json),
            Msg::EditDone { tab, path, edited } => self.edit_done(tab, path, edited),
            Msg::EditWritten { path, ok } => self.edit_written(path, ok),
            Msg::VisualStarted { session, ok } => self.visual_started(session, ok),
            Msg::Yanked { tab, text } => self.yanked(tab, text),
            Msg::Tick => self.tick(),
        }
    }

    fn terminal_event(&mut self, event: Event) {
        crate::trace!("input handled");
        match event {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.key(k),
            Event::Mouse(m) => self.mouse_event(m),
            Event::Paste(text) => self.paste(&text),
            Event::Resize(..) => self.relayout(),
            _ => {}
        }
    }

    fn load_event(&mut self, tab: u64, event: LoadEvent, url: String) {
        let Some(i) = self.index_of(tab) else { return };
        match event {
            LoadEvent::Started => {
                let tab = &mut self.tabs[i];
                (tab.loading, tab.committed, tab.pending_title) = (true, false, None);
            }
            LoadEvent::Committed => self.load_committed(i, url),
            LoadEvent::Finished => {
                self.tabs[i].loading = false;
                if i == self.active {
                    self.query_extents();
                    self.schedule_prepare();
                }
            }
            LoadEvent::Failed => {
                self.tabs[i].loading = false;
                if i == self.active {
                    self.message = format!("load failed: {url}");
                }
            }
        }
        self.draw();
    }

    fn load_committed(&mut self, i: usize, url: String) {
        let tab = &mut self.tabs[i];
        // The empty page a view starts with is in its history: never stay there.
        if url == "about:blank" && !tab.url.is_empty() && tab.url != "about:blank" {
            tab.view.go_forward();
            if i == self.active {
                self.find_generation += 1;
                self.message = "no earlier page".into();
            }
            return;
        }
        tab.committed = true;
        let mut changed = self.history.visit(&url, history::now());
        if let Some(title) = tab.pending_title.take() {
            changed |= self.history.set_title(&url, &title);
        }
        if changed {
            self.saver.changed(Instant::now());
        }
        tab.url = url;
        for p in &mut self.prepared {
            if p.as_ref().is_some_and(|p| p.0 == tab.id) {
                *p = None;
            }
        }
        if i == self.active {
            if self.mode == Mode::Hint {
                self.renderer.hop(None);
            }
            if self.mode == Mode::Visual {
                tab.view.evaluate(COLLAPSE_JS, |_| {});
            }
            if matches!(self.mode, Mode::Hint | Mode::Visual) {
                self.mode = Mode::Browse;
            }
            self.renderer.forget();
            self.extents = None;
        }
    }

    fn title_changed(&mut self, tab: u64, title: String) {
        if let Some(i) = self.index_of(tab) {
            let tab = &mut self.tabs[i];
            if !tab.committed {
                tab.pending_title = Some(title.clone());
            } else if self.history.set_title(&tab.url, &title) {
                self.saver.changed(Instant::now());
            }
            tab.title = title;
            self.draw();
        }
    }

    fn caret_changed(&mut self, tab: u64, on: bool) {
        // A caret in an editable element, however it got there (a click,
        // Tab, the page's autofocus): keys type into it, as in Vimium.
        // Esc leaves it. A find's selection is not editable: no change.
        if self.index_of(tab) != Some(self.active) || self.prompt.is_some() {
            return;
        }
        if on && self.mode == Mode::Browse {
            self.view().evaluate(EDITABLE_JS, |r| {
                if r.as_deref() == Ok("true") {
                    send(Msg::Focused { ok: true });
                }
            });
        } else if !on && self.mode == Mode::Insert {
            self.mode = Mode::Browse;
            self.draw();
        }
    }

    fn popup_changed(&mut self, tab: u64, rect: Option<(i32, i32, i32, i32)>) {
        if let Some(i) = self.index_of(tab) {
            self.tabs[i].popup = rect.is_some();
            if i == self.active {
                self.renderer.layout.lock().unwrap().popup = rect;
                if rect.is_none() {
                    self.renderer.clear_popup();
                    self.view().request_frame();
                }
            }
        }
    }

    fn dialog_opened(&mut self, tab: u64, dialog: Dialog) {
        let Some(i) = self.index_of(tab) else { return };
        if !dialog.held {
            return;
        }
        self.switch(i);
        let (label, text) = match dialog.kind {
            DialogType::Alert => (format!("alert: {} [Enter]", dialog.message), String::new()),
            DialogType::Confirm => (format!("confirm: {} [y/n]", dialog.message), String::new()),
            DialogType::Prompt => (
                format!("prompt: {}: ", dialog.message),
                dialog.default_prompt.clone(),
            ),
            DialogType::BeforeUnload => ("leave this page? [y/n]".into(), String::new()),
            DialogType::Print => {
                self.view().respond_dialog(dialog.id, false, None);
                return;
            }
        };
        self.prompt(PromptKind::Dialog(dialog.id, dialog.kind), &label, &text);
        self.draw();
    }

    fn hint_reply_received(&mut self, tab: u64, reply: HintReply) {
        match reply {
            HintReply::Targets(json) => self.hints_received(tab, json),
            HintReply::Image(json) => self.hint_image_received(tab, &json),
        }
    }

    fn hints_received(&mut self, tab: u64, json: Result<String, String>) {
        crate::trace!("hints result");
        if self.index_of(tab) != Some(self.active) || self.mode != Mode::Hint {
            return;
        }
        // A JSON string of the JSON list, of {png, list}, or of {id}: the
        // hints read ahead are still right (hints.js).
        let v = json.ok().map(|j| parse_hints(&j)).unwrap_or_default();
        let prepared = self.prepared[self.hint_filter as usize]
            .take()
            .filter(|p| p.0 == tab && v.0.as_deref() == Some(p.1.as_str()));
        let hop = match prepared {
            Some((_, _, hop, hints)) => {
                self.hints = hints;
                Some(hop)
            }
            None => {
                self.hints = v.2;
                v.1
            }
        };
        if self.hints.is_empty() {
            self.mode = Mode::Browse;
            self.message = "no links here".into();
        } else if let Some(hop) = hop {
            self.hint_dim = Some(hop.dim);
            if self.hint_filter {
                self.hint_hop = Some(hop.clone());
            }
            self.renderer.hop(Some(hop));
        }
        self.draw();
    }

    fn hint_image_received(&mut self, tab: u64, json: &str) {
        if self.index_of(tab) == Some(self.active) && self.mode == Mode::Hint {
            let mut hop = serde_json::from_str(json).ok().and_then(|v| parse_hop(&v));
            // Same dim layer as when the hints were shown.
            if let (Some(h), Some(dim)) = (hop.as_mut(), self.hint_dim) {
                h.dim = dim;
            }
            if hop.is_some() {
                self.renderer.hop(hop);
            }
        }
    }

    fn prepare_hints(&self, gen: u64) {
        // Only once hints were used: the canvas's first text can load fonts
        // for long (over 300 ms on CI runners), and every page script after
        // it waits. That cost then comes with the first `f`, as before. Not
        // while a prompt or help is open: its page scripts wait behind this.
        if gen == self.prep_gen
            && self.hints_used
            && self.mode == Mode::Browse
            && self.prompt.is_none()
            && !self.help
            && !self.tabs.is_empty()
        {
            let lib = HINTS_JS.split("\n(() => {").next().unwrap_or("");
            let tab = self.tab().id;
            for filter in [false, true] {
                crate::trace!("hints prepare");
                self.view().evaluate(
                    &format!(
                        "{}{lib};__zwebHopPrepare(__zwebHopKeys)",
                        self.hints_mode(filter)
                    ),
                    move |r| {
                        crate::trace!(
                            "hints prepared {}",
                            r.as_ref()
                                .map_or_else(|e| e.to_string(), |j| format!("{} B", j.len()))
                        );
                        if let Ok(json) = r {
                            send(Msg::Prepared { tab, filter, json });
                        }
                    },
                );
            }
        }
    }

    fn hints_prepared(&mut self, tab: u64, filter: bool, json: &str) {
        if let (Some(id), Some(hop), hints) = parse_hints(json) {
            self.prepared[filter as usize] = Some((tab, id, hop, hints));
        }
    }

    fn extents_received(&mut self, tab: u64, json: Option<String>) {
        if self.index_of(tab) == Some(self.active) {
            let v: Option<Vec<f32>> = json.and_then(|j| serde_json::from_str(&j).ok());
            self.extents = v.filter(|v| v.len() == 3).map(|v| (v[0], v[1], v[2]));
        }
    }

    fn focused(&mut self, ok: bool) {
        if ok {
            self.mode = Mode::Insert;
        } else {
            self.message = "no text field here".into();
        }
        self.draw();
    }

    fn edit_text(&mut self, tab: u64, json: Result<String, String>) {
        let v: serde_json::Value = json
            .ok()
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let Some(original) = v["text"].as_str().map(String::from) else {
            self.editing = false;
            self.message = match v.as_str() {
                Some("password") => "password field: not edited",
                Some("readonly") => "read-only field: not edited",
                _ => "no text field here",
            }
            .into();
            return self.draw();
        };
        let path = match edit::temp_file(&original) {
            Ok(path) => path,
            Err(error) => {
                self.editing = false;
                self.message = format!("cannot write edit file: {error}");
                return self.draw();
            }
        };
        let (single_line, enhanced) = (v["input"].as_bool().unwrap_or(false), self.opts.enhanced);
        std::thread::spawn(move || {
            let edited = edit::run(&path, &original, single_line, enhanced);
            zebkit::post(move || send(Msg::EditDone { tab, path, edited }));
        });
    }

    fn edit_done(&mut self, tab: u64, path: PathBuf, edited: Edited) {
        edit::INPUT_PAUSED.store(false, std::sync::atomic::Ordering::SeqCst);
        let text = match edited {
            Edited::Text(text) => text,
            other => {
                self.editing = false;
                let _ = std::fs::remove_file(&path);
                self.message = if matches!(other, Edited::Failed) {
                    "editor failed: no change"
                } else {
                    "no change"
                }
                .into();
                return self.relayout();
            }
        };
        self.relayout();
        let Some(i) = self.index_of(tab) else {
            return self.edit_written(path, false);
        };
        self.tabs[i]
            .view
            .evaluate(&edit::write_js(&text), move |r| {
                send(Msg::EditWritten {
                    path,
                    ok: r.as_deref() == Ok("true"),
                })
            });
    }

    fn edit_written(&mut self, path: PathBuf, ok: bool) {
        self.editing = false;
        if ok {
            let _ = std::fs::remove_file(&path);
        } else {
            self.message = format!("field gone or read-only: text kept in {}", path.display());
        }
        self.draw();
    }

    fn tick(&mut self) {
        self.saver.tick(&self.history, Instant::now());
        let frames = self.renderer.stats.lock().unwrap().frames;
        if frames != self.last_stats {
            self.last_stats = frames;
            self.draw();
        }
    }

    // ---- Keys ----

    fn key(&mut self, k: KeyEvent) {
        let global = self.bindings.action(Scope::Global, k);
        if global == Some(Action::Quit) {
            return self.quit();
        }
        if self.help {
            return self.help_key(k);
        }
        if self.prompt.is_some() {
            return self.prompt_key(k);
        }
        if let Some(action) = global {
            return self.global_command(action);
        }
        // An open <select> list takes every key.
        if self.tab().popup {
            return self.forward(&k);
        }
        match self.mode {
            Mode::Insert => {
                if self.bindings.action(Scope::Insert, k) == Some(Action::Cancel) {
                    self.mode = Mode::Browse;
                    // Kept for `g e` after Esc (edit.rs `READ_JS`).
                    self.view().evaluate(
                        "(e=>{window.__zwebLastEdit=e;e&&e.blur()})(document.activeElement)",
                        |_| {},
                    );
                    self.draw();
                } else {
                    self.forward(&k);
                }
            }
            Mode::Hint => self.hint_key(k),
            Mode::Visual => self.visual_key(k),
            Mode::Browse => self.command(k),
        }
    }

    fn forward(&self, k: &KeyEvent) {
        if let Some(dom) = keys::to_dom(k) {
            keys::send(&self.tab().view, &dom);
        }
    }

    fn command(&mut self, k: KeyEvent) {
        let prefix = self.pending_key.take();
        match self.bindings.resolve(Scope::Browse, k, prefix) {
            Binding::Action(action) => self.browser_command(action),
            Binding::Prefix(prefix) => self.pending_key = Some(prefix),
            Binding::Unbound => {}
        }
    }

    fn browser_command(&mut self, action: Action) {
        self.message.clear();
        match action {
            Action::Quit => self.quit(),
            Action::QuitHint => {
                self.message = format!(
                    "{} quits; {} or {} shows help",
                    self.bindings.labels(Scope::Global, Action::Quit),
                    self.bindings.labels(Scope::Browse, Action::Help),
                    self.bindings.labels(Scope::Global, Action::Help)
                );
                self.draw();
            }
            Action::CloseTab => self.close_tab(),
            Action::Help => self.show_help(),
            _ => {
                self.browse_command(action);
                self.draw();
            }
        }
    }

    fn browse_command(&mut self, action: Action) {
        if self.scroll_command(action)
            || self.jump_command(action)
            || self.navigation_command(action)
            || self.tab_command(action)
            || self.prompt_command(action)
            || self.page_command(action)
            || self.zoom_command(action)
        {
            return;
        }
        self.page_focus_command(action);
    }

    fn scroll_command(&mut self, action: Action) -> bool {
        let l = self.layout();
        let line = STEP_ROWS * self.row_css;
        let half = (l.rows / 2) as f32 * self.row_css;
        let page = l.rows.saturating_sub(2).max(1) as f32 * self.row_css;
        let (dx, dy) = match action {
            Action::ScrollDown => (0.0, line),
            Action::ScrollUp => (0.0, -line),
            Action::ScrollLeft => (-line, 0.0),
            Action::ScrollRight => (line, 0.0),
            Action::HalfDown => (0.0, half),
            Action::HalfUp => (0.0, -half),
            Action::PageDown => (0.0, page),
            Action::PageUp => (0.0, -page),
            _ => return false,
        };
        self.wheel(dx, dy);
        true
    }

    fn jump_command(&mut self, action: Action) -> bool {
        match action {
            Action::Top => self.view().evaluate("scrollTo(0,0)", |_| {}),
            Action::Bottom => self
                .view()
                .evaluate("scrollTo(0,document.scrollingElement.scrollHeight)", |_| {}),
            _ => return false,
        }
        true
    }

    fn navigation_command(&self, action: Action) -> bool {
        match action {
            Action::Back => self.view().go_back(),
            Action::Forward => self.view().go_forward(),
            Action::Reload => self.view().reload(),
            Action::Stop => self.view().stop(),
            _ => return false,
        }
        true
    }

    fn tab_command(&mut self, action: Action) -> bool {
        match action {
            Action::NextTab => self.switch((self.active + 1) % self.tabs.len()),
            Action::PreviousTab => {
                self.switch((self.active + self.tabs.len() - 1) % self.tabs.len())
            }
            Action::ReopenTab => self.reopen_tab(),
            Action::AlternateTab => self.alternate_tab(),
            Action::Tab1 => self.switch(0),
            Action::Tab2 => self.switch(1),
            Action::Tab3 => self.switch(2),
            Action::Tab4 => self.switch(3),
            Action::Tab5 => self.switch(4),
            Action::Tab6 => self.switch(5),
            Action::Tab7 => self.switch(6),
            Action::Tab8 => self.switch(7),
            Action::Tab9 => self.switch(8),
            _ => return false,
        }
        true
    }

    fn prompt_command(&mut self, action: Action) -> bool {
        match action {
            Action::Open => self.prompt(PromptKind::Open, "open: ", ""),
            Action::EditAddress => {
                let url = self.tab().url.clone();
                self.prompt(PromptKind::Open, "open: ", &url)
            }
            Action::NewTab => self.prompt(PromptKind::OpenTab, "new tab: ", ""),
            Action::Find => self.start_find(false),
            Action::NextMatch => self.find_next(false),
            Action::PreviousMatch => self.find_next(true),
            _ => return false,
        }
        true
    }

    fn page_command(&mut self, action: Action) -> bool {
        match action {
            Action::FocusInput => {
                let focus = "(()=>{const e=[...document.querySelectorAll('input:not([type=hidden]),textarea,[contenteditable]')]\
                    .find(e=>e.getClientRects().length);if(e){e.focus();return true}return false})()";
                self.view().evaluate(focus, |r| {
                    send(Msg::Focused {
                        ok: r.as_deref() == Ok("true"),
                    })
                });
            }
            Action::EditExternal if self.editing => {
                self.message = "editor already open".into();
                self.draw();
            }
            Action::EditExternal => {
                self.editing = true;
                let tab = self.tab().id;
                self.view().evaluate(edit::READ_JS, move |r| {
                    send(Msg::EditText {
                        tab,
                        json: r.map_err(|e| e.to_string()),
                    })
                });
            }
            Action::Insert => self.mode = Mode::Insert,
            Action::Hints => self.start_hints(false, false),
            Action::HintsNewTab => self.start_hints(true, false),
            Action::HintsFilter => self.start_hints(false, true),
            Action::CopyUrl => self.copy_url(),
            Action::Visual => self.start_visual(),
            Action::ToggleImages => self.toggle_placeholders(),
            _ => return false,
        }
        true
    }

    fn zoom_command(&mut self, action: Action) -> bool {
        match action {
            Action::ZoomIn => self.set_zoom(self.zoom * 1.1),
            Action::ZoomOut => self.set_zoom(self.zoom / 1.1),
            Action::ZoomReset => self.set_zoom(1.0),
            _ => return false,
        }
        true
    }

    fn page_focus_command(&self, action: Action) {
        let code = match action {
            Action::FocusNext => KeyCode::Tab,
            Action::FocusPrevious => KeyCode::BackTab,
            Action::Activate => KeyCode::Enter,
            Action::PageEscape => {
                self.view()
                    .evaluate("window.__zwebSearch?.mark('')", |_| {});
                KeyCode::Esc
            }
            _ => return,
        };
        self.forward(&KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn copy_url(&mut self) {
        let url = self.tab().url.clone();
        self.renderer.write(&osc52(&url));
        self.message = format!("copied {url}");
    }

    fn toggle_placeholders(&mut self) {
        self.opts.placeholders = !self.opts.placeholders;
        // Preloads cannot be removed: each toggle adds one, the last wins.
        let js = format!("{PLACEHOLDER_JS}({})", self.opts.placeholders);
        for t in &self.tabs {
            t.view.add_script(&js);
            t.view.evaluate(&js, |_| {});
        }
        self.message = if self.opts.placeholders {
            "images: placeholders"
        } else {
            "images: shown"
        }
        .into();
    }

    fn wheel(&mut self, dx: f32, dy: f32) {
        let l = self.layout();
        let (x, y) = if self.mouse == (0.0, 0.0) {
            (l.css_w as f32 / 2.0, l.css_h as f32 / 2.0)
        } else {
            self.mouse
        };
        self.scroll_at(x, y, dx, dy, Modifiers::NONE);
    }

    /// A wheel event; a small vertical step of the page is shown at once
    /// from the rows the terminal holds (render.rs, `predict`).
    fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, mods: Modifiers) {
        self.leave_for_wheel();
        if dx == 0.0 {
            self.predict(dy);
        }
        self.view()
            .send_wheel(x, y, dx, dy, false, WheelPhase::None, mods);
        crate::trace!("wheel sent {dy} at {}", crate::trace::ticks_us());
        self.query_extents();
        self.schedule_prepare();
    }

    /// A wheel scroll ends Hint and Visual mode.
    fn leave_for_wheel(&mut self) {
        if matches!(self.mode, Mode::Hint | Mode::Visual) {
            self.leave_hint_or_visual();
            self.draw();
        }
    }

    fn predict(&mut self, dy: f32) {
        let l = self.layout();
        let k = (dy / self.row_css).round() as i32;
        // Any step: the renderer shows it only when it knows the rows it brings.
        let small = k != 0 && k.unsigned_abs() < l.rows as u32;
        let browsing =
            self.mode != Mode::Hint && !self.help && self.prompt.is_none() && !self.tab().popup;
        // Only where the page itself scrolls the whole step.
        let Some((sy, ih, sh)) = self.extents else {
            return;
        };
        let room = if dy > 0.0 { sh - ih - sy } else { sy };
        let off = std::env::var_os("ZWEB_NOPREDICT").is_some();
        if small
            && browsing
            && !off
            && room >= dy.abs()
            && self.renderer.gfx != crate::render::Gfx::Blocks
        {
            // Shown only while predictions for this page are right (render.rs).
            self.renderer.predict(k);
            self.extents = Some((sy + dy, ih, sh));
        }
    }

    fn schedule_prepare(&mut self) {
        self.prep_gen += 1;
        let gen = self.prep_gen;
        zebkit::after(Duration::from_millis(200), move || {
            send(Msg::Prepare { gen })
        });
    }

    fn query_extents(&self) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let id = t.id;
        t.view.evaluate(
            "[scrollY, innerHeight, document.scrollingElement ? document.scrollingElement.scrollHeight : 0]",
            move |r| send(Msg::Extents { tab: id, json: r.ok() }),
        );
    }

    fn set_zoom(&mut self, zoom: f64) {
        self.zoom = zoom.clamp(0.3, 4.0);
        self.message = format!("zoom {:.0}%", self.zoom * 100.0);
        self.relayout();
    }

    fn find_next(&mut self, back: bool) {
        if self.find.is_empty() {
            self.message = format!(
                "Use {} or {} to find text in this page",
                self.bindings.labels(Scope::Browse, Action::Find),
                self.bindings.labels(Scope::Global, Action::Find)
            );
            return;
        }
        // The page may have changed since the prompt: define the helpers again.
        let lib = FIND_JS.split("\n// SEARCH SESSION").next().unwrap_or("");
        let script = format!(
            "{lib};__zwebSearch.step({}, {back})",
            serde_json::to_string(&self.find).unwrap()
        );
        self.run_find(&script, self.find.clone());
    }

    fn global_command(&mut self, action: Action) {
        self.cancel_input();
        if action == Action::Find {
            self.start_find(true);
            self.draw();
        } else {
            self.browser_command(action);
        }
    }

    fn cancel_input(&mut self) {
        self.pending_key = None;
        if self.prompt.is_some() {
            self.prompt_done(false);
        }
        self.leave_hint_or_visual();
    }

    fn start_find(&mut self, keep_open: bool) {
        self.find_generation += 1;
        self.view().evaluate(FIND_JS, |result| {
            if let Err(error) = result {
                eprintln!("webskiff: cannot start page search: {error}");
            }
        });
        self.prompt(PromptKind::Find, "/", "");
        let p = self.prompt.as_mut().unwrap();
        p.keep_open = keep_open;
        if keep_open {
            p.hint = format!(
                "Find in page; {} next, {} previous, {} closes",
                self.bindings.labels(Scope::Prompt, Action::Accept),
                self.bindings.labels(Scope::Prompt, Action::PreviousMatch),
                self.bindings.labels(Scope::Prompt, Action::Cancel)
            );
        }
    }

    fn preview_find(&mut self) {
        let Some(p) = self.prompt.as_ref().filter(|p| p.kind == PromptKind::Find) else {
            return;
        };
        let query = p.input.text.clone();
        let script = format!(
            "window.__zwebFind.preview({})",
            serde_json::to_string(&query).unwrap()
        );
        self.run_find(&script, query);
    }

    fn run_find(&mut self, script: &str, query: String) {
        self.find_generation += 1;
        let generation = self.find_generation;
        let tab = self.tab().id;
        self.view().evaluate(script, move |result| {
            send(Msg::Found {
                tab,
                generation,
                query,
                result: result.map(|s| s == "true").map_err(|e| e.to_string()),
            });
        });
    }

    fn found(&mut self, tab: u64, generation: u64, query: &str, result: Result<bool, String>) {
        if self.tab().id != tab || generation != self.find_generation {
            return;
        }
        match result {
            Err(error) => {
                self.message = format!("search failed: {error}");
                eprintln!("webskiff: {}", self.message);
            }
            Ok(found) => {
                if let Some(p) = self.prompt.as_mut().filter(|p| p.kind == PromptKind::Find) {
                    let not_found = !found && !query.is_empty();
                    if p.not_found == not_found {
                        return;
                    }
                    p.not_found = not_found;
                } else {
                    self.message = if found {
                        format!("/{query}")
                    } else {
                        format!("not found: {query}")
                    };
                }
            }
        }
        self.draw();
    }

    // ---- Hints ----

    fn start_hints(&mut self, new_tab: bool, filter: bool) {
        self.hints_used = true;
        self.mode = Mode::Hint;
        self.hint_new_tab = new_tab;
        self.hint_filter = filter;
        self.hint_typed.clear();
        self.hints.clear();
        let id = self.tab().id;
        let kept = self.prepared[filter as usize]
            .as_ref()
            .filter(|p| p.0 == id)
            .map_or("", |p| p.1.as_str());
        let js = format!(
            "{}window.__zwebHopKept={};{HINTS_JS}",
            self.hints_mode(filter),
            serde_json::to_string(kept).unwrap()
        );
        self.view().evaluate(&js, move |r| {
            send(Msg::Hints {
                tab: id,
                reply: HintReply::Targets(r.map_err(|e| e.to_string())),
            })
        });
    }

    /// With Kitty graphics, hints.js gives the overlay as a terminal image:
    /// showing it needs no new page frame. And the code keys.
    fn hints_mode(&self, filter: bool) -> String {
        let keys = format!("window.__zwebHopKeys='{}';", HINT_KEYS[filter as usize]);
        if self.renderer.gfx == crate::render::Gfx::Blocks {
            return keys;
        }
        let l = self.layout();
        format!("{keys}window.__zwebHopGrid=[{},{}];", l.cols, l.rows)
    }

    fn clear_hints(&mut self) {
        if !self.hints.is_empty() {
            self.renderer.hop(None);
        }
        self.hint_dim = None;
        self.hints.clear();
        self.hint_typed.clear();
        self.hint_text.clear();
        self.hint_hop = None;
        if let Some(t) = self.tabs.get(self.active) {
            t.view
                .evaluate("window.__zwebHop && __zwebHop.clear()", |_| {});
        }
        self.schedule_prepare();
    }

    /// Keeps the codes that start with `typed`, showing what is left of them.
    fn show_hints(&self, typed: &str) {
        let js = format!(
            "window.__zwebHop && __zwebHop.show({})",
            serde_json::to_string(typed).unwrap()
        );
        let id = self.tab().id;
        self.view().evaluate(&js, move |r| {
            if let Some(json) = r.ok().and_then(|j| serde_json::from_str::<String>(&j).ok()) {
                send(Msg::Hints {
                    tab: id,
                    reply: HintReply::Image(json),
                });
            }
        });
    }

    fn edit_hint(&mut self, k: KeyEvent) {
        match self.bindings.action(Scope::Hint, k) {
            Some(Action::Cancel) => {
                self.mode = Mode::Browse;
                self.clear_hints();
            }
            Some(Action::Backspace) => {
                self.hint_typed.pop();
            }
            None => {
                if let Some(c) = input::character(k) {
                    self.hint_typed.push(c.to_ascii_lowercase());
                }
            }
            _ => {}
        }
    }

    fn hint_key(&mut self, k: KeyEvent) {
        if self.hint_filter {
            return self.filter_key(k);
        }
        self.edit_hint(k);
        if self.mode != Mode::Hint {
            return self.draw();
        }
        let typed = self.hint_typed.clone();
        let matches: Vec<usize> = (0..self.hints.len())
            .filter(|&i| self.hints[i].key.starts_with(&typed))
            .collect();
        if matches.is_empty() && !self.hints.is_empty() {
            self.hint_typed.pop();
        } else if matches.len() == 1 && self.hints[matches[0]].key == typed {
            return self.pick_hint(matches[0]);
        } else {
            self.show_hints(&self.hint_typed);
        }
        self.draw();
    }

    /// Ends Hint or Visual mode, as a scroll, click, resize or tab switch does.
    fn leave_hint_or_visual(&mut self) {
        match self.mode {
            Mode::Hint => self.clear_hints(),
            Mode::Visual => self.view().evaluate(COLLAPSE_JS, |_| {}),
            _ => return,
        }
        self.mode = Mode::Browse;
    }

    // ---- Visual ----

    /// Visual mode starts at once: the page runs scripts in order, so the
    /// keys that follow act on the selection the start script makes.
    fn start_visual(&mut self) {
        self.visual_session += 1;
        let session = self.visual_session;
        self.mode = Mode::Visual;
        self.view().evaluate(VISUAL_START_JS, move |r| {
            send(Msg::VisualStarted {
                session,
                ok: r.as_deref() == Ok("true"),
            })
        });
    }

    /// Any change of mode or tab since the start ends Visual: only a fail of
    /// the session still open has effect.
    fn visual_started(&mut self, session: u64, ok: bool) {
        if ok || self.mode != Mode::Visual || session != self.visual_session {
            return;
        }
        self.mode = Mode::Browse;
        self.message = "no text here".into();
        self.draw();
    }

    fn visual_key(&mut self, k: KeyEvent) {
        match self.bindings.action(Scope::Visual, k) {
            Some(Action::Cancel) => self.view().evaluate(COLLAPSE_JS, |_| {}),
            Some(Action::Yank) => {
                let tab = self.tab().id;
                self.view().evaluate(YANK_JS, move |r| {
                    send(Msg::Yanked {
                        tab,
                        text: r.ok().and_then(|j| serde_json::from_str(&j).ok()),
                    })
                });
            }
            // Fire and forget: the selection's paint comes as a frame.
            Some(action) => {
                if let Some(js) = visual_script(action) {
                    self.view().evaluate(&js, |_| {});
                }
                return;
            }
            None => return,
        }
        self.mode = Mode::Browse;
        self.draw();
    }

    fn yanked(&mut self, tab: u64, text: Option<String>) {
        if self.index_of(tab) != Some(self.active) {
            return;
        }
        let Some(text) = text else {
            self.message = "cannot read the selection".into();
            return self.draw();
        };
        let (bytes, message) = yank(&text);
        if let Some(bytes) = bytes {
            self.renderer.write(&bytes);
        }
        self.message = message;
        self.draw();
    }

    /// Filtered hints: other characters filter by text, digits by code;
    /// Enter picks the first left. Labels never change, and no new image is
    /// made: the overlay is placed again with only the codes left.
    fn filter_key(&mut self, k: KeyEvent) {
        match self.bindings.action(Scope::Hint, k) {
            Some(Action::Cancel) => {
                self.mode = Mode::Browse;
                self.clear_hints();
                return self.draw();
            }
            _ if self.hints.is_empty() => return,
            Some(Action::Backspace) => {
                if self.hint_typed.pop().is_none() {
                    self.hint_text.pop();
                }
            }
            Some(Action::Accept) => return self.pick_hint(self.filtered()[0]),
            None => {
                if let Some(c) = input::character(k) {
                    let digit = c.is_ascii_digit();
                    let field = if digit {
                        &mut self.hint_typed
                    } else {
                        &mut self.hint_text
                    };
                    field.push(c);
                    if self.filtered().is_empty() {
                        let field = if digit {
                            &mut self.hint_typed
                        } else {
                            &mut self.hint_text
                        };
                        field.pop();
                    }
                }
            }
            _ => {}
        }
        let left = self.filtered();
        if left.len() == 1 {
            return self.pick_hint(left[0]);
        }
        if let Some(hop) = &self.hint_hop {
            let cells = left
                .iter()
                .filter_map(|&i| hop.cells.get(i).copied())
                .collect();
            self.renderer.hop(Some(Hop {
                cells,
                ..hop.clone()
            }));
        } else {
            let js = format!(
                "window.__zwebHop && __zwebHop.pick({})",
                serde_json::to_string(&left).unwrap()
            );
            self.view().evaluate(&js, |_| {});
        }
        self.draw();
    }

    fn filtered(&self) -> Vec<usize> {
        filter_hints(&self.hints, &self.hint_text, &self.hint_typed)
    }

    /// Follows hint `i`: a click at it, or its link in a new tab.
    fn pick_hint(&mut self, i: usize) {
        let h = &self.hints[i];
        let (x, y, href, edit) = (h.x, h.y, h.href.clone(), h.edit);
        self.mode = Mode::Browse;
        self.clear_hints();
        if self.hint_new_tab {
            if let Some(href) = href {
                self.open_tab(Some(&href), None);
                return;
            }
        }
        self.last_click = (Instant::now(), x, y, 1);
        self.click(x, y, 1);
        if edit {
            self.mode = Mode::Insert;
        }
        self.draw();
    }

    // ---- Mouse ----

    fn click(&mut self, x: f32, y: f32, count: u32) {
        let v = self.view();
        v.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, Modifiers::NONE);
        v.send_mouse(
            ZMouse::Down,
            x,
            y,
            MouseButton::Left,
            count,
            Modifiers::NONE,
        );
        v.send_mouse(ZMouse::Up, x, y, MouseButton::Left, count, Modifiers::NONE);
    }

    fn mouse_event(&mut self, m: MouseEvent) {
        if self.help || self.prompt.is_some() {
            return;
        }
        if m.row == 0 {
            if let MouseEventKind::Down(TButton::Left) = m.kind {
                self.tab_bar_click(m.column);
            }
            return;
        }
        let Some((x, y)) = self.to_css(m.column, m.row) else {
            return;
        };
        self.mouse = (x, y);
        self.page_mouse_event(m.kind, x, y, m.modifiers);
    }

    fn page_mouse_event(&mut self, kind: MouseEventKind, x: f32, y: f32, modifiers: KeyModifiers) {
        let mut mods = Modifiers::NONE;
        if modifiers.contains(KeyModifiers::SHIFT) {
            mods = mods | Modifiers::SHIFT;
        }
        if modifiers.contains(KeyModifiers::CONTROL) {
            mods = mods | Modifiers::CONTROL;
        }
        if modifiers.contains(KeyModifiers::ALT) {
            mods = mods | Modifiers::ALT;
        }
        let button = |b: TButton| match b {
            TButton::Left => MouseButton::Left,
            TButton::Right => MouseButton::Right,
            TButton::Middle => MouseButton::Middle,
        };
        let v = self.view();
        let line = STEP_ROWS * self.row_css;
        match kind {
            MouseEventKind::ScrollDown => self.scroll_at(x, y, 0.0, line, mods),
            MouseEventKind::ScrollUp => self.scroll_at(x, y, 0.0, -line, mods),
            MouseEventKind::Down(b) => self.mouse_down(&v, x, y, button(b), mods),
            MouseEventKind::Up(b) => {
                v.send_mouse(ZMouse::Up, x, y, button(b), self.last_click.3, mods)
            }
            MouseEventKind::Drag(_) => v.send_mouse(
                ZMouse::Move,
                x,
                y,
                MouseButton::None,
                0,
                mods | Modifiers::LEFT_BUTTON,
            ),
            MouseEventKind::Moved => v.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, mods),
            MouseEventKind::ScrollRight => {
                self.leave_for_wheel();
                v.send_wheel(x, y, line, 0.0, false, WheelPhase::None, mods)
            }
            MouseEventKind::ScrollLeft => {
                self.leave_for_wheel();
                v.send_wheel(x, y, -line, 0.0, false, WheelPhase::None, mods)
            }
        }
    }

    fn mouse_down(&mut self, view: &View, x: f32, y: f32, button: MouseButton, mods: Modifiers) {
        self.leave_hint_or_visual();
        let (at, lx, ly, n) = self.last_click;
        let count = if at.elapsed() < Duration::from_millis(400)
            && (lx - x).abs() < 4.0
            && (ly - y).abs() < 4.0
        {
            n + 1
        } else {
            1
        };
        self.last_click = (Instant::now(), x, y, count);
        view.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, mods);
        view.send_mouse(ZMouse::Down, x, y, button, count, mods);
    }

    fn tab_bar_click(&mut self, col: u16) {
        let mut x = 0u16;
        for (i, label) in self.tab_labels().iter().enumerate() {
            let w = label.width() as u16;
            if col >= x && col < x + w {
                return self.switch(i);
            }
            x += w;
        }
    }

    fn paste(&mut self, text: &str) {
        if let Some(p) = &mut self.prompt {
            if matches!(
                p.kind,
                PromptKind::Dialog(_, DialogType::Confirm | DialogType::BeforeUnload)
            ) {
                return;
            }
            p.input.insert(text.trim_end_matches(['\r', '\n']));
            p.suggest(&self.history, history::now());
            self.preview_find();
            return self.draw();
        }
        if self.mode == Mode::Insert || self.mode == Mode::Browse {
            self.view().ime_commit(text);
        }
    }

    // ---- Prompt ----

    fn prompt(&mut self, kind: PromptKind, label: &str, text: &str) {
        let task = match kind {
            PromptKind::Find => "Find in page",
            PromptKind::Open | PromptKind::OpenTab => "Address or web search",
            _ => "",
        };
        let hint = format!(
            "{task}; {} accepts, {} cancels",
            self.bindings.labels(Scope::Prompt, Action::Accept),
            self.bindings.labels(Scope::Prompt, Action::Cancel)
        );
        let mut prompt = Prompt::new(kind, label, text);
        prompt.hint = hint;
        prompt.suggest(&self.history, history::now());
        self.prompt = Some(prompt);
    }

    fn prompt_key(&mut self, k: KeyEvent) {
        let action = self.bindings.action(Scope::Prompt, k);
        let p = self.prompt.as_ref().unwrap();
        let yes_no = matches!(
            p.kind,
            PromptKind::Dialog(_, DialogType::Confirm | DialogType::BeforeUnload)
        );
        if yes_no {
            match input::character(k).map(|c| c.to_ascii_lowercase()) {
                Some('y') => return self.prompt_done(true),
                Some('n') => return self.prompt_done(false),
                _ => {}
            }
        }
        if action.is_none() && !matches!(p.kind, PromptKind::Dialog(..)) {
            if let Some(global) = self.bindings.action(Scope::Global, k) {
                return self.global_command(global);
            }
        }
        self.edit_prompt(k, action, yes_no);
    }

    fn edit_prompt(&mut self, k: KeyEvent, action: Option<Action>, yes_no: bool) {
        let p = self.prompt.as_mut().unwrap();
        let changed = match action {
            Some(Action::Cancel) => {
                let accept = p.keep_open && !p.input.text.is_empty();
                return self.prompt_done(accept);
            }
            Some(Action::Accept) if p.kind == PromptKind::Find && p.keep_open => {
                return self.prompt_match(false)
            }
            Some(Action::Accept) => return self.prompt_done(true),
            Some(Action::PreviousMatch) if p.kind == PromptKind::Find => {
                return self.prompt_match(true)
            }
            Some(Action::NextMatch) if p.kind == PromptKind::Find => {
                return self.prompt_match(false)
            }
            Some(action @ (Action::Complete | Action::CompletePrevious)) => {
                p.complete(
                    &self.history,
                    action == Action::CompletePrevious,
                    history::now(),
                );
                false
            }
            Some(action) if !yes_no => p.input.edit(action) == Edit::Changed,
            None if !yes_no => {
                if let Some(c) = input::character(k) {
                    p.input.insert(&c.to_string());
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if changed {
            p.suggest(&self.history, history::now());
            self.preview_find();
        }
        self.draw();
    }

    fn prompt_match(&mut self, back: bool) {
        let p = self.prompt.as_ref().unwrap();
        if !p.input.text.is_empty() {
            self.find = p.input.text.clone();
        }
        self.find_next(back);
        self.draw();
    }

    fn prompt_done(&mut self, accept: bool) {
        let p = self.prompt.take().unwrap();
        // Hints were not read ahead while it was open.
        self.schedule_prepare();
        let text = p.input.text;
        match p.kind {
            PromptKind::Open | PromptKind::OpenTab if accept && !text.trim().is_empty() => {
                let url = match to_url(text.trim()) {
                    Ok(url) => url,
                    Err(error) => {
                        self.message = format!("cannot open file: {error}");
                        return self.draw();
                    }
                };
                if p.kind == PromptKind::OpenTab {
                    self.open_tab(Some(&url), None);
                } else {
                    self.tab().view.load_url(&url);
                    self.tabs[self.active].url = url;
                }
            }
            PromptKind::Find => self.finish_find(accept, text, p.not_found),
            PromptKind::Dialog(id, kind) => {
                let text = (kind == DialogType::Prompt).then_some(text.as_str());
                self.view().respond_dialog(id, accept, text);
            }
            _ => {}
        }
        self.draw();
    }

    fn finish_find(&mut self, accept: bool, text: String, not_found: bool) {
        if !accept {
            self.find_generation += 1;
        }
        let script = format!(
            "window.__zwebFind?.finish({accept}, {})",
            serde_json::to_string(&text).unwrap()
        );
        self.view().evaluate(&script, |result| {
            if let Err(error) = result {
                eprintln!("webskiff: cannot finish page search: {error}");
            }
        });
        if !accept {
            return;
        }
        self.mode = Mode::Browse;
        if text.is_empty() {
            self.find_next(false);
        } else {
            self.find = text;
            self.message = if not_found {
                format!("not found: {}", self.find)
            } else {
                format!("/{}", self.find)
            };
        }
    }

    // ---- Drawing ----

    fn tab_labels(&self) -> Vec<String> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let name = if t.title.is_empty() { &t.url } else { &t.title };
                format!(
                    " {}{} {} ",
                    i + 1,
                    if t.loading { "*" } else { "" },
                    clip(name, 24)
                )
            })
            .collect()
    }

    pub fn draw(&self) {
        let (cols, rows) = terminal::size().unwrap_or((100, 30));
        let cols = cols as usize;
        let mut out = Vec::new();
        // Top: tabs.
        let _ = queue!(
            out,
            cursor::MoveTo(0, 0),
            SetBackgroundColor(Color::Rgb {
                r: 40,
                g: 40,
                b: 48
            })
        );
        let mut used = 0;
        for (i, label) in self.tab_labels().iter().enumerate() {
            let label = clip(label, cols.saturating_sub(used));
            if label.is_empty() {
                break;
            }
            used += label.width();
            if i == self.active {
                let _ = queue!(
                    out,
                    SetAttribute(Attribute::Reverse),
                    Print(&label),
                    SetAttribute(Attribute::NoReverse)
                );
            } else {
                let _ = queue!(
                    out,
                    SetForegroundColor(Color::Grey),
                    Print(&label),
                    SetForegroundColor(Color::Reset)
                );
            }
        }
        let _ = queue!(
            out,
            Print(" ".repeat(cols.saturating_sub(used))),
            ResetColor
        );
        // Bottom: mode, prompt or URL and message, frame stats.
        let row = rows.saturating_sub(1);
        let (tag, color) = self.status_tag();
        let _ = queue!(
            out,
            cursor::MoveTo(0, row),
            SetBackgroundColor(color),
            SetForegroundColor(Color::Black),
            Print(tag),
            ResetColor
        );
        let s = *self.renderer.stats.lock().unwrap();
        let stats = format!(
            " {}x{} {:.0}ms {}KB {}new/{}moved ",
            s.width,
            s.height,
            s.last_ms,
            s.last_bytes / 1024,
            s.sent,
            s.moved
        );
        let room = cols.saturating_sub(tag.len() + stats.len() + 1);
        let (body, dim) = match &self.prompt {
            Some(p) => p.body(room),
            None if self.mode == Mode::Hint && self.hint_filter => (
                format!(
                    " filter: {}  {} matches",
                    self.hint_text,
                    self.filtered().len()
                ),
                String::new(),
            ),
            None if self.mode == Mode::Hint => {
                (format!(" type a hint: {}", self.hint_typed), String::new())
            }
            None if !self.message.is_empty() => (
                format!(
                    " {}  {}",
                    self.message,
                    self.tabs.get(self.active).map_or("", |t| &t.url)
                ),
                String::new(),
            ),
            None => (
                format!(" {}", self.tabs.get(self.active).map_or("", |t| &t.url)),
                String::new(),
            ),
        };
        let body = clip(&body, room);
        let pad = room.saturating_sub(body.width() + dim.width());
        let _ = queue!(out, Print(&body));
        if !dim.is_empty() {
            let _ = queue!(
                out,
                SetForegroundColor(Color::DarkGrey),
                Print(&dim),
                SetForegroundColor(Color::Reset)
            );
        }
        let _ = queue!(
            out,
            Print(" ".repeat(pad + 1)),
            SetForegroundColor(Color::DarkGrey),
            Print(&stats),
            ResetColor
        );
        self.renderer.write(&out);
    }

    fn status_tag(&self) -> (&'static str, Color) {
        match (self.prompt.as_ref().map(|p| &p.kind), self.mode) {
            (Some(PromptKind::Find), _) => (" FIND ", Color::Cyan),
            (Some(PromptKind::Open), _) => (" OPEN ", Color::Cyan),
            (Some(PromptKind::OpenTab), _) => (" NEW TAB ", Color::Cyan),
            (Some(PromptKind::Dialog(..)), _) => (" DIALOG ", Color::Cyan),
            (_, Mode::Browse) => (" BROWSE ", Color::Blue),
            (_, Mode::Insert) => (" INSERT ", Color::Green),
            (_, Mode::Hint) => (" HINT ", Color::Yellow),
            (_, Mode::Visual) => (" VISUAL ", Color::Magenta),
        }
    }

    fn clear_page_area(&self) {
        let l = self.layout();
        let mut out = Vec::new();
        for r in 0..l.rows {
            let _ = queue!(
                out,
                cursor::MoveTo(0, l.top + r),
                terminal::Clear(terminal::ClearType::CurrentLine)
            );
        }
        self.renderer.write(&out);
    }

    fn show_help(&mut self) {
        self.leave_hint_or_visual();
        self.help = true;
        self.help_offset = 0;
        self.pending_key = None;
        self.renderer.layout.lock().unwrap().hidden = true;
        // The images go first; the help text is queued after them.
        self.renderer.clear_all();
        self.draw_help();
    }

    fn draw_help(&self) {
        self.clear_page_area();
        let l = self.layout();
        let mut out = Vec::new();
        for (i, line) in self
            .help_lines
            .iter()
            .skip(self.help_offset)
            .take(l.rows as usize)
            .enumerate()
        {
            let _ = queue!(
                out,
                cursor::MoveTo(2, l.top + i as u16),
                Print(clip(line, l.cols.saturating_sub(2) as usize))
            );
        }
        self.renderer.write(&out);
        self.draw();
    }

    fn help_key(&mut self, k: KeyEvent) {
        let rows = self.layout().rows as usize;
        let end = self.help_lines.len().saturating_sub(rows);
        match self.bindings.action(Scope::Help, k) {
            Some(Action::ScrollDown) => self.help_offset = (self.help_offset + 1).min(end),
            Some(Action::ScrollUp) => self.help_offset = self.help_offset.saturating_sub(1),
            Some(Action::PageDown) => self.help_offset = (self.help_offset + rows).min(end),
            Some(Action::PageUp) => self.help_offset = self.help_offset.saturating_sub(rows),
            Some(Action::Top) => self.help_offset = 0,
            Some(Action::Bottom) => self.help_offset = end,
            Some(Action::Cancel) => {
                self.help = false;
                self.schedule_prepare();
                self.renderer.layout.lock().unwrap().hidden = false;
                self.clear_page_area();
                self.view().request_frame();
                return self.draw();
            }
            _ => return,
        }
        self.draw_help();
    }
}

/// How many closed tabs can be reopened.
const CLOSED_MAX: usize = 10;

/// Remembers a closed tab's URL, newest last; drops the oldest past `CLOSED_MAX`.
fn push_closed(stack: &mut VecDeque<String>, url: String) {
    if url.is_empty() {
        return;
    }
    stack.push_back(url);
    if stack.len() > CLOSED_MAX {
        stack.pop_front();
    }
}

/// Resolves an address as a URL, an existing local file, a host, or a search.
fn load_history() -> History {
    let path = crate::data_dir().join("history.tsv");
    History::load(&path).unwrap_or_else(|error| {
        eprintln!("webskiff: cannot read history {}: {error}", path.display());
        History::default()
    })
}

pub fn to_url(text: &str) -> std::io::Result<String> {
    if text.contains("://")
        || text.starts_with("about:")
        || text.starts_with("data:")
        || text.starts_with("file:")
    {
        return Ok(text.into());
    }
    if let Some(url) = local_file_url(text)? {
        return Ok(url);
    }
    if !text.contains(' ') && (text.contains('.') || text.starts_with("localhost")) {
        Ok(format!("https://{text}"))
    } else {
        let q: String = text
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                b' ' => "+".into(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        Ok(format!("https://duckduckgo.com/?q={q}"))
    }
}

fn local_file_url(text: &str) -> std::io::Result<Option<String>> {
    let path = std::path::Path::new(text);
    let explicit = path.is_absolute() || text.starts_with("./") || text.starts_with("../");
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !explicit => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not a regular file",
        ));
    }
    let absolute = path.canonicalize()?;
    let encoded: String = absolute
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|&byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect();
    Ok(Some(format!("file://{encoded}")))
}

/// The page script for a Visual-mode move: extend the selection, then scroll
/// only if its end left the viewport.
pub fn visual_script(action: Action) -> Option<String> {
    let (dir, unit) = match action {
        Action::CharBackward => ("backward", "character"),
        Action::CharForward => ("forward", "character"),
        Action::LineForward => ("forward", "line"),
        Action::LineBackward => ("backward", "line"),
        Action::WordForward => ("forward", "word"),
        Action::WordBackward => ("backward", "word"),
        Action::LineStart => ("backward", "lineboundary"),
        Action::LineEnd => ("forward", "lineboundary"),
        _ => return None,
    };
    Some(format!(
        "getSelection().modify('extend','{dir}','{unit}');__zwebCaret.reveal()"
    ))
}

/// The terminal sequence that puts `text` on the clipboard.
fn osc52(text: &str) -> Vec<u8> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    format!("\x1b]52;c;{b64}\x07").into_bytes()
}

/// A yank's clipboard bytes, none if empty or too large, and its status message.
pub fn yank(text: &str) -> (Option<Vec<u8>>, String) {
    if text.is_empty() {
        return (None, "nothing selected".into());
    }
    let encoded = text.len().div_ceil(3) * 4;
    if encoded > YANK_MAX {
        return (
            None,
            format!("selection too large to copy ({encoded} bytes base64, max {YANK_MAX})"),
        );
    }
    (
        Some(osc52(text)),
        format!("copied {} chars", text.chars().count()),
    )
}

pub fn home() -> &'static str {
    HOME
}

fn display_char(c: char) -> char {
    if c.is_control() {
        '?'
    } else {
        c
    }
}

fn clip(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars().map(display_char) {
        let cw = c.width().unwrap_or(0);
        if w + cw > width {
            if w < width {
                out.push('…');
            }
            break;
        }
        w += cw;
        out.push(c);
    }
    out
}

fn clip_start(s: &str, width: usize) -> String {
    let mut out: Vec<char> = Vec::new();
    let mut w = 1;
    for c in s.chars().rev().map(display_char) {
        let cw = c.width().unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        out.push(c);
    }
    std::iter::once('…').chain(out.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests {
    use super::to_url;

    #[test]
    fn hint_filter_returns_original_indices_of_text_matches_ignoring_case() {
        let list = r#"[{"k":"1","t":"Home Page"},{"k":"2","t":"About"},{"k":"3","t":"HOMEWORK"},{"k":"4"}]"#;
        let (_, _, hints) = super::parse_hints(&serde_json::to_string(list).unwrap());
        let filter = |text, typed| super::filter_hints(&hints, text, typed);
        assert_eq!(filter("home", ""), [0, 2]);
        assert_eq!(filter("hOmE", ""), [0, 2]);
        assert_eq!(filter("e p", ""), [0]);
        assert_eq!(filter("zzz", ""), [] as [usize; 0]);
        assert_eq!(filter("", ""), [0, 1, 2, 3]);
        assert_eq!(filter("home", "3"), [2]);
    }

    fn filter_hints_of(list: &str) -> Vec<super::Hint> {
        super::parse_hints(&serde_json::to_string(list).unwrap()).2
    }

    #[test]
    fn filter_keys_typed_while_targets_load_apply_in_order_when_they_arrive() {
        use super::{FilterKey::*, FilterStep::*, HintFilter};
        let hints = filter_hints_of(
            r#"[{"k":"1","t":"link one"},{"k":"2","t":"lion"},{"k":"3","t":"link two"}]"#,
        );
        let mut f = HintFilter::default();
        for c in "lin".chars() {
            assert_eq!(f.key(&[], Char(c)), None);
        }
        assert_eq!(f.ready(&hints), Some(Show(vec![0, 2])));
        let mut f = HintFilter::default();
        for c in "lio".chars() {
            f.key(&[], Char(c));
        }
        assert_eq!(f.ready(&hints), Some(Pick(1)));
        // A queued Enter picks the first match once there are targets.
        let mut f = HintFilter::default();
        f.key(&[], Char('k'));
        f.key(&[], Accept);
        assert_eq!(f.ready(&hints), Some(Pick(0)));
        // No targets at all: a queued Enter picks nothing.
        let mut f = HintFilter::default();
        f.key(&[], Accept);
        assert_eq!(f.ready(&[]), None);
        // Nothing queued: nothing to do.
        assert_eq!(HintFilter::default().ready(&hints), None);
    }

    #[test]
    fn opens_existing_relative_and_absolute_files() {
        let path = std::fs::canonicalize("README.md").unwrap();
        let expected = format!("file://{}", path.display());
        assert_eq!(to_url("README.md").unwrap(), expected);
        assert_eq!(to_url("./README.md").unwrap(), expected);
        assert_eq!(to_url(path.to_str().unwrap()).unwrap(), expected);
    }

    #[test]
    fn escapes_local_filename_url_characters() {
        let path = std::env::temp_dir().join(format!(
            "webskiff-{}-read me #100%?\u{00e9}.html",
            std::process::id()
        ));
        std::fs::write(&path, "<title>local file</title>").unwrap();
        let absolute = std::fs::canonicalize(&path).unwrap();
        let actual = to_url(path.to_str().unwrap());
        let expected = format!("file://{}", absolute.display())
            .replace('%', "%25")
            .replace(' ', "%20")
            .replace('#', "%23")
            .replace('?', "%3F")
            .replace('\u{00e9}', "%C3%A9");
        std::fs::remove_file(path).unwrap();
        assert_eq!(actual.unwrap(), expected);
    }

    #[test]
    fn preserves_urls_hosts_and_searches() {
        for url in [
            "https://example.com/a?q=b",
            "file:///tmp/page.html",
            "about:blank",
            "data:text/html,hello",
        ] {
            assert_eq!(to_url(url).unwrap(), url);
        }
        assert_eq!(to_url("example.com").unwrap(), "https://example.com");
        assert_eq!(to_url("localhost:8080").unwrap(), "https://localhost:8080");
        assert_eq!(
            to_url("terminal browser").unwrap(),
            "https://duckduckgo.com/?q=terminal+browser"
        );
    }

    #[test]
    fn reports_missing_explicit_files_and_directories() {
        let missing =
            std::env::temp_dir().join(format!("webskiff-{}-missing.html", std::process::id()));
        assert!(!missing.exists());
        assert_eq!(
            to_url(missing.to_str().unwrap()).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            to_url(".").unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn page_dialogs_cannot_write_terminal_controls() {
        use super::{clip, clip_start, DialogType, Prompt, PromptKind};
        let payload = "\x1b]52;c;UFdORUQ=\x07\x1b[2J\r\n\t\u{009b}2J";
        let prompt = Prompt::new(
            PromptKind::Dialog(1, DialogType::Prompt),
            &format!("prompt: {payload}: "),
            payload,
        );
        for text in [
            prompt.body(200).0,
            prompt.body(18).0,
            clip(payload, 200),
            clip_start(payload, 12),
        ] {
            assert!(!text.chars().any(char::is_control), "{text:?}");
        }
        assert_eq!(clip("Browser / café", 30), "Browser / café");
    }

    #[test]
    fn visual_keys_extend_the_page_selection() {
        use super::visual_script;
        use crate::config::{Config, Scope};
        use crossterm::event::{KeyCode, KeyEvent};
        let config = Config::parse("").unwrap();
        for (key, dir, unit) in [
            ('h', "backward", "character"),
            ('l', "forward", "character"),
            ('j', "forward", "line"),
            ('k', "backward", "line"),
            ('w', "forward", "word"),
            ('b', "backward", "word"),
            ('0', "backward", "lineboundary"),
            ('$', "forward", "lineboundary"),
        ] {
            let action = config
                .action(Scope::Visual, KeyEvent::from(KeyCode::Char(key)))
                .unwrap();
            let js = visual_script(action).unwrap();
            assert!(
                js.contains(&format!("getSelection().modify('extend','{dir}','{unit}')")),
                "{key}: {js}"
            );
        }
    }

    #[test]
    fn closed_tabs_keep_the_ten_newest_and_reopen_newest_first() {
        use super::push_closed;
        let mut stack = std::collections::VecDeque::new();
        push_closed(&mut stack, String::new());
        assert!(stack.is_empty());
        for i in 1..=11 {
            push_closed(&mut stack, format!("https://example.com/{i}"));
        }
        assert_eq!(stack.len(), 10);
        for i in (2..=11).rev() {
            assert_eq!(stack.pop_back(), Some(format!("https://example.com/{i}")));
        }
        assert!(stack.is_empty());
    }

    #[test]
    fn yank_writes_osc52_up_to_100000_encoded_bytes() {
        use super::yank;
        let (bytes, status) = yank("hello");
        assert_eq!(bytes.as_deref(), Some(&b"\x1b]52;c;aGVsbG8=\x07"[..]));
        assert_eq!(status, "copied 5 chars");
        let (bytes, _) = yank(&"a".repeat(75_000));
        assert_eq!(
            bytes.map(|b| b.len()),
            Some("\x1b]52;c;".len() + 100_000 + 1)
        );
        let (bytes, status) = yank(&"a".repeat(75_001));
        assert_eq!(bytes, None);
        assert!(status.contains("too large"), "{status}");
    }

    fn two_pages() -> crate::history::History {
        let mut history = crate::history::History::default();
        history.visit("https://second.example/", 1_000);
        for _ in 0..3 {
            history.visit("https://first.example/", 1_000);
        }
        history
    }

    #[test]
    fn open_prompts_complete_from_history_with_tab_and_shift_tab() {
        use super::{Prompt, PromptKind};
        let history = two_pages();
        for kind in [PromptKind::Open, PromptKind::OpenTab] {
            let mut prompt = Prompt::new(kind, "open: ", "EXAMPLE");
            prompt.complete(&history, false, 1_000);
            assert_eq!(prompt.input.text, "https://first.example/");
            prompt.complete(&history, false, 1_000);
            assert_eq!(prompt.input.text, "https://second.example/");
            prompt.complete(&history, true, 1_000);
            assert_eq!(prompt.input.text, "https://first.example/");
            assert_eq!(prompt.input.cursor, prompt.input.text.len());
        }
        let mut find = Prompt::new(PromptKind::Find, "/", "example");
        find.complete(&history, false, 1_000);
        assert_eq!(find.input.text, "example");
    }

    #[test]
    fn open_prompts_show_the_best_match_dim_after_the_cursor() {
        use super::{Prompt, PromptKind};
        let history = two_pages();
        let mut prompt = Prompt::new(PromptKind::Open, "open: ", "https://fi");
        prompt.suggest(&history, 1_000);
        assert_eq!(
            prompt.body(200),
            (" open: https://fi█".to_string(), "rst.example/".to_string())
        );
        assert_eq!(prompt.body(22).1, "rst.");
        let mut other = Prompt::new(PromptKind::OpenTab, "new tab: ", "second");
        other.suggest(&history, 1_000);
        assert_eq!(other.body(200).1, "  → https://second.example/");
        let mut find = Prompt::new(PromptKind::Find, "/", "https://fi");
        find.suggest(&history, 1_000);
        assert_eq!(find.body(200).1, "");
    }

    #[test]
    fn yank_of_an_empty_selection_writes_nothing() {
        assert_eq!(super::yank(""), (None, "nothing selected".to_string()));
    }
}
