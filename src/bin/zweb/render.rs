//! Draws page frames in the terminal, on a thread of its own.
//!
//! Kitty graphics: raw RGB, zlib-compressed and base64 in the escape codes
//! (works everywhere, also over SSH), or through POSIX shared memory when the
//! terminal says it reads it (local only, nearly no bytes on the pipe).
//! Blocks: `▀` cells with truecolor, for terminals without graphics.
//! Either way only cell rows whose pixels changed are drawn.

use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::ffi::CString;
use std::io::Write;
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
    /// Page rows to leave to text at the bottom (the prefix popup), or Some(0).
    cover: Option<u16>,
    /// The hint overlay to show, or Some(None) to remove it.
    hop: Option<Option<Hop>>,
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
/// The hint overlay, over the page and its popup: the code boxes, and the
/// dim layer under them.
const HOP_ID: u32 = 3;
const DIM_ID: u32 = 4;

/// The hint overlay (hints.js): a base64 PNG atlas of the code boxes, where
/// each goes, and the dim layer's colour.
#[derive(Default, Clone)]
pub struct Hop {
    pub png: String,
    /// [col, row, cols, rows, x, y, w, h]: cells, and pixels in the atlas.
    pub cells: Vec<[u32; 8]>,
    pub dim: [u8; 4],
}
/// After this long with no new frame, photo tiles sent blurry are sent sharp.
const REFINE_AFTER: Duration = Duration::from_millis(120);

/// A new tile to send.
struct Part {
    /// Escape codes to write before it (cursor move, earlier deletes).
    before: Vec<u8>,
    /// Size of `rgb`: the tile and its border.
    w: usize,
    h: usize,
    /// The tile in `rgb` (x, y, w, h), and as placement keys.
    inner: (usize, usize, usize, usize),
    src: String,
    id: u32,
    place: String,
    rgb: Vec<u8>,
    hash: u64,
}

/// Half the width and height, each pixel the mean of up to 4.
fn half(rgb: &[u8], w: usize, h: usize) -> (usize, usize, Vec<u8>) {
    let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
    let mut out = vec![0u8; hw * hh * 3];
    for y in 0..hh {
        for x in 0..hw {
            let at = (y * hw + x) * 3;
            out[at..at + 3].copy_from_slice(&half_pixel(rgb, w, h, x, y));
        }
    }
    (hw, hh, out)
}

fn half_pixel(rgb: &[u8], w: usize, h: usize, x: usize, y: usize) -> [u8; 3] {
    let mut pixel = [0; 3];
    for (c, value) in pixel.iter_mut().enumerate() {
        let (mut sum, mut n) = (0u32, 0u32);
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (sx, sy) = (2 * x + dx, 2 * y + dy);
            if sx < w && sy < h {
                sum += rgb[(sy * w + sx) * 3 + c] as u32;
                n += 1;
            }
        }
        *value = (sum / n) as u8;
    }
    pixel
}

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
/// A prediction is shown with at most this many rows not known yet.
const MAX_UNKNOWN_ROWS: usize = 3;

struct Cached {
    id: u32,
    used: u64,
    size: usize,
    /// The part of the image shown (`x=,y=,w=,h=,`): the tile without the
    /// 1-pixel border of its neighbours sent with it.
    src: String,
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
    /// Where row 0 of `base` is in the page (rows of the tape).
    base_offset: Option<i64>,
    /// What is on screen for the whole shift, tape tiles included.
    target: Vec<u64>,
}

struct PredictionMatch {
    best: i32,
    total: i32,
    best_score: usize,
    movable: usize,
    total_score: usize,
    total_known: usize,
}

impl PredictionMatch {
    fn right(&self) -> bool {
        self.best == self.total && self.total_score * 3 >= self.total_known * 2
    }

    fn partial(&self, prediction: &Prediction) -> bool {
        prediction.shown
            && self.best != self.total
            && self.best_score * 2 >= self.movable
            && Instant::now() < prediction.deadline
    }
}

impl Prediction {
    fn score(&self, real: &[u64], fixed: &[bool], bands: usize) -> PredictionMatch {
        let total = *self.steps.last().unwrap();
        let share = |s: i32| {
            let want = shifted(&self.base, fixed, bands, s);
            let known = want.iter().filter(|h| **h != BLANK).count();
            (matches(real, &want), known.max(1))
        };
        let better = |a: (usize, usize), b: (usize, usize)| a.0 * b.1 > b.0 * a.1;
        let mut best = total;
        for s in std::iter::once(0).chain(self.steps.iter().copied()) {
            if better(share(s), share(best)) {
                best = s;
            }
        }
        let ((best_score, movable), (mut total_score, mut total_known)) =
            (share(best), share(total));
        if self.shown && self.target.len() == real.len() {
            total_known = self.target.iter().filter(|h| **h != BLANK).count().max(1);
            total_score = matches(real, &self.target);
        }
        PredictionMatch {
            best,
            total,
            best_score,
            movable,
            total_score,
            total_known,
        }
    }
}

/// Rows of the page seen before, by their place in it: a jump back to
/// them (page up, then down) is shown at once too.
#[derive(Default)]
struct Tape {
    /// Where screen row 0 is in the page, when known.
    offset: Option<i64>,
    rows: HashMap<i64, Vec<u64>>,
    /// A distinctive row's signature to its place; AMBIGUOUS if in two.
    places: HashMap<u64, i64>,
    /// Where a part of the page not linked to the rest starts.
    next_island: i64,
    /// Where the frame being drawn was expected to be (after a scroll).
    expect: Option<i64>,
}

const AMBIGUOUS: i64 = i64::MIN;

impl Tape {
    fn clear(&mut self) {
        self.offset = None;
        self.rows.clear();
        self.places.clear();
        self.expect = None;
    }

    /// The signature of a row: its tiles that scroll, if they are not all
    /// alike (an empty row is everywhere).
    fn signature(tiles: &[u64], fixed: &[bool]) -> Option<u64> {
        let moving: Vec<u64> = tiles
            .iter()
            .zip(fixed)
            .filter(|(_, f)| !**f)
            .map(|(t, _)| *t)
            .collect();
        if moving.windows(2).all(|w| w[0] == w[1]) {
            return None;
        }
        Some(xxhash_rust::xxh3::xxh3_64(
            &moving
                .iter()
                .flat_map(|t| t.to_le_bytes())
                .collect::<Vec<u8>>(),
        ))
    }

