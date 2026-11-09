//! Draws page frames in the terminal, on a thread of its own.
//!
//! Kitty graphics: raw RGB, zlib-compressed and base64 in the escape codes
//! (works everywhere, also over SSH), or through POSIX shared memory when the
//! terminal says it reads it (local only, nearly no bytes on the pipe).
//! Blocks: `▀` cells with truecolor, for terminals without graphics.
//! Either way only cell rows whose pixels changed are drawn.

use std::ffi::{c_void, CString};
use std::io::Write;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use flate2::{write::ZlibEncoder, Compression};
use zebkit::offscreen::{Frame, FrameBuffer, Layer};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gfx {
    KittyDirect,
    KittyShm,
    Blocks,
}

/// Where the page is on screen, and its size in CSS pixels.
#[derive(Clone, Copy, Default)]
pub struct Layout {
    pub top: u16,
    pub cols: u16,
    pub rows: u16,
    pub css_w: u32,
    pub css_h: u32,
    /// Page not shown (help screen).
    pub hidden: bool,
    /// The open popup's place in CSS pixels.
    pub popup: Option<(i32, i32, i32, i32)>,
}

#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub frames: u64,
    pub last_ms: f64,
    pub last_bytes: usize,
    /// Rows of the last frame sent as new images, and rows only placed again.
    pub sent: u16,
    pub moved: u16,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
struct Pending {
    page: Option<Frame>,
    popup: Option<Frame>,
    /// Text for the terminal (tab bar, status line, help), written in order.
    text: Vec<u8>,
    /// Rows to scroll at once from the terminal's own images (down > 0).
    predict: i32,
    /// Forget what was learned of the page (new page, other tab).
    forget: bool,
    clear_popup: bool,
    clear_all: bool,
    stop: bool,
}

pub struct Renderer {
    pub gfx: Gfx,
    pending: Mutex<Pending>,
    wake: Condvar,
    pub layout: Mutex<Layout>,
    pub stats: Mutex<Stats>,
    /// Predictions for this page have been right: keep making them.
    pub predict_ok: AtomicBool,
}

const POPUP_ID: u32 = 1;
/// A 1x1 image of the page's background, stretched over rows not known yet.
const BG_ID: u32 = 2;
/// Not the hash of any row: a row shown as background.
const BLANK: u64 = u64::MAX;
/// Raw pixel bytes of row images the terminal may keep for reuse.
const CACHE_BYTES: usize = 64 << 20;
/// How long frames that do not show the whole scroll yet are held back.
const SETTLE: Duration = Duration::from_millis(150);
/// Terminal columns per tile, about.
const TILE_COLS: usize = 16;

struct Cached {
    id: u32,
    used: u64,
    size: usize,
}

/// A scroll shown before the page's frame for it came.
struct Prediction {
    /// Rows as they were before it.
    base: Vec<u64>,
    /// Total shift after each step (rows, down > 0).
    steps: Vec<i32>,
    deadline: Instant,
    /// The newest frame held back, drawn if no better one comes.
    held: Option<Frame>,
    /// Shown on screen; else only checked against the page's frames, to
    /// learn whether predictions are right again.
    shown: bool,
}

/// What the terminal shows and holds: one image per cell row, kept by the
/// hash of its pixels. A row that only moved (scrolling by whole rows) is
/// placed again from the terminal's copy, not sent again.
struct Rows {
    key: Option<(u32, u32, u16, u16, u16)>,
    shown: Vec<Option<u64>>,
    cache: HashMap<u64, Cached>,
    bytes: usize,
    next_id: u32,
    clock: u64,
    /// Tiles per cell row: a fixed sidebar is learned apart from the text
    /// that scrolls beside it. Tile `t` is row `t / bands`, band `t % bands`.
    bands: usize,
    /// Tiles that stayed in place when the page scrolled (a fixed header).
    fixed: Vec<bool>,
    prediction: Option<Prediction>,
    /// Background colour of the last frame, and the one sent as BG_ID.
    bg: [u8; 3],
    bg_sent: Option<[u8; 3]>,
    misses: u32,
    /// Right predictions in a row while not shown.
    hits: u32,
}

