//! Draws page frames in the terminal, on a thread of its own.
//!
//! Kitty graphics: raw RGB, zlib-compressed and base64 in the escape codes
//! (works everywhere, also over SSH), or through POSIX shared memory when the
//! terminal says it reads it (local only, nearly no bytes on the pipe).
//! Blocks: `▀` cells with truecolor, for terminals without graphics.

use std::ffi::{c_void, CString};
use std::io::Write;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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
    /// Strips sent for the last frame.
    pub strips: u16,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
struct Pending {
    page: Option<Frame>,
    popup: Option<Frame>,
    /// Damage of page frames not drawn yet, newest frame's included.
    damage: Vec<(i32, i32, i32, i32)>,
    /// Redraw the whole page with the next frame.
    full: bool,
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
    /// Size and place of the last page drawn; another one means a full redraw.
    drawn: Mutex<Option<(u32, u32, u16, u16, u16)>>,
    /// Which of a strip's two image ids is shown (bit per strip).
    flip: AtomicU64,
}

/// Cell rows per image strip: a small change resends one strip, not the page.
const STRIP_ROWS: u16 = 4;
const POPUP_ID: u32 = 1000;

fn strip_id(strip: u16, alt: bool) -> u32 {
    1 + strip as u32 * 2 + alt as u32
}

impl Renderer {
    pub fn new(gfx: Gfx) -> Renderer {
        Renderer {
            gfx,
            pending: Mutex::new(Pending { full: true, ..Pending::default() }),
            wake: Condvar::new(),
            layout: Mutex::default(),
            stats: Mutex::default(),
            drawn: Mutex::new(None),
            flip: AtomicU64::new(0),
        }
    }

    /// Keeps only the newest frame of each layer; an older one is released,
    /// its damage kept.
    pub fn submit(&self, frame: Frame) {
        let mut p = self.pending.lock().unwrap();
        match frame.layer() {
            Layer::View => {
                p.damage.extend(frame.damage().iter().map(|r| (r.x, r.y, r.width, r.height)));
                p.page = Some(frame);
            }
            Layer::Popup => p.popup = Some(frame),
        }
        self.wake.notify_one();
    }

    pub fn clear_popup(&self) {
        self.pending.lock().unwrap().clear_popup = true;
        self.wake.notify_one();
    }

    /// Removes every image (before the help screen, a resize, quitting); the
    /// next page frame is drawn whole.
    pub fn clear_all(&self) {
        let mut p = self.pending.lock().unwrap();
        p.clear_all = true;
        p.full = true;
        p.page = None;
        p.popup = None;
        p.damage.clear();
        self.wake.notify_one();
    }

    pub fn stop(&self) {
        self.pending.lock().unwrap().stop = true;
        self.wake.notify_one();
    }

    pub fn run(&self) {
        loop {
            let mut p = self.pending.lock().unwrap();
            while p.page.is_none() && p.popup.is_none() && !p.clear_popup && !p.clear_all && !p.stop {
                p = self.wake.wait(p).unwrap();
            }
            if p.stop {
                return;
            }
            let layout = *self.layout.lock().unwrap();
            if layout.hidden {
                // Keep the damage: the page is drawn whole when shown again.
                p.page = None;
                p.popup = None;
            }
            let page = p.page.take();
            let damage = if page.is_some() { std::mem::take(&mut p.damage) } else { Vec::new() };
            let full = page.is_some() && std::mem::take(&mut p.full);
            let (popup, clear_popup, clear_all) =
                (p.popup.take(), std::mem::take(&mut p.clear_popup), std::mem::take(&mut p.clear_all));
            drop(p);
            if clear_all {
                *self.drawn.lock().unwrap() = None;
                if self.gfx != Gfx::Blocks {
                    write_out(b"\x1b_Ga=d,d=A,q=2\x1b\\");
                }
            }
            if clear_popup && self.gfx != Gfx::Blocks {
                write_out(format!("\x1b_Ga=d,d=I,i={POPUP_ID},q=2\x1b\\").as_bytes());
            }
            if layout.hidden {
                continue;
            }
            if let Some(frame) = page {
                self.draw_page(&frame, &layout, full, &damage);
            }
            if let (Some(frame), Some(_)) = (popup, layout.popup) {
                self.draw_popup(&frame, &layout);
            }
        }
    }

    fn draw_page(&self, frame: &Frame, layout: &Layout, full: bool, damage: &[(i32, i32, i32, i32)]) {
        let started = Instant::now();
        let (w, h) = (frame.width(), frame.height());
        if w == 0 || h == 0 || layout.cols == 0 || layout.rows == 0 {
            return;
        }
        let key = (w, h, layout.top, layout.cols, layout.rows);
        let full = full || *self.drawn.lock().unwrap() != Some(key);
        // Pixel row where cell row `r` of the page starts.
        let px_row = |r: u16| (r as u64 * h as u64 / layout.rows as u64) as i32;
        // At most 64 strips: one flip bit each.
        let strip_rows = STRIP_ROWS.max(layout.rows.div_ceil(64));
        let strips = layout.rows.div_ceil(strip_rows);
        let mut out = Vec::new();
        let mut sent = 0;
        let mut pixels = Pixels::new(frame);
        for strip in 0..strips {
            let (r0, r1) = (strip * strip_rows, ((strip + 1) * strip_rows).min(layout.rows));
            let (y0, y1) = (px_row(r0), px_row(r1));
            let dirty = full || damage.iter().any(|&(_, dy, _, dh)| dy < y1 && dy + dh > y0);
            if !dirty {
                continue;
            }
            sent += 1;
            match self.gfx {
                Gfx::Blocks => blocks(&mut pixels, &mut out, 0, layout.top, layout.cols, layout.rows, r0, r1),
                Gfx::KittyDirect | Gfx::KittyShm => {
                    let alt = self.flip.fetch_xor(1 << strip, Ordering::Relaxed) & (1 << strip) == 0;
                    let rgb = pixels.rgb(0, y0 as usize, w as usize, (y1 - y0) as usize);
                    out.extend_from_slice(format!("\x1b[{};1H", layout.top + r0 + 1).as_bytes());
                    let place = format!(
                        "a=T,f=24,s={w},v={},i={},p=1,c={},r={},C=1,q=2",
                        y1 - y0,
                        strip_id(strip, alt),
                        layout.cols,
                        r1 - r0
                    );
                    self.transmit(&mut out, &place, &rgb);
                    // The strip's previous image goes after the new one is placed: no flicker.
                    out.extend_from_slice(format!("\x1b_Ga=d,d=I,i={},q=2\x1b\\", strip_id(strip, !alt)).as_bytes());
                }
            }
        }
        drop(pixels);
        if sent == 0 {
            return;
        }
        write_out(&out);
        *self.drawn.lock().unwrap() = Some(key);
        let mut s = self.stats.lock().unwrap();
        s.frames += 1;
        s.last_ms = started.elapsed().as_secs_f64() * 1000.0;
        s.last_bytes = out.len();
        s.strips = sent;
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
