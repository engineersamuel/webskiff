//! The browser: tabs, modes, commands, and the two text rows around the page.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as TButton, MouseEvent, MouseEventKind};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::{cursor, queue, terminal};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use zebkit::offscreen::{Modifiers, MouseButton, MouseEvent as ZMouse, WheelPhase};
use zebkit::page::{Dialog, DialogPolicy, DialogType};
use zebkit::{LoadEvent, View};

use crate::keys;
use crate::render::{Layout, Renderer};

pub const HINTS_JS: &str = include_str!("hints.js");
const EDITABLE_JS: &str = "(e=>!!e&&(e.isContentEditable||e.tagName==='TEXTAREA'||(e.tagName==='INPUT'&&\
    !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(e.type))))(document.activeElement)";
const HOME: &str = "https://duckduckgo.com/";
/// Cell rows per scroll step (j, k, the wheel).
const STEP_ROWS: f32 = 3.0;

// ---- Messages: every callback queues one; `drain` handles them in order. ----

pub enum Msg {
    Term(Event),
    Load { tab: u64, event: LoadEvent, url: String },
    Title { tab: u64, title: String },
    Caret { tab: u64, on: bool },
    Popup { tab: u64, rect: Option<(i32, i32, i32, i32)> },
    WindowOpen { view: View },
    Dialog { tab: u64, dialog: Dialog },
    Hints { tab: u64, json: Result<String, String> },
    Found { found: bool },
    Focused { ok: bool },
    Extents { tab: u64, json: Option<String> },
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
    text: String,
}

struct Tab {
    id: u64,
    view: View,
    title: String,
    url: String,
    loading: bool,
    popup: bool,
}

struct Hint {
    key: String,
    x: f32,
    y: f32,
    /// Top-left corner, where the label goes.
    l: f32,
    t: f32,
    href: Option<String>,
    edit: bool,
}

pub struct Options {
    pub scale: Option<u32>,
    /// Images as flat placeholders of the same size (key I toggles).
    pub placeholders: bool,
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
    next_id: u64,
    mode: Mode,
    prompt: Option<Prompt>,
    hints: Vec<Hint>,
    hint_typed: String,
    hint_new_tab: bool,
    find: String,
    message: String,
    help: bool,
    pending_g: bool,
    zoom: f64,
    scale: u32,
    /// CSS pixels per cell row. The page is a whole number of rows tall and
    /// scrolls by whole rows, so moved rows are the same pixels, reused.
    row_css: f32,
    /// The active page's scrollY, innerHeight and scrollHeight, as last
    /// read and moved by predicted scrolls since.
    extents: Option<(f32, f32, f32)>,
    opts: Options,
    renderer: Arc<Renderer>,
    last_click: (Instant, f32, f32, u32),
    mouse: (f32, f32),
    last_stats: u64,
}

impl App {
    pub fn new(renderer: Arc<Renderer>, opts: Options) -> App {
        App {
            tabs: Vec::new(),
            active: 0,
            active_id: Rc::new(Cell::new(0)),
            next_id: 1,
            mode: Mode::Browse,
            prompt: None,
            hints: Vec::new(),
            hint_typed: String::new(),
            hint_new_tab: false,
            find: String::new(),
            message: "? for help".into(),
            help: false,
            pending_g: false,
            zoom: 1.0,
            scale: 1,
            row_css: 16.0,
            extents: None,
            opts,
            renderer,
            last_click: (Instant::now() - Duration::from_secs(10), 0.0, 0.0, 0),
            mouse: (0.0, 0.0),
            last_stats: 0,
        }
    }

    // ---- Layout ----