impl Rows {
    fn new() -> Rows {
        Rows {
            key: None,
            shown: Vec::new(),
            cache: HashMap::new(),
            bytes: 0,
            next_id: 10,
            clock: 0,
            bands: 1,
            fixed: Vec::new(),
            prediction: None,
            bg: [255; 3],
            bg_sent: None,
            misses: 0,
            hits: 0,
        }
    }

    /// Forgets everything (the terminal's images were all deleted).
    fn reset(&mut self) {
        self.key = None;
        self.shown.clear();
        self.cache.clear();
        self.bytes = 0;
        self.prediction = None;
        self.bg_sent = None;
    }

    /// The cell row, first column and width in columns of tile `t`.
    fn cell(&self, layout: &Layout, t: usize) -> (u16, u16, u16) {
        let (r, b) = (t / self.bands, t % self.bands);
        let col = |b: usize| (b * layout.cols as usize / self.bands) as u16;
        (r as u16, col(b), col(b + 1) - col(b))
    }

    /// Puts image `hash` (a cached tile, or BLANK) at tile `t`, then removes
    /// what was there. Returns whether a placement was written.
    fn place(&mut self, out: &mut Vec<u8>, layout: &Layout, t: usize, hash: u64) -> bool {
        let old = self.shown[t];
        if old == Some(hash) {
            return false;
        }
        let id = if hash == BLANK {
            BG_ID
        } else {
            let Some(c) = self.cache.get_mut(&hash) else { return false };
            self.clock += 1;
            c.used = self.clock;
            c.id
        };
        let (r, col, cols) = self.cell(layout, t);
        out.extend_from_slice(
            format!("\x1b[{};{}H\x1b_Ga=p,i={id},p={},c={cols},r=1,C=1,q=2\x1b\\", layout.top + r + 1, col + 1, t + 1).as_bytes(),
        );
        self.unplace(out, t, old);
        self.shown[t] = Some(hash);
        true
    }

    /// Removes the placement of `old` at tile `t`; the image stays for reuse.
    fn unplace(&self, out: &mut Vec<u8>, t: usize, old: Option<u64>) {
        let id = match old {
            Some(BLANK) => Some(BG_ID),
            Some(h) => self.cache.get(&h).map(|c| c.id),
            None => None,
        };
        if let Some(id) = id {
            out.extend_from_slice(format!("\x1b_Ga=d,d=i,i={id},p={},q=2\x1b\\", t + 1).as_bytes());
        }
    }
}

/// Tiles of `base` scrolled by `shift` rows (down > 0), fixed tiles kept,
/// BLANK where nothing is known.
fn shifted(base: &[u64], fixed: &[bool], bands: usize, shift: i32) -> Vec<u64> {
    (0..base.len())
        .map(|t| {
            if fixed[t] {
                return base[t];
            }
            let from = t as i64 + shift as i64 * bands as i64;
            // A fixed tile does not move into another place: not known there.
            let known = from >= 0 && (from as usize) < base.len() && !fixed[from as usize];
            if known { base[from as usize] } else { BLANK }
        })
        .collect()
}

/// How many rows of `real` are the rows of `want` (BLANK rows not counted).
fn matches(real: &[u64], want: &[u64]) -> usize {
    real.iter().zip(want).filter(|(a, b)| **b != BLANK && a == b).count()
}

impl Renderer {
    pub fn new(gfx: Gfx) -> Renderer {
        Renderer {
            gfx,
            pending: Mutex::default(),
            wake: Condvar::new(),
            layout: Mutex::default(),
            stats: Mutex::default(),
            predict_ok: AtomicBool::new(true),
        }
    }

    /// Keeps only the newest frame of each layer; an older one is released.
    pub fn submit(&self, frame: Frame) {
        let mut p = self.pending.lock().unwrap();
        match frame.layer() {
            Layer::View => p.page = Some(frame),
            Layer::Popup => p.popup = Some(frame),
        }
        self.wake.notify_one();
    }