    /// Finds where the frame `real` is in the page, and records its rows.
    fn record(&mut self, real: &[u64], fixed: &[bool], bands: usize, before: &[Option<u64>]) {
        let n = real.len() / bands;
        let votes = self.offset_votes(real, fixed, bands);
        let expect = self.expect.take();
        let for_expect = expect.and_then(|e| votes.get(&e).copied()).unwrap_or(0);
        // The scroll asked for, unless the rows clearly say another place.
        let voted = votes
            .into_iter()
            .max_by_key(|(o, c)| (*c, Some(*o) == expect))
            .filter(|(o, c)| {
                *c >= 3 && (expect.is_none() || Some(*o) == expect || *c > 2 * for_expect + 3)
            })
            .map(|(o, _)| o);
        let unchanged = before.len() == real.len()
            && before
                .iter()
                .zip(real)
                .filter(|(a, b)| **a == Some(**b))
                .count()
                * 2
                > real.len();
        self.offset = voted
            .or(expect)
            .or(if unchanged { self.offset } else { None })
            .or_else(|| {
                self.next_island += 1 << 32;
                Some(self.next_island)
            });
        let off = self.offset.unwrap();
        crate::trace!("tape offset {off} (voted {voted:?}, unchanged {unchanged})");
        if self.rows.len() > 50_000 {
            self.rows.clear();
            self.places.clear();
        }
        for r in 0..n {
            let v = off + r as i64;
            let row = &real[r * bands..(r + 1) * bands];
            self.record_row(v, row, &fixed[r * bands..(r + 1) * bands]);
        }
    }

    fn offset_votes(&self, real: &[u64], fixed: &[bool], bands: usize) -> HashMap<i64, usize> {
        let mut votes = HashMap::new();
        for r in 0..real.len() / bands {
            let tiles = r * bands..(r + 1) * bands;
            let signature = Tape::signature(&real[tiles.clone()], &fixed[tiles]);
            if let Some(&v) = signature.and_then(|s| self.places.get(&s)) {
                if v != AMBIGUOUS {
                    *votes.entry(v - r as i64).or_default() += 1;
                }
            }
        }
        votes
    }

