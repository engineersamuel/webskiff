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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

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
}

const POPUP_ID: u32 = 1;
/// Raw pixel bytes of row images the terminal may keep for reuse.
const CACHE_BYTES: usize = 64 << 20;

struct Cached {
    id: u32,
    used: u64,
    size: usize,
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
}

impl Rows {
    fn new() -> Rows {
        Rows { key: None, shown: Vec::new(), cache: HashMap::new(), bytes: 0, next_id: 10, clock: 0 }
    }

    /// Forgets everything (the terminal's images were all deleted).
    fn reset(&mut self) {
        self.key = None;
        self.shown.clear();
        self.cache.clear();
        self.bytes = 0;
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
            while p.page.is_none() && p.popup.is_none() && p.text.is_empty() && !p.clear_popup && !p.clear_all && !p.stop {
                p = self.wake.wait(p).unwrap();
            }
            let layout = *self.layout.lock().unwrap();
            if layout.hidden {
                p.page = None;
                p.popup = None;
            }
            let page = p.page.take();
            let text = std::mem::take(&mut p.text);
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
            if clear_popup {
                if self.gfx == Gfx::Blocks {
                    // The popup was drawn over page cells: draw them all again.
                    rows.shown.fill(None);
                } else {
                    out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={POPUP_ID},q=2\x1b\\").as_bytes());
                }
            }
            out.extend_from_slice(&text);
            if !out.is_empty() {
                write_out(&out);
            }
            if stop {
                return;
            }
            if let Some(frame) = page {
                self.draw_page(&frame, &layout, &mut rows);
            }
            if let (Some(frame), Some(_)) = (popup, layout.popup) {
                self.draw_popup(&frame, &layout);
            }
        }
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
            rows.shown = vec![None; layout.rows as usize];
        }
        // Pixel row where cell row `r` of the page starts.
        let px_row = |r: u16| (r as u64 * h as u64 / layout.rows as u64) as usize;
        let (mut sent, mut moved) = (0u16, 0u16);
        let mut pixels = Pixels::new(frame);
        for r in 0..layout.rows {
            let (y0, y1) = (px_row(r), px_row(r + 1));
            let hash = pixels.hash(y0, y1);
            let old = rows.shown[r as usize];
            if old == Some(hash) {
                continue;
            }
            rows.shown[r as usize] = Some(hash);
            if self.gfx == Gfx::Blocks {
                blocks(&mut pixels, &mut out, 0, layout.top, layout.cols, layout.rows, r, r + 1);
                sent += 1;
                continue;
            }
            rows.clock += 1;
            out.extend_from_slice(format!("\x1b[{};1H", layout.top + r + 1).as_bytes());
            let place = format!("p={},c={},r=1,C=1,q=2", r + 1, layout.cols);
            if let Some(c) = rows.cache.get_mut(&hash) {
                c.used = rows.clock;
                out.extend_from_slice(format!("\x1b_Ga=p,i={},{place}\x1b\\", c.id).as_bytes());
                moved += 1;
            } else {
                let id = rows.next_id;
                rows.next_id += 1;
                let rgb = pixels.rgb(0, y0, w as usize, y1 - y0);
                self.transmit(&mut out, &format!("a=T,f=24,s={w},v={},i={id},{place}", y1 - y0), &rgb);
                rows.bytes += rgb.len();
                rows.cache.insert(hash, Cached { id, used: rows.clock, size: rgb.len() });
                sent += 1;
            }
            // The row's previous placement goes after the new one is there: no
            // flicker. Lowercase: the image stays in the terminal for reuse.
            if let Some(c) = old.and_then(|o| rows.cache.get(&o)) {
                out.extend_from_slice(format!("\x1b_Ga=d,d=i,i={},p={},q=2\x1b\\", c.id, r + 1).as_bytes());
            }
        }
        drop(pixels);
        // Over the budget: free the least recently used images not on screen.
        if rows.bytes > CACHE_BYTES {
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
        write_out(&out);
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

    /// Hash of the pixels of rows `y0..y1`, full width.
    fn hash(&self, y0: usize, y1: usize) -> u64 {
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        for y in y0..y1 {
            hasher.update(unsafe { std::slice::from_raw_parts(self.base.add(y * self.stride), self.w * 4) });
        }
        hasher.digest() ^ ((y1 - y0) as u64) << 48
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