    /// Queues text for the terminal: the engine's thread never waits on it.
    pub fn write(&self, bytes: &[u8]) {
        self.pending.lock().unwrap().text.extend_from_slice(bytes);
        self.wake.notify_one();
    }

    /// Scrolls the rows on screen by `rows` at once (down > 0), before the
    /// page's frame comes; that frame corrects what was not right.
    pub fn predict(&self, rows: i32) {
        self.pending.lock().unwrap().predict += rows;
        self.wake.notify_one();
    }

    /// A new page or tab: what was learned of the last one does not hold.
    pub fn forget(&self) {
        self.predict_ok.store(true, Ordering::Relaxed);
        self.pending.lock().unwrap().forget = true;
        self.wake.notify_one();
    }

    pub fn clear_popup(&self) {
        self.pending.lock().unwrap().clear_popup = true;
        self.wake.notify_one();
    }

    /// Removes every image (before the help screen, a resize, quitting); the
    /// next page frame is drawn whole. Text queued after this goes after it.
    pub fn clear_all(&self) {
        let mut p = self.pending.lock().unwrap();
        p.clear_all = true;
        p.page = None;
        p.popup = None;
        p.text.clear();
        p.predict = 0;
        self.wake.notify_one();
    }

    pub fn stop(&self) {
        self.pending.lock().unwrap().stop = true;
        self.wake.notify_one();
    }

    pub fn run(&self) {
        let mut rows = Rows::new();
        loop {
            let mut p = self.pending.lock().unwrap();
            loop {
                let busy = p.page.is_some()
                    || p.popup.is_some()
                    || !p.text.is_empty()
                    || p.predict != 0
                    || p.clear_popup
                    || p.clear_all
                    || p.forget
                    || p.stop;
                if busy {
                    break;
                }
                match &rows.prediction {
                    Some(pr) => {
                        let now = Instant::now();
                        if now >= pr.deadline {
                            break;
                        }
                        p = self.wake.wait_timeout(p, pr.deadline - now).unwrap().0;
                    }
                    None => p = self.wake.wait(p).unwrap(),
                }
            }
            let layout = *self.layout.lock().unwrap();
            if layout.hidden {
                p.page = None;
                p.popup = None;
                p.predict = 0;
            }
            let page = p.page.take();
            let text = std::mem::take(&mut p.text);
            let predict = std::mem::take(&mut p.predict);
            let forget = std::mem::take(&mut p.forget);
            let (popup, clear_popup, clear_all, stop) =
                (p.popup.take(), std::mem::take(&mut p.clear_popup), std::mem::take(&mut p.clear_all), p.stop);
            drop(p);
            let mut out = Vec::new();
            if clear_all {
                rows.reset();
                if self.gfx != Gfx::Blocks {
                    out.extend_from_slice(b"\x1b_Ga=d,d=A,q=2\x1b\\");
                }
            }
            if forget {
                rows.fixed.fill(false);
                rows.misses = 0;
            }
            if clear_popup {
                if self.gfx == Gfx::Blocks {
                    // The popup was drawn over page cells: draw them all again.
                    rows.shown.fill(None);
                } else {
                    out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={POPUP_ID},q=2\x1b\\").as_bytes());
                }
            }
            out.extend_from_slice(&text);
            if predict != 0 && self.gfx != Gfx::Blocks {
                self.show_prediction(&mut out, &layout, &mut rows, predict);
            }
            if !out.is_empty() {
                write_out(&out);
                if predict != 0 {
                    crate::trace!("predict written {predict}");
                }
            }
            if stop {
                return;
            }
            let page = match page {
                Some(frame) => self.settle(frame, &layout, &mut rows),
                // No frame came in time: the newest held one, or back to how it was.
                None if rows.prediction.as_ref().is_some_and(|pr| Instant::now() >= pr.deadline) => {
                    let pr = rows.prediction.take().unwrap();
                    if pr.shown {
                        self.miss(&mut rows);
                    }
                    if pr.shown && pr.held.is_none() {
                        let mut out = Vec::new();
                        for t in 0..pr.base.len() {
                            rows.place(&mut out, &layout, t, pr.base[t]);
                        }
                        write_out(&out);
                    }
                    pr.held
                }
                None => None,
            };
            if let Some(frame) = page {
                self.draw_page(&frame, &layout, &mut rows);
            }
            if let (Some(frame), Some(_)) = (popup, layout.popup) {
                self.draw_popup(&frame, &layout);
            }
        }
    }