    /// Recomputes the page's size from the terminal's and applies it to every tab.
    pub fn relayout(&mut self) {
        let (cols, rows) = terminal::size().unwrap_or((100, 30));
        let px = terminal::window_size().ok();
        let page_rows = rows.saturating_sub(2).max(1);
        let (mut cell_w, mut cell_h) = (8.0, 16.0);
        let known = px.as_ref().is_some_and(|p| p.width > 0 && p.height > 0);
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
        self.scale = self.opts.scale.unwrap_or(if self.renderer.gfx == crate::render::Gfx::KittyShm { dpr as u32 } else { 1 });
        {
            let mut l = self.renderer.layout.lock().unwrap();
            let cell_px = if known { (cell_w.round() as u16, cell_h.round() as u16) } else { (0, 0) };
            *l = Layout { top: 1, cols, rows: page_rows, css_w, css_h, hidden: self.help, popup: l.popup, cell_px };
        }
        for t in &self.tabs {
            t.view.set_size(css_w, css_h, self.scale as f32);
        }
        self.renderer.clear_all();
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
        let y = (row - l.top) as f32 * l.css_h as f32 / l.rows as f32 + 0.5 * l.css_h as f32 / l.rows as f32;
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
            None => View::new_offscreen(None, l.css_w, l.css_h, self.scale as f32).expect("off-screen view"),
        };
        let id = self.next_id;
        self.next_id += 1;
        view.set_frame_rate(std::env::var("ZWEB_FPS").ok().and_then(|v| v.parse().ok()).unwrap_or(240.0));
        view.set_max_frames_in_flight(2);
        view.set_dialog_policy(DialogPolicy::Hold);
        if self.opts.placeholders {
            view.add_script(&format!("{PLACEHOLDER_JS}(true)"));
        }
        let (renderer, active) = (self.renderer.clone(), self.active_id.clone());
        let debug = std::env::var_os("ZWEB_DEBUG").is_some();
        view.on_frame(move |frame| {
            if debug {
                eprintln!("frame tab={id} seq={} layer={:?} {}x{} damage={:?}", frame.sequence(), frame.layer(), frame.width(), frame.height(),
                    frame.damage().iter().map(|r| (r.x, r.y, r.width, r.height)).collect::<Vec<_>>());
            }
            if active.get() == id {
                if crate::trace::on() {
                    let now = crate::trace::ticks_us();
                    crate::trace!("frame {} recv drawn {:.2} ms ago, ready {:.2} ms ago", frame.sequence(),
                        (now - frame.drawn_us()) as f64 / 1000.0, (now - frame.ready_us()) as f64 / 1000.0);
                }
                renderer.submit(frame);
            }
        });
        view.on_load(move |event, url| send(Msg::Load { tab: id, event, url: url.into() }));
        view.on_title(move |title| send(Msg::Title { tab: id, title: title.into() }));
        view.on_caret(move |rect| send(Msg::Caret { tab: id, on: rect.is_some() }));
        view.on_popup(move |rect| send(Msg::Popup { tab: id, rect: rect.map(|r| (r.x, r.y, r.width, r.height)) }));
        view.on_window_open(|view, _url| send(Msg::WindowOpen { view }));
        view.on_dialog(move |d| send(Msg::Dialog { tab: id, dialog: d.clone() }));
        if let Some(url) = url {
            view.load_url(url);
        }
        self.tabs.push(Tab { id, view, title: String::new(), url: url.unwrap_or("").into(), loading: url.is_some(), popup: false });
        self.switch(self.tabs.len() - 1);
    }

    fn switch(&mut self, index: usize) {
        if self.tabs.is_empty() {
            return;
        }
        let index = index.min(self.tabs.len() - 1);
        if let Some(old) = self.tabs.get(self.active) {
            old.view.set_focus(false);
        }
        self.active = index;
        self.mode = Mode::Browse;
        self.clear_hints();
        let tab = &self.tabs[index];
        self.active_id.set(tab.id);
        tab.view.set_focus(true);
        self.renderer.layout.lock().unwrap().popup = None;
        self.renderer.clear_popup();
        self.renderer.forget();
        tab.view.request_frame();
        self.extents = None;
        self.query_extents();
        self.draw();
    }

    fn close_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return self.quit();
        }
        let tab = self.tabs.remove(self.active);
        tab.view.close();
        self.switch(self.active.min(self.tabs.len() - 1));
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn view(&self) -> View {
        self.tab().view.clone()
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn quit(&mut self) {
        zebkit::quit();
    }

    // ---- Messages ----

    pub fn handle(&mut self, msg: Msg) {
        if let Msg::Term(_) = msg {
            crate::trace!("input handled");
        }
        match msg {
            Msg::Term(Event::Key(k)) if k.kind != KeyEventKind::Release => self.key(k),
            Msg::Term(Event::Mouse(m)) => self.mouse_event(m),
            Msg::Term(Event::Paste(text)) => self.paste(&text),
            Msg::Term(Event::Resize(..)) => self.relayout(),
            Msg::Term(_) => {}
            Msg::Load { tab, event, url } => {
                if let Some(i) = self.index_of(tab) {
                    let t = &mut self.tabs[i];
                    match event {
                        LoadEvent::Started => t.loading = true,
                        LoadEvent::Committed => {
                            // The empty page a view starts with is in its history: never stay there.
                            if url == "about:blank" && !t.url.is_empty() && t.url != "about:blank" {
                                t.view.go_forward();
                                if i == self.active {
                                    self.message = "no earlier page".into();
                                }
                                return self.draw();
                            }
                            t.url = url;
                            if i == self.active {
                                if self.mode == Mode::Hint {
                                    self.mode = Mode::Browse;
                                }
                                self.renderer.forget();
                                self.extents = None;
                            }
                        }
                        LoadEvent::Finished => {
                            t.loading = false;
                            if i == self.active {
                                self.query_extents();
                            }
                        }
                        LoadEvent::Failed => {
                            t.loading = false;
                            if i == self.active {
                                self.message = format!("load failed: {url}");
                            }
                        }
                    }
                    self.draw();
                }
            }
            Msg::Title { tab, title } => {
                if let Some(i) = self.index_of(tab) {
                    self.tabs[i].title = title;
                    self.draw();
                }
            }
            Msg::Caret { tab, on } => {
                // A caret soon after the user's click in an editable element: type
                // into it. A page's autofocus or a find's selection is no reason.
                if self.index_of(tab) != Some(self.active) || self.prompt.is_some() {
                    return;
                }
                if on && self.mode == Mode::Browse && self.last_click.0.elapsed() < Duration::from_millis(1500) {
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
            Msg::Popup { tab, rect } => {
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
            Msg::WindowOpen { view } => {
                self.open_tab(None, Some(view));
                self.message = "opened a new tab".into();
            }
            Msg::Dialog { tab, dialog } => {
                if let Some(i) = self.index_of(tab) {
                    if !dialog.held {
                        return;
                    }
                    self.switch(i);
                    let (label, text) = match dialog.kind {
                        DialogType::Alert => (format!("alert: {} [Enter]", dialog.message), String::new()),
                        DialogType::Confirm => (format!("confirm: {} [y/n]", dialog.message), String::new()),
                        DialogType::Prompt => (format!("prompt: {}: ", dialog.message), dialog.default_prompt.clone()),
                        DialogType::BeforeUnload => ("leave this page? [y/n]".into(), String::new()),
                        DialogType::Print => {
                            self.view().respond_dialog(dialog.id, false, None);
                            return;
                        }
                    };
                    self.prompt = Some(Prompt { kind: PromptKind::Dialog(dialog.id, dialog.kind), label, text });
                    self.draw();
                }
            }
            Msg::Hints { tab, json } => {
                crate::trace!("hints result");
                if self.index_of(tab) != Some(self.active) || self.mode != Mode::Hint {
                    return;
                }
                // A JSON string of the JSON list (hints.js).
                let list: Vec<serde_json::Value> = json
                    .ok()
                    .and_then(|j| serde_json::from_str::<String>(&j).ok())
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default();
                self.hints = list
                    .iter()
                    .map(|h| Hint {
                        key: h["k"].as_str().unwrap_or("").into(),
                        x: h["x"].as_f64().unwrap_or(0.0) as f32,
                        y: h["y"].as_f64().unwrap_or(0.0) as f32,
                        l: h["l"].as_f64().unwrap_or(0.0) as f32,
                        t: h["t"].as_f64().unwrap_or(0.0) as f32,
                        href: h["href"].as_str().map(Into::into),
                        edit: h["edit"].as_bool().unwrap_or(false),
                    })
                    .collect();
                if self.hints.is_empty() {
                    self.mode = Mode::Browse;
                    self.message = "no links here".into();
                }
                self.show_hints("");
                self.draw();
            }
            Msg::Found { found } => {
                self.message = if found { format!("/{}", self.find) } else { format!("not found: {}", self.find) };
                self.draw();
            }
            Msg::Extents { tab, json } => {
                if self.index_of(tab) == Some(self.active) {
                    let v: Option<Vec<f32>> = json.and_then(|j| serde_json::from_str(&j).ok());
                    self.extents = v.filter(|v| v.len() == 3).map(|v| (v[0], v[1], v[2]));
                }
            }
            Msg::Focused { ok } => {
                if ok {
                    self.mode = Mode::Insert;
                } else {
                    self.message = "no text field here".into();
                }
                self.draw();
            }
            Msg::Tick => {
                let frames = self.renderer.stats.lock().unwrap().frames;
                if frames != self.last_stats {
                    self.last_stats = frames;
                    self.draw();
                }
            }
        }
    }

    // ---- Keys ----

    fn key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('q') {
            return self.quit();
        }
        if self.help {
            self.help = false;
            self.renderer.layout.lock().unwrap().hidden = false;
            self.clear_page_area();
            self.view().request_frame();
            return self.draw();
        }
        if self.prompt.is_some() {
            return self.prompt_key(k);
        }
        // An open <select> list takes every key.
        if self.tab().popup {
            return self.forward(&k);
        }
        match self.mode {
            Mode::Insert => {
                if k.code == KeyCode::Esc {
                    self.mode = Mode::Browse;
                    self.view().evaluate("document.activeElement && document.activeElement.blur()", |_| {});
                    self.draw();
                } else {
                    self.forward(&k);
                }
            }
            Mode::Hint => self.hint_key(k),
            Mode::Browse => self.command(k),
        }
    }

    fn forward(&self, k: &KeyEvent) {
        if let Some(dom) = keys::to_dom(k) {
            keys::send(&self.tab().view, &dom);
        }
    }

    fn command(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let view = self.view();
        let l = self.layout();
        let line = STEP_ROWS * self.row_css;
        let half = (l.rows / 2) as f32 * self.row_css;
        let page = l.rows.saturating_sub(2).max(1) as f32 * self.row_css;
        let g = std::mem::take(&mut self.pending_g);
        self.message.clear();
        match k.code {
            KeyCode::Char('c') if ctrl => return self.quit(),
            KeyCode::Char('d') if ctrl => self.wheel(0.0, half),
            KeyCode::Char('u') if ctrl => self.wheel(0.0, -half),
            KeyCode::Char('q') => return self.quit(),
            KeyCode::Char('j') | KeyCode::Down => self.wheel(0.0, line),
            KeyCode::Char('k') | KeyCode::Up => self.wheel(0.0, -line),
            KeyCode::Char('h') | KeyCode::Left => self.wheel(-line, 0.0),
            KeyCode::Char('l') | KeyCode::Right => self.wheel(line, 0.0),
            KeyCode::Char('d') => self.wheel(0.0, half),
            KeyCode::Char('u') => self.wheel(0.0, -half),
            KeyCode::Char(' ') | KeyCode::PageDown => self.wheel(0.0, page),
            KeyCode::Char('b') | KeyCode::PageUp => self.wheel(0.0, -page),
            KeyCode::Char('g') if g => view.evaluate("scrollTo(0,0)", |_| {}),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Home => view.evaluate("scrollTo(0,0)", |_| {}),
            KeyCode::Char('G') | KeyCode::End => {
                view.evaluate("scrollTo(0,document.scrollingElement.scrollHeight)", |_| {})
            }
            KeyCode::Char('i') if g => {
                let focus = "(()=>{const e=[...document.querySelectorAll('input:not([type=hidden]),textarea,[contenteditable]')]\
                    .find(e=>e.getClientRects().length);if(e){e.focus();return true}return false})()";
                view.evaluate(focus, |r| send(Msg::Focused { ok: r.as_deref() == Ok("true") }));
            }
            KeyCode::Char('i') => self.mode = Mode::Insert,
            KeyCode::Char('H') => view.go_back(),
            KeyCode::Char('L') => view.go_forward(),
            KeyCode::Char('r') => view.reload(),
            KeyCode::Char('s') => view.stop(),
            KeyCode::Char('o') => self.prompt(PromptKind::Open, "open: ", ""),
            KeyCode::Char('O') => {
                let url = self.tab().url.clone();
                self.prompt(PromptKind::Open, "open: ", &url)
            }
            KeyCode::Char('t') => self.prompt(PromptKind::OpenTab, "new tab: ", ""),
            KeyCode::Char('x') => return self.close_tab(),
            KeyCode::Char('K') | KeyCode::Tab => self.switch((self.active + 1) % self.tabs.len()),
            KeyCode::Char('J') | KeyCode::BackTab => self.switch((self.active + self.tabs.len() - 1) % self.tabs.len()),
            KeyCode::Char(c @ '1'..='9') => self.switch(c as usize - '1' as usize),
            KeyCode::Char('f') => self.start_hints(false),
            KeyCode::Char('F') => self.start_hints(true),
            KeyCode::Char('/') => self.prompt(PromptKind::Find, "/", ""),
            KeyCode::Char('n') => self.find_next(false),
            KeyCode::Char('N') => self.find_next(true),
            KeyCode::Char('+') | KeyCode::Char('=') => self.set_zoom(self.zoom * 1.1),
            KeyCode::Char('-') => self.set_zoom(self.zoom / 1.1),
            KeyCode::Char('0') => self.set_zoom(1.0),
            KeyCode::Char('y') => {
                let url = self.tab().url.clone();
                let b64 = base64::engine::general_purpose::STANDARD.encode(&url);
                self.renderer.write(format!("\x1b]52;c;{b64}\x07").as_bytes());
                self.message = format!("copied {url}");
            }
            KeyCode::Char('I') => {
                self.opts.placeholders = !self.opts.placeholders;
                // Preloads cannot be removed: each toggle adds one, the last wins.
                let js = format!("{PLACEHOLDER_JS}({})", self.opts.placeholders);
                for t in &self.tabs {
                    t.view.add_script(&js);
                    t.view.evaluate(&js, |_| {});
                }
                self.message = if self.opts.placeholders { "images: placeholders" } else { "images: shown" }.into();
            }
            KeyCode::Char('?') => return self.show_help(),
            // Escape and the rest of the keys the page may use.
            KeyCode::Esc | KeyCode::Enter => self.forward(&k),
            _ => {}
        }
        self.draw();
    }

    fn wheel(&mut self, dx: f32, dy: f32) {
        let l = self.layout();
        let (x, y) = if self.mouse == (0.0, 0.0) { (l.css_w as f32 / 2.0, l.css_h as f32 / 2.0) } else { self.mouse };
        self.scroll_at(x, y, dx, dy, Modifiers::NONE);
    }

    /// A wheel event; a small vertical step of the page is shown at once
    /// from the rows the terminal holds (render.rs, `predict`).
    fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32, mods: Modifiers) {
        if self.mode == Mode::Hint {
            self.mode = Mode::Browse;
            self.clear_hints();
            self.draw();
        }
        if dx == 0.0 {
            self.predict(dy);
        }
        self.view().send_wheel(x, y, dx, dy, false, WheelPhase::None, mods);
        crate::trace!("wheel sent {dy} at {}", crate::trace::ticks_us());
        self.query_extents();
    }

    fn predict(&mut self, dy: f32) {
        let l = self.layout();
        let k = (dy / self.row_css).round() as i32;
        // Any step: the renderer shows it only when it knows the rows it brings.
        let small = k != 0 && k.unsigned_abs() < l.rows as u32;
        let browsing = self.mode != Mode::Hint && !self.help && self.prompt.is_none() && !self.tab().popup;
        // Only where the page itself scrolls the whole step.
        let Some((sy, ih, sh)) = self.extents else { return };
        let room = if dy > 0.0 { sh - ih - sy } else { sy };
        let off = std::env::var_os("ZWEB_NOPREDICT").is_some();
        if small && browsing && !off && room >= dy.abs() && self.renderer.gfx != crate::render::Gfx::Blocks {
            // Shown only while predictions for this page are right (render.rs).
            self.renderer.predict(k);
            self.extents = Some((sy + dy, ih, sh));
        }
    }

    fn query_extents(&self) {
        let Some(t) = self.tabs.get(self.active) else { return };
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
            return;
        }
        let script = format!("window.find({}, false, {back}, true)", serde_json::to_string(&self.find).unwrap());
        self.view().evaluate(&script, |r| send(Msg::Found { found: r.as_deref() == Ok("true") }));
    }

    // ---- Hints ----

    fn start_hints(&mut self, new_tab: bool) {
        self.mode = Mode::Hint;
        self.hint_new_tab = new_tab;
        self.hint_typed.clear();
        self.hints.clear();
        let id = self.tab().id;
        self.view().evaluate(HINTS_JS, move |r| send(Msg::Hints { tab: id, json: r.map_err(|e| e.to_string()) }));
    }

    fn clear_hints(&mut self) {
        let had = !self.hints.is_empty();
        self.hints.clear();
        self.hint_typed.clear();
        if had {
            self.renderer.hints(Vec::new());
            if let Some(t) = self.tabs.get(self.active) {
                // Blocks mode wrote the labels into page cells.
                t.view.request_frame();
            }
        }
    }

    /// Labels of the hints that start with `typed`, at their elements' cells.
    fn show_hints(&self, typed: &str) {
        let l = self.layout();
        let (cw, ch) = (l.css_w as f32 / l.cols as f32, l.css_h as f32 / l.rows as f32);
        let mut labels: Vec<(u16, u16, String)> = self
            .hints
            .iter()
            .filter(|h| h.key.starts_with(typed))
            .map(|h| {
                let row = ((h.t / ch) as u16).min(l.rows.saturating_sub(1));
                ((h.l / cw) as u16, row, h.key[typed.len()..].to_string())
            })
            .collect();
        // Labels never cover each other: one in the way moves right.
        labels.sort_by_key(|(c, r, _)| (*r, *c));
        let mut free_from = vec![0u16; l.rows as usize];
        for (col, row, k) in labels.iter_mut() {
            let w = k.chars().count() as u16;
            *col = (*col).max(free_from[*row as usize]).min(l.cols.saturating_sub(w));
            free_from[*row as usize] = *col + w + 1;
        }
        self.renderer.hints(labels);
    }

    fn hint_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.clear_hints();
            }
            KeyCode::Backspace => {
                self.hint_typed.pop();
            }
            KeyCode::Char(c) => self.hint_typed.push(c.to_ascii_lowercase()),
            _ => {}
        }
        if self.mode != Mode::Hint {
            return self.draw();
        }
        let typed = self.hint_typed.clone();
        let matches: Vec<usize> = (0..self.hints.len()).filter(|&i| self.hints[i].key.starts_with(&typed)).collect();
        if matches.is_empty() && !self.hints.is_empty() {
            self.hint_typed.pop();
        } else if matches.len() == 1 && self.hints[matches[0]].key == typed {
            let h = &self.hints[matches[0]];
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
        } else {
            self.show_hints(&self.hint_typed);
        }
        self.draw();
    }

    // ---- Mouse ----

    fn click(&mut self, x: f32, y: f32, count: u32) {
        let v = self.view();
        v.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, Modifiers::NONE);
        v.send_mouse(ZMouse::Down, x, y, MouseButton::Left, count, Modifiers::NONE);
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
        let Some((x, y)) = self.to_css(m.column, m.row) else { return };
        self.mouse = (x, y);
        let mut mods = Modifiers::NONE;
        if m.modifiers.contains(KeyModifiers::SHIFT) {
            mods = mods | Modifiers::SHIFT;
        }
        if m.modifiers.contains(KeyModifiers::CONTROL) {
            mods = mods | Modifiers::CONTROL;
        }
        if m.modifiers.contains(KeyModifiers::ALT) {
            mods = mods | Modifiers::ALT;
        }
        let button = |b: TButton| match b {
            TButton::Left => MouseButton::Left,
            TButton::Right => MouseButton::Right,
            TButton::Middle => MouseButton::Middle,
        };
        let v = self.view();
        let line = STEP_ROWS * self.row_css;
        match m.kind {
            MouseEventKind::ScrollDown => return self.scroll_at(x, y, 0.0, line, mods),
            MouseEventKind::ScrollUp => return self.scroll_at(x, y, 0.0, -line, mods),
            MouseEventKind::Down(b) => {
                if self.mode == Mode::Hint {
                    self.mode = Mode::Browse;
                    self.clear_hints();
                }
                let (at, lx, ly, n) = self.last_click;
                let count = if at.elapsed() < Duration::from_millis(400) && (lx - x).abs() < 4.0 && (ly - y).abs() < 4.0 {
                    n + 1
                } else {
                    1
                };
                self.last_click = (Instant::now(), x, y, count);
                v.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, mods);
                v.send_mouse(ZMouse::Down, x, y, button(b), count, mods);
            }
            MouseEventKind::Up(b) => v.send_mouse(ZMouse::Up, x, y, button(b), self.last_click.3, mods),
            MouseEventKind::Drag(_) => v.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, mods | Modifiers::LEFT_BUTTON),
            MouseEventKind::Moved => v.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, mods),
            MouseEventKind::ScrollRight => v.send_wheel(x, y, line, 0.0, false, WheelPhase::None, mods),
            MouseEventKind::ScrollLeft => v.send_wheel(x, y, -line, 0.0, false, WheelPhase::None, mods),
        }
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
            p.text.push_str(text.trim_end_matches(['\r', '\n']));
            return self.draw();
        }
        if self.mode == Mode::Insert || self.mode == Mode::Browse {
            self.view().ime_commit(text);
        }
    }

    // ---- Prompt ----

    fn prompt(&mut self, kind: PromptKind, label: &str, text: &str) {
        self.prompt = Some(Prompt { kind, label: label.into(), text: text.into() });
    }

    fn prompt_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let p = self.prompt.as_mut().unwrap();
        let yes_no = matches!(p.kind, PromptKind::Dialog(_, DialogType::Confirm | DialogType::BeforeUnload));
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') if yes_no => return self.prompt_done(true),
            KeyCode::Char('n') | KeyCode::Char('N') if yes_no => return self.prompt_done(false),
            KeyCode::Esc => return self.prompt_done(false),
            KeyCode::Enter => return self.prompt_done(true),
            KeyCode::Backspace => {
                p.text.pop();
            }
            KeyCode::Char('u') if ctrl => p.text.clear(),
            KeyCode::Char('w') if ctrl => {
                let t = p.text.trim_end();
                let cut = t.rfind([' ', '/', '.']).map_or(0, |i| i + 1);
                p.text.truncate(cut);
            }
            KeyCode::Char(c) if !ctrl && !yes_no => p.text.push(c),
            _ => {}
        }
        self.draw();
    }

    fn prompt_done(&mut self, accept: bool) {
        let p = self.prompt.take().unwrap();
        match p.kind {
            PromptKind::Open | PromptKind::OpenTab if accept && !p.text.trim().is_empty() => {
                let url = to_url(p.text.trim());
                if p.kind == PromptKind::OpenTab {
                    self.open_tab(Some(&url), None);
                } else {
                    self.tab().view.load_url(&url);
                    self.tabs[self.active].url = url;
                }
            }
            PromptKind::Find if accept => {
                self.find = p.text;
                self.find_next(false);
            }
            PromptKind::Dialog(id, kind) => {
                let text = (kind == DialogType::Prompt).then_some(p.text.as_str());
                self.view().respond_dialog(id, accept, text);
            }
            _ => {}
        }
        self.draw();
    }

    // ---- Drawing ----

    fn tab_labels(&self) -> Vec<String> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let name = if t.title.is_empty() { &t.url } else { &t.title };
                format!(" {}{} {} ", i + 1, if t.loading { "*" } else { "" }, clip(name, 24))
            })
            .collect()
    }

    pub fn draw(&self) {
        let (cols, rows) = terminal::size().unwrap_or((100, 30));
        let cols = cols as usize;
        let mut out = Vec::new();
        // Top: tabs.
        let _ = queue!(out, cursor::MoveTo(0, 0), SetBackgroundColor(Color::Rgb { r: 40, g: 40, b: 48 }));
        let mut used = 0;
        for (i, label) in self.tab_labels().iter().enumerate() {
            let label = clip(label, cols.saturating_sub(used));
            if label.is_empty() {
                break;
            }
            used += label.width();
            if i == self.active {
                let _ = queue!(out, SetAttribute(Attribute::Reverse), Print(&label), SetAttribute(Attribute::NoReverse));
            } else {
                let _ = queue!(out, SetForegroundColor(Color::Grey), Print(&label), SetForegroundColor(Color::Reset));
            }
        }
        let _ = queue!(out, Print(" ".repeat(cols.saturating_sub(used))), ResetColor);
        // Bottom: mode, prompt or URL and message, frame stats.
        let row = rows.saturating_sub(1);
        let (tag, color) = match (self.prompt.is_some(), self.mode) {
            (true, _) => (" CMD ", Color::Magenta),
            (_, Mode::Browse) => (" BROWSE ", Color::Blue),
            (_, Mode::Insert) => (" INSERT ", Color::Green),
            (_, Mode::Hint) => (" HINT ", Color::Yellow),
        };
        let _ = queue!(out, cursor::MoveTo(0, row), SetBackgroundColor(color), SetForegroundColor(Color::Black), Print(tag), ResetColor);
        let s = *self.renderer.stats.lock().unwrap();
        let stats = format!(" {}x{} {:.0}ms {}KB {}new/{}moved ", s.width, s.height, s.last_ms, s.last_bytes / 1024, s.sent, s.moved);
        let room = cols.saturating_sub(tag.len() + stats.len() + 1);
        let body = match &self.prompt {
            Some(p) => format!(" {}{}█", p.label, p.text),
            None if self.mode == Mode::Hint => format!(" type a hint: {}", self.hint_typed),
            None if !self.message.is_empty() => format!(" {}  {}", self.message, self.tabs.get(self.active).map_or("", |t| &t.url)),
            None => format!(" {}", self.tabs.get(self.active).map_or("", |t| &t.url)),
        };
        // A long prompt shows its end, where the typing is.
        let body = if self.prompt.is_some() && body.width() > room { clip_start(&body, room) } else { clip(&body, room) };
        let pad = room.saturating_sub(body.width());
        let _ = queue!(
            out,
            Print(&body),
            Print(" ".repeat(pad + 1)),
            SetForegroundColor(Color::DarkGrey),
            Print(&stats),
            ResetColor
        );
        self.renderer.write(&out);
    }

    fn clear_page_area(&self) {
        let l = self.layout();
        let mut out = Vec::new();
        for r in 0..l.rows {
            let _ = queue!(out, cursor::MoveTo(0, l.top + r), terminal::Clear(terminal::ClearType::CurrentLine));
        }
        self.renderer.write(&out);
    }

    fn show_help(&mut self) {
        self.help = true;
        self.renderer.layout.lock().unwrap().hidden = true;
        // The images go first; the help text is queued after them.
        self.renderer.clear_all();
        self.clear_page_area();
        let l = self.layout();
        let mut out = Vec::new();
        for (i, line) in HELP.lines().enumerate().take(l.rows as usize) {
            let _ = queue!(out, cursor::MoveTo(2, l.top + i as u16), Print(line));
        }
        self.renderer.write(&out);
        self.draw();
    }
}