    fn record_row(&mut self, v: i64, row: &[u64], fixed: &[bool]) {
        // What was there before is not there any more.
        if let Some(old) = self.rows.insert(v, row.to_vec()).filter(|old| old != row) {
            if let Some(s) = Tape::signature(&old, fixed) {
                if self.places.get(&s) == Some(&v) {
                    self.places.remove(&s);
                }
            }
        }
        if let Some(s) = Tape::signature(row, fixed) {
            let e = self.places.entry(s).or_insert(v);
            if *e != v {
                *e = AMBIGUOUS;
            }
        }
    }
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
    tape: Tape,
    /// Photo tiles sent at half size: their full pixels, by hash.
    refine: HashMap<u64, (usize, usize, Vec<u8>, String)>,
    /// When the last page frame was drawn.
    drawn_at: Instant,
    /// Hash of the hint overlay the terminal holds: shown again, it is only placed.
    hop_sent: Option<u64>,
    /// Bottom page rows left to text: no tiles are placed there.
    cover: u16,
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
            tape: Tape::default(),
            refine: HashMap::new(),
            drawn_at: Instant::now(),
            hop_sent: None,
            cover: 0,
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
        self.tape.clear();
        self.refine.clear();
        self.hop_sent = None;
    }

    /// The cell row, first column and width in columns of tile `t`.
    fn cell(&self, layout: &Layout, t: usize) -> (u16, u16, u16) {
        let (r, b) = (t / self.bands, t % self.bands);
        let col = |b: usize| (b * layout.cols as usize / self.bands) as u16;
        (r as u16, col(b), col(b + 1) - col(b))
    }

    fn covered(&self, layout: &Layout, t: usize) -> bool {
        t / self.bands + self.cover as usize >= layout.rows as usize
    }

    /// Leaves the bottom `n` page rows to text: their tiles go, and are drawn
    /// again by the next frame once uncovered.
    fn set_cover(&mut self, out: &mut Vec<u8>, layout: &Layout, n: u16, kitty: bool) {
        self.cover = n;
        for t in 0..self.shown.len() {
            if self.covered(layout, t) {
                if kitty {
                    self.unplace(out, t, self.shown[t]);
                }
                self.shown[t] = None;
            }
        }
    }

    /// Puts image `hash` (a cached tile, or BLANK) at tile `t`, then removes
    /// what was there. Returns whether a placement was written.
    fn place(&mut self, out: &mut Vec<u8>, layout: &Layout, t: usize, hash: u64) -> bool {
        let old = self.shown[t];
        if old == Some(hash) || self.covered(layout, t) {
            return false;
        }
        let id = if hash == BLANK {
            BG_ID
        } else {
            let Some(c) = self.cache.get_mut(&hash) else {
                return false;
            };
            self.clock += 1;
            c.used = self.clock;
            c.id
        };
        let src = self.cache.get(&hash).map_or("", |c| c.src.as_str());
        let (r, col, cols) = self.cell(layout, t);
        out.extend_from_slice(
            format!(
                "\x1b[{};{}H\x1b_Ga=p,i={id},{src}p={},c={cols},r=1,C=1,q=2\x1b\\",
                layout.top + r + 1,
                col + 1,
                t + 1
            )
            .as_bytes(),
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

    fn has_blurry_tiles(&self) -> bool {
        self.refine.keys().any(|h| self.shown.contains(&Some(*h)))
    }

    fn wake_deadline(&self) -> Option<Instant> {
        let refine = self
            .has_blurry_tiles()
            .then(|| self.drawn_at + REFINE_AFTER);
        self.prediction.as_ref().map(|pr| pr.deadline).or(refine)
    }

    fn prediction_target(&self, total: i32) -> Vec<u64> {
        let pr = self.prediction.as_ref().unwrap();
        let mut target = shifted(&pr.base, &self.fixed, self.bands, total);
        let Some(off) = pr.base_offset else {
            return target;
        };
        // Tiles scrolled in from beyond the screen: from the tape, if seen.
        for (t, h) in target.iter_mut().enumerate() {
            if *h == BLANK && !self.fixed[t] {
                let seen = self
                    .tape
                    .rows
                    .get(&(off + (t / self.bands) as i64 + total as i64))
                    .map(|row| row[t % self.bands]);
                if let Some(tile) = seen.filter(|tile| self.cache.contains_key(tile)) {
                    *h = tile;
                }
            }
        }
        target
    }

    fn learn_fixed(&mut self, base: &[u64], real: &[u64], total: i32) {
        // Not from the background: blank tiles look the same anywhere.
        let mut counts: HashMap<u64, usize> = HashMap::new();
        base.iter()
            .for_each(|h| *counts.entry(*h).or_default() += 1);
        let background = counts.into_iter().max_by_key(|(_, c)| *c).map(|(h, _)| h);
        for t in 0..real.len() {
            let from = t as i64 + total as i64 * self.bands as i64;
            let inside = from >= 0 && (from as usize) < real.len();
            // Something else would be here had this tile scrolled.
            let other = !inside || base[from as usize] != base[t];
            if real[t] == base[t] && other && Some(real[t]) != background {
                self.fixed[t] = true;
            } else if real[t] != base[t] {
                self.fixed[t] = false;
            }
        }
    }

    fn prepare_tile(
        &mut self,
        pixels: &mut Pixels,
        layout: &Layout,
        t: usize,
        hash: u64,
        out: &mut Vec<u8>,
    ) -> Part {
        let (w, h) = (pixels.w, pixels.h);
        let px_row = |r: u16| (r as u64 * h as u64 / layout.rows as u64) as usize;
        let px_col = |c: u16| c as usize * w / layout.cols as usize;
        let (r, col, cols) = self.cell(layout, t);
        let (y0, y1) = (px_row(r), px_row(r + 1));
        let (x0, x1) = (px_col(col), px_col(col + cols));
        let id = self.next_id;
        self.next_id += 1;
        self.clock += 1;
        // Neighbour pixels prevent seams when the terminal scales the tile.
        let (bx0, by0) = (x0.saturating_sub(1), y0.saturating_sub(1));
        let (bx1, by1) = ((x1 + 1).min(w), (y1 + 1).min(h));
        let rgb = pixels.rgb(bx0, by0, bx1 - bx0, by1 - by0);
        let inner = (x0 - bx0, y0 - by0, x1 - x0, y1 - y0);
        let src = format!("x={},y={},w={},h={},", inner.0, inner.1, inner.2, inner.3);
        out.extend_from_slice(format!("\x1b[{};{}H", layout.top + r + 1, col + 1).as_bytes());
        let place = format!("p={},c={cols},r=1,C=1,q=2", t + 1);
        let size = rgb.len();
        self.cache.insert(
            hash,
            Cached {
                id,
                used: self.clock,
                size,
                src: src.clone(),
            },
        );
        let part = Part {
            before: std::mem::take(out),
            w: bx1 - bx0,
            h: by1 - by0,
            inner,
            src,
            id,
            place,
            rgb,
            hash,
        };
        self.bytes += size;
        // Delete the previous placement only after the new image is sent.
        self.unplace(out, t, self.shown[t]);
        self.shown[t] = Some(hash);
        part
    }

    /// Shows the hint overlay over the page's cells, or removes it. The
    /// terminal keeps the atlas: the same one shown again is only placed.
    fn hop(&mut self, out: &mut Vec<u8>, layout: &Layout, hop: Option<&Hop>) {
        out.extend_from_slice(format!("\x1b_Ga=d,d=i,i={HOP_ID},q=2\x1b\\").as_bytes());
        let Some(hop) = hop.filter(|_| layout.cols > 0 && layout.rows > 0) else {
            out.extend_from_slice(format!("\x1b_Ga=d,d=i,i={DIM_ID},q=2\x1b\\").as_bytes());
            return;
        };
        let dim = base64::engine::general_purpose::STANDARD.encode(hop.dim);
        out.extend_from_slice(
            format!(
                "\x1b[{};1H\x1b_Ga=T,f=32,s=1,v=1,i={DIM_ID},p=1,c={},r={},C=1,q=2,z=2;{dim}\x1b\\",
                layout.top + 1,
                layout.cols,
                layout.rows
            )
            .as_bytes(),
        );
        if hop.png.is_empty() {
            return;
        }
        let hash = xxhash_rust::xxh3::xxh3_64(hop.png.as_bytes());
        if self.hop_sent != Some(hash) {
            self.hop_sent = Some(hash);
            let chunks: Vec<&[u8]> = hop.png.as_bytes().chunks(4096).collect();
            for (i, chunk) in chunks.iter().enumerate() {
                let m = u8::from(i + 1 < chunks.len());
                let head = if i == 0 {
                    format!("a=t,f=100,i={HOP_ID},q=2,m={m}")
                } else {
                    format!("m={m},q=2")
                };
                out.extend_from_slice(format!("\x1b_G{head};").as_bytes());
                out.extend_from_slice(chunk);
                out.extend_from_slice(b"\x1b\\");
            }
        }
        for (k, &[col, row, cols, rows, x, y, w, h]) in hop.cells.iter().enumerate() {
            if row >= layout.rows as u32 || col >= layout.cols as u32 {
                continue;
            }
            out.extend_from_slice(
                format!(
                    "\x1b[{};{}H\x1b_Ga=p,i={HOP_ID},p={},x={x},y={y},w={w},h={h},c={cols},r={rows},C=1,q=2,z=3\x1b\\",
                    layout.top as u32 + row + 1,
                    col + 1,
                    k + 1
                )
                .as_bytes(),
            );
        }
    }

    fn evict(&mut self, out: &mut Vec<u8>) {
        if self.bytes <= CACHE_BYTES || self.prediction.is_some() {
            return;
        }
        let on_screen: std::collections::HashSet<u64> =
            self.shown.iter().flatten().copied().collect();
        let mut old: Vec<(u64, u64)> = self
            .cache
            .iter()
            .filter(|(h, _)| !on_screen.contains(h))
            .map(|(h, c)| (c.used, *h))
            .collect();
        old.sort_unstable();
        for (_, h) in old {
            if self.bytes <= CACHE_BYTES * 3 / 4 {
                break;
            }
            let c = self.cache.remove(&h).unwrap();
            self.refine.remove(&h);
            self.bytes -= c.size;
            out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={},q=2\x1b\\", c.id).as_bytes());
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
            if known {
                base[from as usize]
            } else {
                BLANK
            }
        })
        .collect()
}

/// How many rows of `real` are the rows of `want` (BLANK rows not counted).
fn matches(real: &[u64], want: &[u64]) -> usize {
    real.iter()
        .zip(want)
        .filter(|(a, b)| **b != BLANK && a == b)
        .count()
}

impl Pending {
    fn busy(&self) -> bool {
        self.page.is_some()
            || self.popup.is_some()
            || !self.text.is_empty()
            || self.predict != 0
            || self.clear_popup
            || self.cover.is_some()
            || self.hop.is_some()
            || self.clear_all
            || self.forget
            || self.stop
    }
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

    /// Shows the hint overlay over the page; None removes it.
    pub fn hop(&self, hop: Option<Hop>) {
        self.pending.lock().unwrap().hop = Some(hop);
        self.wake.notify_one();
    }

    pub fn clear_popup(&self) {
        self.pending.lock().unwrap().clear_popup = true;
        self.wake.notify_one();
    }

    /// Leaves the bottom `rows` page rows to text; 0 gives them back.
    pub fn cover(&self, rows: u16) {
        self.pending.lock().unwrap().cover = Some(rows);
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
        p.hop = None;
        self.wake.notify_one();
    }

    pub fn stop(&self) {
        self.pending.lock().unwrap().stop = true;
        self.wake.notify_one();
    }

    pub fn run(&self) {
        let mut rows = Rows::new();
        loop {
            let (layout, pending) = self.wait_pending(&rows);
            self.write_pending(&pending, &layout, &mut rows);
            if pending.stop {
                return;
            }
            let page = match pending.page {
                Some(frame) => self.settle(frame, &layout, &mut rows),
                None => self.expire_prediction(&layout, &mut rows),
            };
            if let Some(frame) = page {
                self.draw_page(&frame, &layout, &mut rows);
            }
            if let (Some(frame), Some(_)) = (pending.popup, layout.popup) {
                self.draw_popup(&frame, &layout);
            }
            let on_screen = rows.has_blurry_tiles();
            if rows.prediction.is_none() && on_screen && rows.drawn_at.elapsed() >= REFINE_AFTER {
                self.refine(&layout, &mut rows);
            }
        }
    }

    fn wait_pending(&self, rows: &Rows) -> (Layout, Pending) {
        let mut p = self.pending.lock().unwrap();
        while !p.busy() {
            match rows.wake_deadline() {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    p = self.wake.wait_timeout(p, deadline - now).unwrap().0;
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
        let pending = std::mem::take(&mut *p);
        p.stop = pending.stop;
        (layout, pending)
    }

    fn clear_pending(&self, pending: &Pending, rows: &mut Rows, out: &mut Vec<u8>) {
        if pending.clear_all {
            rows.reset();
            if self.gfx != Gfx::Blocks {
                out.extend_from_slice(b"\x1b_Ga=d,d=A,q=2\x1b\\");
            }
        }
        if pending.forget {
            rows.fixed.fill(false);
            rows.misses = 0;
            rows.tape.clear();
        }
        if pending.clear_popup {
            if self.gfx == Gfx::Blocks {
                // The popup was drawn over page cells: draw them all again.
                rows.shown.fill(None);
            } else {
                out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={POPUP_ID},q=2\x1b\\").as_bytes());
            }
        }
    }

    fn write_pending(&self, pending: &Pending, layout: &Layout, rows: &mut Rows) {
        let mut out = Vec::new();
        self.clear_pending(pending, rows, &mut out);
        if let Some(n) = pending.cover {
            rows.set_cover(&mut out, layout, n, self.gfx != Gfx::Blocks);
        }
        out.extend_from_slice(&pending.text);
        if pending.predict != 0 && self.gfx != Gfx::Blocks {
            self.show_prediction(&mut out, layout, rows, pending.predict);
        }
        // Not over the help screen.
        let hop = pending
            .hop
            .as_ref()
            .filter(|h| self.gfx != Gfx::Blocks && (h.is_none() || !layout.hidden));
        if let Some(h) = hop {
            rows.hop(&mut out, layout, h.as_ref());
        }
        if !out.is_empty() {
            write_out(&out);
            if pending.predict != 0 {
                crate::trace!("predict written {}", pending.predict);
            }
            if hop.is_some_and(|h| h.is_some()) {
                crate::trace!("hints written");
            }
        }
    }

    fn expire_prediction(&self, layout: &Layout, rows: &mut Rows) -> Option<Frame> {
        let deadline = rows.prediction.as_ref()?.deadline;
        if Instant::now() < deadline {
            return None;
        }
        let pr = rows.prediction.take().unwrap();
        if pr.shown {
            self.miss(rows);
        }
        // No frame came in time: the newest held one, or back to how it was.
        if pr.shown && pr.held.is_none() {
            let mut out = Vec::new();
            for t in 0..pr.base.len() {
                rows.place(&mut out, layout, t, pr.base[t]);
            }
            write_out(&out);
        }
        pr.held
    }

    /// Sends sharp the photo tiles still shown blurry.
    fn refine(&self, layout: &Layout, rows: &mut Rows) {
        let mut out = Vec::new();
        let mut n = 0;
        let mut keep = HashMap::new();
        for (hash, (w, h, rgb, src)) in std::mem::take(&mut rows.refine) {
            let tiles: Vec<usize> = (0..rows.shown.len())
                .filter(|&t| rows.shown[t] == Some(hash))
                .collect();
            let Some(old) = rows.cache.get(&hash).map(|c| c.id) else {
                continue;
            };
            if tiles.is_empty() {
                // Not on screen: sharp if it comes back (cached as blurry).
                keep.insert(hash, (w, h, rgb, src));
                continue;
            }
            let id = rows.next_id;
            rows.next_id += 1;
            for (i, &t) in tiles.iter().enumerate() {
                let (r, col, cols) = rows.cell(layout, t);
                out.extend_from_slice(
                    format!("\x1b[{};{}H", layout.top + r + 1, col + 1).as_bytes(),
                );
                let place = format!("{src}p={},c={cols},r=1,C=1,q=2", t + 1);
                if i == 0 {
                    self.transmit(
                        &mut out,
                        &format!("a=T,f=24,s={w},v={h},i={id},{place}"),
                        &rgb,
                    );
                } else {
                    out.extend_from_slice(format!("\x1b_Ga=p,i={id},{place}\x1b\\").as_bytes());
                }
            }
            // The blurry image and all its placements go.
            out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={old},q=2\x1b\\").as_bytes());
            let c = rows.cache.get_mut(&hash).unwrap();
            rows.bytes += rgb.len() - c.size;
            c.id = id;
            c.size = rgb.len();
            c.src = src;
            n += 1;
        }
        // Bounded: about 6 KB each.
        if keep.len() <= 4000 {
            rows.refine = keep;
        }
        if !out.is_empty() {
            write_out(&out);
            crate::trace!("refined {n} tiles, {} KB", out.len() / 1024);
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
        if !self.start_prediction(rows) {
            return;
        }
        let pr = rows.prediction.as_mut().unwrap();
        let total = pr.steps.last().copied().unwrap_or(0) + delta;
        pr.steps.push(total);
        pr.deadline = Instant::now() + SETTLE;
        if !pr.shown {
            return;
        }
        let bands = rows.bands;
        let target = rows.prediction_target(total);
        // Never a screen of background: unknown rows wait for the page.
        let unknown = target.iter().filter(|h| **h == BLANK).count();
        let pr = rows.prediction.as_mut().unwrap();
        pr.target = target.clone();
        if unknown > MAX_UNKNOWN_ROWS * bands {
            pr.shown = false;
            crate::trace!(
                "predict not shown: {} unknown tiles (base offset {:?}, total {total})",
                unknown,
                pr.base_offset
            );
            return;
        }
        if target.contains(&BLANK) && rows.bg_sent != Some(rows.bg) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(rows.bg);
            out.extend_from_slice(
                format!("\x1b_Ga=t,f=24,s=1,v=1,i={BG_ID},q=2;{b64}\x1b\\").as_bytes(),
            );
            rows.bg_sent = Some(rows.bg);
        }
        for (t, h) in target.into_iter().enumerate() {
            rows.place(out, layout, t, h);
        }
    }

    fn start_prediction(&self, rows: &mut Rows) -> bool {
        if rows.prediction.is_some() {
            return true;
        }
        // Only from a screen of known rows.
        if rows
            .shown
            .iter()
            .any(|h| h.is_none_or(|h| h == BLANK || !rows.cache.contains_key(&h)))
        {
            return false;
        }
        let base = rows.shown.iter().map(|h| h.unwrap()).collect();
        let shown = self.predict_ok.load(Ordering::Relaxed);
        let base_offset = rows.tape.offset;
        rows.prediction = Some(Prediction {
            base,
            steps: Vec::new(),
            deadline: Instant::now(),
            held: None,
            shown,
            base_offset,
            target: Vec::new(),
        });
        true
    }

    /// While a prediction waits: a frame that shows the whole scroll ends it
    /// (and teaches which rows are fixed); one that shows less is held back.
    fn settle(&self, frame: Frame, layout: &Layout, rows: &mut Rows) -> Option<Frame> {
        let Some(pr) = &rows.prediction else {
            return Some(frame);
        };
        let (w, h) = (frame.width(), frame.height());
        if rows.key != Some((w, h, layout.top, layout.cols, layout.rows)) {
            rows.prediction = None;
            return Some(frame);
        }
        let real = Pixels::new(&frame).tile_hashes(layout.rows, layout.cols, rows.bands);
        let score = pr.score(&real, &rows.fixed, rows.bands);
        if score.partial(pr) {
            rows.prediction.as_mut().unwrap().held = Some(frame);
            return None;
        }
        let pr = rows.prediction.take().unwrap();
        self.record_prediction(rows, &pr, &score, &real);
        Self::trace_prediction(rows, &pr, &score, &real);
        Some(frame)
    }

    fn record_prediction(
        &self,
        rows: &mut Rows,
        pr: &Prediction,
        score: &PredictionMatch,
        real: &[u64],
    ) {
        let right = score.right();
        if right {
            rows.tape.expect = pr.base_offset.map(|o| o + score.total as i64);
        }
        if right && score.total != 0 {
            rows.learn_fixed(&pr.base, real, score.total);
        }
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
    }

    fn trace_prediction(rows: &Rows, pr: &Prediction, score: &PredictionMatch, real: &[u64]) {
        if crate::trace::on() {
            let PredictionMatch {
                best,
                total,
                total_score,
                total_known,
                ..
            } = score;
            let want = if pr.target.len() == real.len() {
                pr.target.clone()
            } else {
                shifted(&pr.base, &rows.fixed, rows.bands, *total)
            };
            let off: Vec<usize> = (0..real.len())
                .filter(|&r| want[r] != BLANK && want[r] != real[r])
                .collect();
            let fixed: Vec<usize> = (0..real.len()).filter(|&r| rows.fixed[r]).collect();
            crate::trace!("settled{} best {best} of {total}, score {total_score} of {total_known}, wrong rows {off:?}, fixed {fixed:?}", if pr.shown { "" } else { " (unshown)" });
        }
    }

    fn draw_page(&self, frame: &Frame, layout: &Layout, rows: &mut Rows) {
        let started = Instant::now();
        let (w, h) = (frame.width(), frame.height());
        if w == 0 || h == 0 || layout.cols == 0 || layout.rows == 0 {
            return;
        }
        let mut out = Vec::new();
        self.prepare_page((w, h), layout, rows, &mut out);
        let mut pixels = Pixels::new(frame);
        let hashes = pixels.tile_hashes(layout.rows, layout.cols, rows.bands);
        if self.gfx != Gfx::Blocks {
            let before = rows.shown.clone();
            rows.tape.record(&hashes, &rows.fixed, rows.bands, &before);
        }
        // The middle left edge avoids fixed footers when sampling background.
        let (br, bg, bb) = pixels.at(1, h as usize / 2);
        rows.bg = [br, bg, bb];
        let (parts, sent, moved) = self.draw_tiles(&mut pixels, layout, rows, &hashes, &mut out);
        drop(pixels);
        out = self.encode_parts(parts, out, rows);
        rows.evict(&mut out);
        if out.is_empty() {
            return;
        }
        rows.drawn_at = Instant::now();
        crate::trace!(
            "frame encoded {} new {} moved {} KB in {:.2} ms",
            sent,
            moved,
            out.len() / 1024,
            started.elapsed().as_secs_f64() * 1000.0
        );
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

    fn prepare_page(&self, size: (u32, u32), layout: &Layout, rows: &mut Rows, out: &mut Vec<u8>) {
        let key = (size.0, size.1, layout.top, layout.cols, layout.rows);
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
    }

    fn draw_tiles(
        &self,
        pixels: &mut Pixels,
        layout: &Layout,
        rows: &mut Rows,
        hashes: &[u64],
        out: &mut Vec<u8>,
    ) -> (Vec<Part>, u16, u16) {
        let (mut sent, mut moved) = (0u16, 0u16);
        // (bytes before, image keys, pixels) of each new tile, in order.
        let mut parts: Vec<Part> = Vec::new();
        for t in 0..hashes.len() {
            let hash = hashes[t];
            if rows.shown[t] == Some(hash) || rows.covered(layout, t) {
                continue;
            }
            if self.gfx == Gfx::Blocks {
                // Cells: the whole row again.
                let r = (t / rows.bands) as u16;
                let row = r as usize * rows.bands..(r as usize + 1) * rows.bands;
                rows.shown[row.clone()]
                    .iter_mut()
                    .zip(&hashes[row])
                    .for_each(|(s, h)| *s = Some(*h));
                blocks(
                    pixels,
                    out,
                    0,
                    layout.top,
                    layout.cols,
                    layout.rows,
                    r,
                    r + 1,
                );
                sent += 1;
                continue;
            }
            if rows.place(out, layout, t, hash) {
                moved += 1;
                continue;
            }
            parts.push(rows.prepare_tile(pixels, layout, t, hash, out));
            sent += 1;
        }
        (parts, sent, moved)
    }

    fn encode_parts(&self, parts: Vec<Part>, out: Vec<u8>, rows: &mut Rows) -> Vec<u8> {
        if parts.is_empty() {
            return out;
        }
        let encoded = self.encode_all(&parts);
        let mut all =
            Vec::with_capacity(encoded.iter().map(|e| e.0.len()).sum::<usize>() + out.len() + 4096);
        for (part, (image, blurry)) in parts.into_iter().zip(encoded) {
            all.extend_from_slice(&part.before);
            all.extend_from_slice(&image);
            if blurry {
                // Shown whole until sent sharp (refine).
                if let Some(c) = rows.cache.get_mut(&part.hash) {
                    c.src.clear();
                }
                rows.refine
                    .insert(part.hash, (part.w, part.h, part.rgb, part.src));
            }
        }
        all.extend_from_slice(&out);
        all
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
        let cols =
            ((pw as f64 / cw).ceil() as u16).clamp(1, layout.cols.saturating_sub(col).max(1));
        let rows =
            ((ph as f64 / ch).ceil() as u16).clamp(1, layout.rows.saturating_sub(row).max(1));
        let mut out = Vec::new();
        let mut pixels = Pixels::new(frame);
        match self.gfx {
            Gfx::Blocks => blocks(
                &mut pixels,
                &mut out,
                col,
                layout.top + row,
                cols,
                rows,
                0,
                rows,
            ),
            Gfx::KittyDirect | Gfx::KittyShm => {
                let rgb = pixels.rgb(0, 0, w as usize, h as usize);
                out.extend_from_slice(
                    format!("\x1b[{};{}H", layout.top + row + 1, col + 1).as_bytes(),
                );
                let place =
                    format!("a=T,f=24,s={w},v={h},i={POPUP_ID},p=1,c={cols},r={rows},C=1,q=2,z=1");
                self.transmit(&mut out, &place, &rgb);
            }
        }
        drop(pixels);
        write_out(&out);
    }

    /// The escape codes of many images, compressed on up to 4 threads.
    /// Returns each one's codes, and whether it was sent blurry (a photo,
    /// through the pipe: half size first, sharp when the page is still).
    fn encode_all(&self, parts: &[Part]) -> Vec<(Vec<u8>, bool)> {
        let progressive =
            self.gfx == Gfx::KittyDirect && std::env::var_os("ZWEB_NOPROGRESSIVE").is_none();
        let encode = |p: &Part| {
            let keys = |w: usize, h: usize, src: &str| {
                format!("a=T,f=24,s={w},v={h},i={},{src}{}", p.id, p.place)
            };
            if progressive && p.inner.3 >= 2 {
                let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
                z.write_all(&p.rgb).unwrap();
                if z.finish().unwrap().len() * 4 > p.rgb.len() {
                    // Blurry: the tile alone, at half size.
                    let (ix, iy, iw, ih) = p.inner;
                    let tile: Vec<u8> = (iy..iy + ih)
                        .flat_map(|y| {
                            p.rgb[(y * p.w + ix) * 3..(y * p.w + ix + iw) * 3]
                                .iter()
                                .copied()
                        })
                        .collect();
                    let (hw, hh, small) = half(&tile, iw, ih);
                    let mut out = Vec::new();
                    self.transmit(&mut out, &keys(hw, hh, ""), &small);
                    return (out, true);
                }
            }
            let mut out = Vec::new();
            self.transmit(&mut out, &keys(p.w, p.h, &p.src), &p.rgb);
            (out, false)
        };
        let bytes: usize = parts.iter().map(|p| p.rgb.len()).sum();
        if parts.len() < 4 || bytes < 256 << 10 {
            return parts.iter().map(encode).collect();
        }
        let per = parts.len().div_ceil(4);
        std::thread::scope(|scope| {
            let handles: Vec<_> = parts
                .chunks(per)
                .map(|chunk| scope.spawn(move || chunk.iter().map(encode).collect::<Vec<_>>()))
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        })
    }

    /// One Kitty image: through shared memory, or zlib and base64 in chunks.
    fn transmit(&self, out: &mut Vec<u8>, place: &str, rgb: &[u8]) {
        if self.gfx == Gfx::KittyShm {
            if let Some(name) = shm_write(rgb) {
                let b64 = base64::engine::general_purpose::STANDARD.encode(name);
                out.extend_from_slice(
                    format!("\x1b_G{place},t=s,S={};{b64}\x1b\\", rgb.len()).as_bytes(),
                );
                return;
            }
        }
        let mut z = ZlibEncoder::new(Vec::with_capacity(rgb.len() / 4), Compression::fast());
        z.write_all(rgb).unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(z.finish().unwrap());
        let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let m = u8::from(i + 1 < chunks.len());
            let head = if i == 0 {
                format!("{place},o=z,m={m}")
            } else {
                format!("m={m},q=2")
            };
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

#[cfg(target_os = "macos")]
#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceLock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddress(surface: *mut c_void) -> *mut c_void;
    fn IOSurfaceGetBytesPerRow(surface: *mut c_void) -> usize;
}

/// A frame's BGRA pixels, locked for reading while this lives.
pub struct Pixels {
    #[cfg(target_os = "macos")]
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
            #[cfg(target_os = "macos")]
            FrameBuffer::IoSurface(surface) => unsafe {
                IOSurfaceLock(surface, 1, std::ptr::null_mut());
                let base = IOSurfaceGetBaseAddress(surface) as *const u8;
                Pixels {
                    surface: Some(surface),
                    base,
                    stride: IOSurfaceGetBytesPerRow(surface),
                    w,
                    h,
                }
            },
            #[cfg(not(target_os = "macos"))]
            FrameBuffer::IoSurface(_) => panic!("IOSurface frames require macOS"),
            FrameBuffer::SharedMemory { pixels, stride, .. } => Pixels {
                #[cfg(target_os = "macos")]
                surface: None,
                base: pixels.as_ptr(),
                stride,
                w,
                h,
            },
            FrameBuffer::Dmabuf { .. } => panic!("dmabuf frames are not handled"),
        }
    }

    fn at(&self, x: usize, y: usize) -> (u8, u8, u8) {
        unsafe {
            let p = self
                .base
                .add(y.min(self.h - 1) * self.stride + x.min(self.w - 1) * 4);
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
                let line = unsafe {
                    std::slice::from_raw_parts(self.base.add(y * self.stride), self.w * 4)
                };
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
            let src = unsafe {
                std::slice::from_raw_parts(self.base.add((y + row) * self.stride + x * 4), w * 4)
            };
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
        #[cfg(target_os = "macos")]
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
    let name = format!(
        "/zweb-{}-{}",
        std::process::id(),
        SHM_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let c = CString::new(name.clone()).unwrap();
    unsafe {
        let fd = libc::shm_open(
            c.as_ptr(),
            libc::O_CREAT | libc::O_RDWR | libc::O_EXCL,
            0o600,
        );
        if fd < 0 {
            return None;
        }
        if libc::ftruncate(fd, data.len() as libc::off_t) != 0 {
            libc::close(fd);
            libc::shm_unlink(c.as_ptr());
            return None;
        }
        let map = libc::mmap(
            std::ptr::null_mut(),
            data.len(),
            libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
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
fn blocks(
    px: &mut Pixels,
    out: &mut Vec<u8>,
    col: u16,
    row: u16,
    cols: u16,
    rows: u16,
    from: u16,
    to: u16,
) {
    let (w, h) = (px.w, px.h);
    // Average of a small grid in the cell half, for readable text.
    let sample = |cx: usize, half: usize| {
        let (x0, x1) = (
            cx * w / cols as usize,
            ((cx + 1) * w / cols as usize).max(cx * w / cols as usize + 1),
        );
        let total = rows as usize * 2;
        let (y0, y1) = (
            half * h / total,
            ((half + 1) * h / total).max(half * h / total + 1),
        );
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
                out.extend_from_slice(
                    format!(
                        "\x1b[38;2;{};{};{};48;2;{};{};{}m",
                        t.0, t.1, t.2, b.0, b.1, b.2
                    )
                    .as_bytes(),
                );
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
    fn half_averages_partial_edge_pixels() {
        let rgb: Vec<u8> = (0..9)
            .flat_map(|i| [i * 10, i * 10 + 1, i * 10 + 2])
            .collect();
        assert_eq!(
            half(&rgb, 3, 3),
            (2, 2, vec![20, 21, 22, 35, 36, 37, 65, 66, 67, 80, 81, 82])
        );
        assert_eq!(half(&[8, 4, 2], 1, 1), (1, 1, vec![8, 4, 2]));
    }

    #[test]
    fn tape_tracks_scroll_and_removes_replaced_signatures() {
        let mut tape = Tape::default();
        let fixed = [false; 8];
        tape.record(&[11, 21, 12, 22, 13, 23, 14, 24], &fixed, 2, &[]);
        let start = tape.offset.unwrap();
        tape.record(&[12, 22, 13, 23, 14, 24, 15, 25], &fixed, 2, &[]);
        assert_eq!(tape.offset, Some(start + 1));
        assert_eq!(tape.rows[&(start + 4)], vec![15, 25]);
        let old = Tape::signature(&[13, 23], &[false; 2]).unwrap();
        tape.expect = tape.offset;
        tape.record(&[12, 22, 100, 200, 14, 24, 15, 25], &fixed, 2, &[]);
        assert!(!tape.places.contains_key(&old));
        assert_eq!(tape.rows[&(start + 2)], vec![100, 200]);
    }

    #[test]
    fn tape_does_not_locate_a_frame_from_ambiguous_rows() {
        let mut tape = Tape::default();
        let real = [1, 2, 1, 2, 1, 2];
        tape.record(&real, &[false; 6], 2, &[]);
        let start = tape.offset;
        let signature = Tape::signature(&real[..2], &[false; 2]).unwrap();
        assert_eq!(tape.places[&signature], AMBIGUOUS);
        tape.record(&real, &[false; 6], 2, &[]);
        assert_ne!(tape.offset, start);
    }

    #[test]
    fn prediction_reuses_tape_tiles_and_holds_unknown_screens() {
        let renderer = Renderer::new(Gfx::KittyDirect);
        let layout = Layout {
            rows: 5,
            cols: 16,
            ..Layout::default()
        };
        let mut rows = Rows::new();
        rows.shown = (10..15).map(Some).collect();
        rows.fixed = vec![false; 5];
        for id in 10..16 {
            rows.cache.insert(
                id as u64,
                Cached {
                    id,
                    used: 0,
                    size: 3,
                    src: String::new(),
                },
            );
        }
        rows.tape.offset = Some(100);
        rows.tape.rows.insert(105, vec![15]);
        let mut out = Vec::new();
        renderer.show_prediction(&mut out, &layout, &mut rows, 1);
        assert_eq!(
            rows.shown,
            vec![Some(11), Some(12), Some(13), Some(14), Some(15)]
        );
        assert!(rows.prediction.as_ref().unwrap().shown);
        assert!(out.starts_with(
            b"\x1b[1;1H\x1b_Ga=p,i=11,p=1,c=16,r=1,C=1,q=2\x1b\\\x1b_Ga=d,d=i,i=10,p=1,q=2\x1b\\"
        ));
        out.clear();
        renderer.show_prediction(&mut out, &layout, &mut rows, 5);
        assert!(out.is_empty());
        assert!(!rows.prediction.as_ref().unwrap().shown);
        assert_eq!(
            rows.shown,
            vec![Some(11), Some(12), Some(13), Some(14), Some(15)]
        );
    }

    #[test]
    fn prediction_scores_partial_steps_and_recovers_after_hidden_hits() {
        let renderer = Renderer::new(Gfx::KittyDirect);
        let mut rows = Rows::new();
        rows.fixed = vec![false; 5];
        let mut prediction = Prediction {
            base: vec![1, 2, 3, 4, 5],
            steps: vec![1, 2],
            deadline: Instant::now() + SETTLE,
            held: None,
            shown: true,
            base_offset: Some(100),
            target: vec![3, 4, 5, BLANK, BLANK],
        };
        let partial = prediction.score(&[2, 3, 4, 5, 6], &rows.fixed, 1);
        assert!(partial.partial(&prediction));
        assert!(!partial.right());
        let real = [3, 4, 5, 6, 7];
        let complete = prediction.score(&real, &rows.fixed, 1);
        assert!(complete.right());
        renderer.record_prediction(&mut rows, &prediction, &complete, &real);
        assert_eq!(rows.tape.expect, Some(102));
        renderer.miss(&mut rows);
        renderer.miss(&mut rows);
        assert!(!renderer.predict_ok.load(Ordering::Relaxed));
        prediction.shown = false;
        for _ in 0..3 {
            renderer.record_prediction(&mut rows, &prediction, &complete, &real);
        }
        assert!(renderer.predict_ok.load(Ordering::Relaxed));
        assert_eq!(rows.misses, 0);
    }

    #[test]
    fn pending_keeps_clear_text_and_stop_when_page_is_hidden() {
        let renderer = Renderer::new(Gfx::KittyDirect);
        renderer.layout.lock().unwrap().hidden = true;
        *renderer.pending.lock().unwrap() = Pending {
            text: b"help".to_vec(),
            predict: 2,
            clear_all: true,
            clear_popup: true,
            forget: true,
            stop: true,
            ..Pending::default()
        };
        let (layout, pending) = renderer.wait_pending(&Rows::new());
        assert!(layout.hidden);
        assert_eq!(pending.predict, 0);
        assert_eq!(pending.text, b"help");
        assert!(pending.clear_all && pending.clear_popup && pending.forget && pending.stop);
        let queued = renderer.pending.lock().unwrap();
        assert!(queued.stop);
        assert!(queued.text.is_empty());
        assert!(!queued.clear_all && !queued.clear_popup && !queued.forget);
    }

    #[test]
    fn tiles_preserve_borders_and_place_cached_images_before_deleting_old_ones() {
        let renderer = Renderer::new(Gfx::KittyDirect);
        let layout = Layout {
            top: 1,
            cols: 2,
            rows: 2,
            ..Layout::default()
        };
        let mut rows = Rows::new();
        let mut out = Vec::new();
        renderer.prepare_page((4, 4), &layout, &mut rows, &mut out);
        let bgra: Vec<u8> = (0..16).flat_map(|i| [i, i + 1, i + 2, 255]).collect();
        let mut pixels = Pixels {
            #[cfg(target_os = "macos")]
            surface: None,
            base: bgra.as_ptr(),
            stride: 16,
            w: 4,
            h: 4,
        };
        let (parts, sent, moved) =
            renderer.draw_tiles(&mut pixels, &layout, &mut rows, &[101, 102], &mut out);
        assert_eq!((sent, moved, rows.bytes), (2, 0, 72));
        assert_eq!(parts[0].inner, (0, 0, 4, 2));
        assert_eq!(parts[1].inner, (0, 1, 4, 2));
        assert_eq!(&parts[0].rgb[..3], &[2, 1, 0]);
        assert_eq!(&parts[1].rgb[..3], &[6, 5, 4]);
        let (unchanged, sent, moved) =
            renderer.draw_tiles(&mut pixels, &layout, &mut rows, &[101, 102], &mut out);
        assert!(unchanged.is_empty() && out.is_empty());
        assert_eq!((sent, moved), (0, 0));
        let (reused, sent, moved) =
            renderer.draw_tiles(&mut pixels, &layout, &mut rows, &[102, 101], &mut out);
        assert!(reused.is_empty());
        assert_eq!((sent, moved), (0, 2));
        assert_eq!(
            out,
            concat!(
                "\x1b[2;1H\x1b_Ga=p,i=11,x=0,y=1,w=4,h=2,p=1,c=2,r=1,C=1,q=2\x1b\\",
                "\x1b_Ga=d,d=i,i=10,p=1,q=2\x1b\\",
                "\x1b[3;1H\x1b_Ga=p,i=10,x=0,y=0,w=4,h=2,p=2,c=2,r=1,C=1,q=2\x1b\\",
                "\x1b_Ga=d,d=i,i=11,p=2,q=2\x1b\\",
            )
            .as_bytes()
        );
    }

    #[test]
    fn shift_keeps_fixed_rows_and_blanks_new_ones() {
        let base = [10, 1, 2, 3, 4, 20];
        let fixed = [true, false, false, false, false, true];
        assert_eq!(
            shifted(&base, &fixed, 1, 2),
            vec![10, 3, 4, BLANK, BLANK, 20]
        );
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