    fn miss(&self, rows: &mut Rows) {
        rows.misses += 1;
        if rows.misses >= 2 {
            self.predict_ok.store(false, Ordering::Relaxed);
            rows.hits = 0;
        }
    }

    fn show_prediction(&self, out: &mut Vec<u8>, layout: &Layout, rows: &mut Rows, delta: i32) {
        let n = layout.rows as usize * rows.bands;
        if rows.shown.len() != n || rows.fixed.len() != n {
            return;
        }
        if rows.prediction.is_none() {
            // Only from a screen of known rows.
            if rows.shown.iter().any(|h| h.is_none_or(|h| h == BLANK || !rows.cache.contains_key(&h))) {
                return;
            }
            let base = rows.shown.iter().map(|h| h.unwrap()).collect();
            let shown = self.predict_ok.load(Ordering::Relaxed);
            rows.prediction = Some(Prediction { base, steps: Vec::new(), deadline: Instant::now(), held: None, shown });
        }
        let pr = rows.prediction.as_mut().unwrap();
        let total = pr.steps.last().copied().unwrap_or(0) + delta;
        pr.steps.push(total);
        pr.deadline = Instant::now() + SETTLE;
        if !pr.shown {
            return;
        }
        let target = shifted(&pr.base, &rows.fixed, rows.bands, total);
        if target.contains(&BLANK) && rows.bg_sent != Some(rows.bg) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(rows.bg);
            out.extend_from_slice(format!("\x1b_Ga=t,f=24,s=1,v=1,i={BG_ID},q=2;{b64}\x1b\\").as_bytes());
            rows.bg_sent = Some(rows.bg);
        }
        for (t, h) in target.into_iter().enumerate() {
            rows.place(out, layout, t, h);
        }
    }

    /// While a prediction waits: a frame that shows the whole scroll ends it
    /// (and teaches which rows are fixed); one that shows less is held back.
    fn settle(&self, frame: Frame, layout: &Layout, rows: &mut Rows) -> Option<Frame> {
        let Some(pr) = &rows.prediction else { return Some(frame) };
        let (w, h) = (frame.width(), frame.height());
        if rows.key != Some((w, h, layout.top, layout.cols, layout.rows)) {
            rows.prediction = None;
            return Some(frame);
        }
        let real = Pixels::new(&frame).tile_hashes(layout.rows, layout.cols, rows.bands);
        let bands = rows.bands;
        let total = *pr.steps.last().unwrap();
        // The shift this frame shows best: none yet, a step, or all of it.
        let score = |s: i32| matches(&real, &shifted(&pr.base, &rows.fixed, bands, s));
        let best = std::iter::once(0).chain(pr.steps.iter().copied()).max_by_key(|&s| (score(s), s == total)).unwrap();
        let (best_score, total_score) = (score(best), score(total));
        // Rows the full prediction knew (not background).
        let movable = shifted(&pr.base, &rows.fixed, bands, total).iter().filter(|h| **h != BLANK).count().max(1);
        if pr.shown && best != total && best_score * 2 >= movable && Instant::now() < pr.deadline {
            rows.prediction.as_mut().unwrap().held = Some(frame);
            return None;
        }
        let pr = rows.prediction.take().unwrap();
        // Learn the fixed rows: same as before though the rest moved.
        if best == total && total != 0 {
            for r in 0..real.len() {
                let from = r as i64 + total as i64 * bands as i64;
                let inside = from >= 0 && (from as usize) < real.len();
                // Something else would be here had the row scrolled (or nothing
                // known, at the edge it scrolled in from).
                let other = !inside || pr.base[from as usize] != pr.base[r];
                if real[r] == pr.base[r] && other {
                    rows.fixed[r] = true;
                } else if real[r] != pr.base[r] {
                    rows.fixed[r] = false;
                }
            }
        }
        let right = best == total && total_score * 3 >= movable * 2;
        if !pr.shown {
            rows.hits = if right { rows.hits + 1 } else { 0 };
            if rows.hits >= 3 {
                rows.misses = 0;
                self.predict_ok.store(true, Ordering::Relaxed);
            }
        } else if right {
            rows.misses = 0;
        } else {
            self.miss(rows);
        }
        if crate::trace::on() {
            let want = shifted(&pr.base, &rows.fixed, bands, total);
            let off: Vec<usize> = (0..real.len()).filter(|&r| want[r] != BLANK && want[r] != real[r]).collect();
            let fixed: Vec<usize> = (0..real.len()).filter(|&r| rows.fixed[r]).collect();
            crate::trace!("settled{} best {best} of {total}, score {best_score} of {movable}, wrong rows {off:?}, fixed {fixed:?}", if pr.shown { "" } else { " (unshown)" });
        }
        Some(frame)
    }

    fn draw_page(&self, frame: &Frame, layout: &Layout, rows: &mut Rows) {
        let started = Instant::now();
        let (w, h) = (frame.width(), frame.height());
        if w == 0 || h == 0 || layout.cols == 0 || layout.rows == 0 {
            return;
        }
        let mut out = Vec::new();
        let key = (w, h, layout.top, layout.cols, layout.rows);
        if rows.key != Some(key) {
            if self.gfx != Gfx::Blocks && !rows.cache.is_empty() {
                out.extend_from_slice(b"\x1b_Ga=d,d=A,q=2\x1b\\");
            }
            rows.reset();
            rows.key = Some(key);
            rows.bands = (layout.cols as usize / TILE_COLS).clamp(1, 12);
            rows.shown = vec![None; layout.rows as usize * rows.bands];
            rows.fixed = vec![false; layout.rows as usize * rows.bands];
        }
        // Pixel row where cell row `r` of the page starts.
        let px_row = |r: u16| (r as u64 * h as u64 / layout.rows as u64) as usize;
        let (mut sent, mut moved) = (0u16, 0u16);
        let mut pixels = Pixels::new(frame);
        let hashes = pixels.tile_hashes(layout.rows, layout.cols, rows.bands);
        // The page's background, for tiles not known yet: at the left edge of
        // the middle row (the bottom often is a fixed footer).
        let (br, bg, bb) = pixels.at(1, h as usize / 2);
        rows.bg = [br, bg, bb];
        let px_col = |c: u16| c as usize * w as usize / layout.cols as usize;
        for t in 0..hashes.len() {
            let hash = hashes[t];
            if rows.shown[t] == Some(hash) {
                continue;
            }
            if self.gfx == Gfx::Blocks {
                // Cells: the whole row again.
                let r = (t / rows.bands) as u16;
                let row = r as usize * rows.bands..(r as usize + 1) * rows.bands;
                rows.shown[row.clone()].iter_mut().zip(&hashes[row]).for_each(|(s, h)| *s = Some(*h));
                blocks(&mut pixels, &mut out, 0, layout.top, layout.cols, layout.rows, r, r + 1);
                sent += 1;
                continue;
            }
            if rows.place(&mut out, layout, t, hash) {
                moved += 1;
                continue;
            }
            let (r, col, cols) = rows.cell(layout, t);
            let (y0, y1) = (px_row(r), px_row(r + 1));
            let (x0, x1) = (px_col(col), px_col(col + cols));
            let id = rows.next_id;
            rows.next_id += 1;
            rows.clock += 1;
            let rgb = pixels.rgb(x0, y0, x1 - x0, y1 - y0);
            out.extend_from_slice(format!("\x1b[{};{}H", layout.top + r + 1, col + 1).as_bytes());
            let place = format!("p={},c={cols},r=1,C=1,q=2", t + 1);
            self.transmit(&mut out, &format!("a=T,f=24,s={},v={},i={id},{place}", x1 - x0, y1 - y0), &rgb);
            rows.bytes += rgb.len();
            rows.cache.insert(hash, Cached { id, used: rows.clock, size: rgb.len() });
            // The tile's previous placement goes after the new one is there: no
            // flicker. Lowercase: the image stays in the terminal for reuse.
            rows.unplace(&mut out, t, rows.shown[t]);
            rows.shown[t] = Some(hash);
            sent += 1;
        }
        drop(pixels);
        // Over the budget: free the least recently used images not on screen.
        if rows.bytes > CACHE_BYTES && rows.prediction.is_none() {
            let on_screen: std::collections::HashSet<u64> = rows.shown.iter().flatten().copied().collect();
            let mut old: Vec<(u64, u64)> =
                rows.cache.iter().filter(|(h, _)| !on_screen.contains(h)).map(|(h, c)| (c.used, *h)).collect();
            old.sort_unstable();
            for (_, h) in old {
                if rows.bytes <= CACHE_BYTES * 3 / 4 {
                    break;
                }
                let c = rows.cache.remove(&h).unwrap();
                rows.bytes -= c.size;
                out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={},q=2\x1b\\", c.id).as_bytes());
            }
        }
        if out.is_empty() {
            return;
        }
        crate::trace!("frame encoded {} new {} moved {} KB in {:.2} ms", sent, moved, out.len() / 1024, started.elapsed().as_secs_f64() * 1000.0);
        write_out(&out);
        crate::trace!("frame written");
        let mut s = self.stats.lock().unwrap();
        s.frames += 1;
        s.last_ms = started.elapsed().as_secs_f64() * 1000.0;
        s.last_bytes = out.len();
        s.sent = sent;
        s.moved = moved;
        s.width = w;
        s.height = h;
    }

    fn draw_popup(&self, frame: &Frame, layout: &Layout) {
        let (w, h) = (frame.width(), frame.height());
        if w == 0 || h == 0 || layout.cols == 0 || layout.rows == 0 {
            return;
        }
        // The popup's place in cells.
        let (x, y, pw, ph) = layout.popup.unwrap();
        let cw = layout.css_w as f64 / layout.cols as f64;
        let ch = layout.css_h as f64 / layout.rows as f64;
        let col = (x as f64 / cw).floor().max(0.0) as u16;
        let row = (y as f64 / ch).floor().max(0.0) as u16;
        let cols = ((pw as f64 / cw).ceil() as u16).clamp(1, layout.cols.saturating_sub(col).max(1));
        let rows = ((ph as f64 / ch).ceil() as u16).clamp(1, layout.rows.saturating_sub(row).max(1));
        let mut out = Vec::new();
        let mut pixels = Pixels::new(frame);
        match self.gfx {
            Gfx::Blocks => blocks(&mut pixels, &mut out, col, layout.top + row, cols, rows, 0, rows),
            Gfx::KittyDirect | Gfx::KittyShm => {
                let rgb = pixels.rgb(0, 0, w as usize, h as usize);
                out.extend_from_slice(format!("\x1b[{};{}H", layout.top + row + 1, col + 1).as_bytes());
                let place = format!("a=T,f=24,s={w},v={h},i={POPUP_ID},p=1,c={cols},r={rows},C=1,q=2,z=1");
                self.transmit(&mut out, &place, &rgb);
            }
        }
        drop(pixels);
        write_out(&out);
    }

    /// One Kitty image: through shared memory, or zlib and base64 in chunks.
    fn transmit(&self, out: &mut Vec<u8>, place: &str, rgb: &[u8]) {
        if self.gfx == Gfx::KittyShm {
            if let Some(name) = shm_write(rgb) {
                let b64 = base64::engine::general_purpose::STANDARD.encode(name);
                out.extend_from_slice(format!("\x1b_G{place},t=s,S={};{b64}\x1b\\", rgb.len()).as_bytes());
                return;
            }
        }
        let mut z = ZlibEncoder::new(Vec::with_capacity(rgb.len() / 4), Compression::fast());
        z.write_all(rgb).unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(z.finish().unwrap());
        let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let m = u8::from(i + 1 < chunks.len());
            let head = if i == 0 { format!("{place},o=z,m={m}") } else { format!("m={m},q=2") };
            out.extend_from_slice(format!("\x1b_G{head};").as_bytes());
            out.extend_from_slice(chunk);
            out.extend_from_slice(b"\x1b\\");
        }
    }
}