const HELP: &str = "\
zweb: a web browser in the terminal (zebkit / Chromium 157)          any key closes this

 BROWSE mode                                   INSERT mode (typing into the page)
  j k h l  arrows   scroll                       Esc        back to BROWSE
  d u  Ctrl-d Ctrl-u  half page                  Ctrl-A C X V Z  select all, copy, cut, paste, undo
  Space b  PgDn PgUp  page                       a text field you click or focus starts INSERT
  gg G     top, bottom
  f        link hints, click                   HINT mode
  F        link hints, open in new tab           type the yellow letters; Esc cancels
  gi       focus first text field
  i        INSERT mode                         PROMPT (bottom line)
  o  O     open URL (O: edit current)            Enter go, Esc cancel, Ctrl-U clear, Ctrl-W word
  t        open URL in new tab                   text without a dot is a DuckDuckGo search
  x        close tab
  K J  Tab   next, previous tab   1-9 tab N    MOUSE
  H L      back, forward                         click, drag, wheel go to the page
  r  s     reload, stop                          click a tab on the top line to switch
  /  n N   find, next, previous
  + - 0    zoom in, out, reset
  y        copy URL (OSC 52)
  I        images: show, or flat placeholders (faster)
  q  Ctrl-Q  quit (Ctrl-Q works in every mode)";

/// Text typed in the prompt as a URL: as is with a scheme, https for a host, else a search.
pub fn to_url(text: &str) -> String {
    if text.contains("://") || text.starts_with("about:") || text.starts_with("data:") || text.starts_with("file:") {
        text.into()
    } else if !text.contains(' ') && (text.contains('.') || text.starts_with("localhost")) {
        format!("https://{text}")
    } else {
        let q: String = text
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                b' ' => "+".into(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        format!("https://duckduckgo.com/?q={q}")
    }
}

pub fn home() -> &'static str {
    HOME
}

fn clip(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width {
            if width > 0 && w + 1 <= width {
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
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        out.push(c);
    }
    std::iter::once('…').chain(out.into_iter().rev()).collect()
}