fn write_out(bytes: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
    let _ = out.flush();
}

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceLock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddress(surface: *mut c_void) -> *mut c_void;
    fn IOSurfaceGetBytesPerRow(surface: *mut c_void) -> usize;
}

/// A frame's BGRA pixels, locked for reading while this lives.
pub struct Pixels {
    surface: Option<*mut c_void>,
    base: *const u8,
    stride: usize,
    w: usize,
    h: usize,
}

impl Pixels {
    pub fn new(frame: &Frame) -> Pixels {
        let (w, h) = (frame.width() as usize, frame.height() as usize);
        match frame.buffer() {
            FrameBuffer::IoSurface(surface) => unsafe {
                IOSurfaceLock(surface, 1, std::ptr::null_mut());
                let base = IOSurfaceGetBaseAddress(surface) as *const u8;
                Pixels { surface: Some(surface), base, stride: IOSurfaceGetBytesPerRow(surface), w, h }
            },
            FrameBuffer::SharedMemory { pixels, stride, .. } => {
                Pixels { surface: None, base: pixels.as_ptr(), stride, w, h }
            }
            FrameBuffer::Dmabuf { .. } => panic!("dmabuf frames are not handled"),
        }
    }

    fn at(&self, x: usize, y: usize) -> (u8, u8, u8) {
        unsafe {
            let p = self.base.add(y.min(self.h - 1) * self.stride + x.min(self.w - 1) * 4);
            (*p.add(2), *p.add(1), *p)
        }
    }

    /// Hash of each tile: `rows` cell rows, each of `bands` column bands
    /// of a `cols` wide grid. The size is part of the hash.
    fn tile_hashes(&self, rows: u16, cols: u16, bands: usize) -> Vec<u64> {
        let px_row = |r: usize| r * self.h / rows as usize;
        let px_col = |b: usize| (b * cols as usize / bands) * self.w / cols as usize;
        let mut out = Vec::with_capacity(rows as usize * bands);
        let mut hashers: Vec<xxhash_rust::xxh3::Xxh3> = Vec::with_capacity(bands);
        for r in 0..rows as usize {
            hashers.clear();
            hashers.resize_with(bands, xxhash_rust::xxh3::Xxh3::new);
            for y in px_row(r)..px_row(r + 1) {
                let line = unsafe { std::slice::from_raw_parts(self.base.add(y * self.stride), self.w * 4) };
                for (b, hasher) in hashers.iter_mut().enumerate() {
                    hasher.update(&line[px_col(b) * 4..px_col(b + 1) * 4]);
                }
            }
            let height = (px_row(r + 1) - px_row(r)) as u64;
            for (b, hasher) in hashers.iter().enumerate() {
                let width = (px_col(b + 1) - px_col(b)) as u64;
                out.push(hasher.digest() ^ (height << 48) ^ (width << 32));
            }
        }
        out
    }

    /// Packed RGB of a rectangle.
    pub fn rgb(&mut self, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
        let mut out = vec![0u8; w * h * 3];
        for row in 0..h {
            let src = unsafe { std::slice::from_raw_parts(self.base.add((y + row) * self.stride + x * 4), w * 4) };
            let dst = &mut out[row * w * 3..(row + 1) * w * 3];
            for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
            }
        }
        out
    }
}

impl Drop for Pixels {
    fn drop(&mut self) {
        if let Some(surface) = self.surface {
            unsafe { IOSurfaceUnlock(surface, 1, std::ptr::null_mut()) };
        }
    }
}

/// Packed RGB of the whole frame.
pub fn rgb(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    Pixels::new(frame).rgb(0, 0, w, h)
}

static SHM_SEQ: AtomicU32 = AtomicU32::new(0);

/// Writes `data` to a new POSIX shared memory object; the terminal unlinks it.
pub fn shm_write(data: &[u8]) -> Option<String> {
    let name = format!("/zweb-{}-{}", std::process::id(), SHM_SEQ.fetch_add(1, Ordering::Relaxed));
    let c = CString::new(name.clone()).unwrap();
    unsafe {
        let fd = libc::shm_open(c.as_ptr(), libc::O_CREAT | libc::O_RDWR | libc::O_EXCL, 0o600);
        if fd < 0 {
            return None;
        }
        if libc::ftruncate(fd, data.len() as libc::off_t) != 0 {
            libc::close(fd);
            libc::shm_unlink(c.as_ptr());
            return None;
        }
        let map = libc::mmap(std::ptr::null_mut(), data.len(), libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
        libc::close(fd);
        if map == libc::MAP_FAILED {
            libc::shm_unlink(c.as_ptr());
            return None;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), map as *mut u8, data.len());
        libc::munmap(map, data.len());
    }
    Some(name)
}

/// Half-block cells: each cell is two pixels, top as foreground of `▀`.
/// The frame fills `cols` x `rows` cells at (`col`, `row`); cell rows
/// `from..to` of them are drawn.
#[allow(clippy::too_many_arguments)]
fn blocks(px: &mut Pixels, out: &mut Vec<u8>, col: u16, row: u16, cols: u16, rows: u16, from: u16, to: u16) {
    let (w, h) = (px.w, px.h);
    // Average of a small grid in the cell half, for readable text.
    let sample = |cx: usize, half: usize| {
        let (x0, x1) = (cx * w / cols as usize, ((cx + 1) * w / cols as usize).max(cx * w / cols as usize + 1));
        let total = rows as usize * 2;
        let (y0, y1) = (half * h / total, ((half + 1) * h / total).max(half * h / total + 1));
        let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
        let (sx, sy) = (((x1 - x0) / 3).max(1), ((y1 - y0) / 3).max(1));
        for y in (y0..y1).step_by(sy) {
            for x in (x0..x1).step_by(sx) {
                let (pr, pg, pb) = px.at(x, y);
                r += pr as u32;
                g += pg as u32;
                b += pb as u32;
                n += 1;
            }
        }
        ((r / n) as u8, (g / n) as u8, (b / n) as u8)
    };
    let mut last = None;
    for r in from as usize..to as usize {
        out.extend_from_slice(format!("\x1b[{};{}H", row as usize + r + 1, col + 1).as_bytes());
        for c in 0..cols as usize {
            let (t, b) = (sample(c, r * 2), sample(c, r * 2 + 1));
            if last != Some((t, b)) {
                out.extend_from_slice(format!("\x1b[38;2;{};{};{};48;2;{};{};{}m", t.0, t.1, t.2, b.0, b.1, b.2).as_bytes());
                last = Some((t, b));
            }
            out.extend_from_slice("▀".as_bytes());
        }
    }
    out.extend_from_slice(b"\x1b[0m");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_keeps_fixed_rows_and_blanks_new_ones() {
        let base = [10, 1, 2, 3, 4, 20];
        let fixed = [true, false, false, false, false, true];
        assert_eq!(shifted(&base, &fixed, 1, 2), vec![10, 3, 4, BLANK, BLANK, 20]);
        assert_eq!(shifted(&base, &fixed, 1, -1), vec![10, BLANK, 1, 2, 3, 20]);
        assert_eq!(shifted(&base, &fixed, 1, 0), base.to_vec());
        // Two bands: a fixed sidebar (band 0) beside text that scrolls (band 1).
        let base = [90, 1, 91, 2, 92, 3];
        let fixed = [true, false, true, false, true, false];
        assert_eq!(shifted(&base, &fixed, 2, 1), vec![90, 2, 91, 3, 92, BLANK]);
    }

    #[test]
    fn matches_skips_blank() {
        assert_eq!(matches(&[1, 2, 3], &[1, BLANK, 4]), 1);
    }
}
