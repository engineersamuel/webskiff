//! Probe: how well and how fast zebkit works as the engine of a terminal browser.
//!
//! zebkit-probe text  <url>              page text (innerText) to stdout
//! zebkit-probe kitty <url> [cols]       page picture via the Kitty graphics protocol
//! zebkit-probe png   <url> <file.png>   page picture to a PNG file
//! zebkit-probe scroll <url>             time from a wheel event to its new frame, 10 times
//!
//! Timings go to stderr.

use std::cell::Cell;
use std::ffi::c_void;
use std::io::Write;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use base64::Engine as _;
use zebkit::offscreen::{Frame, FrameBuffer, Modifiers, WheelPhase};
use zebkit::services::{encode_image, Image, ImageFormat};
use zebkit::{LoadEvent, View};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
const TIMEOUT: Duration = Duration::from_secs(30);

static START: OnceLock<Instant> = OnceLock::new();

fn mark(what: &str) {
    let ms = START.get().unwrap().elapsed().as_secs_f64() * 1000.0;
    eprintln!("[{ms:8.1} ms] {what}");
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Text,
    Kitty,
    Png,
    Scroll,
}

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceLock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(surface: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddress(surface: *mut c_void) -> *mut c_void;
    fn IOSurfaceGetBytesPerRow(surface: *mut c_void) -> usize;
}
const READ_ONLY: u32 = 1;

/// The frame's BGRA pixels as tightly packed RGBA.
fn frame_rgba(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    let mut copy = |base: *const u8, stride: usize| {
        for y in 0..h {
            let row = unsafe { std::slice::from_raw_parts(base.add(y * stride), w * 4) };
            for px in row.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
    };
    match frame.buffer() {
        FrameBuffer::IoSurface(surface) => unsafe {
            IOSurfaceLock(surface, READ_ONLY, std::ptr::null_mut());
            copy(IOSurfaceGetBaseAddress(surface) as *const u8, IOSurfaceGetBytesPerRow(surface));
            IOSurfaceUnlock(surface, READ_ONLY, std::ptr::null_mut());
        },
        FrameBuffer::SharedMemory { pixels, stride, .. } => copy(pixels.as_ptr(), stride),
        FrameBuffer::Dmabuf { .. } => panic!("dmabuf frames are not handled"),
    }
    rgba
}

fn kitty(png: &[u8], cols: u32) {
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    let mut out = std::io::stdout().lock();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let head = if i == 0 { format!("f=100,a=T,c={cols},m={more}") } else { format!("m={more}") };
        write!(out, "\x1b_G{head};").unwrap();
        out.write_all(chunk).unwrap();
        write!(out, "\x1b\\").unwrap();
    }
    writeln!(out).unwrap();
}

fn capture(frame: &Frame, mode: Mode, arg: &str) {
    mark(&format!("frame {}x{} @{} (seq {})", frame.width(), frame.height(), frame.scale(), frame.sequence()));
    let rgba = frame_rgba(frame);
    mark("pixels copied (BGRA -> RGBA)");
    let image = Image { width: frame.width(), height: frame.height(), rgba };
    let png = encode_image(ImageFormat::Png, &image, 0).expect("encode PNG");
    mark(&format!("PNG encoded ({} KB)", png.len() / 1024));
    match mode {
        Mode::Png => {
            std::fs::write(arg, &png).expect("write PNG");
            mark(&format!("written to {arg}"));
        }
        Mode::Kitty => {
            kitty(&png, arg.parse().unwrap_or(100));
            mark("sent to terminal");
        }
        Mode::Text | Mode::Scroll => unreachable!(),
    }
}

fn main() {
    START.set(Instant::now()).unwrap();
    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: zebkit-probe text|kitty|png|scroll <url> [cols|file.png]";
    let mode = match args.get(1).map(String::as_str) {
        Some("text") => Mode::Text,
        Some("kitty") => Mode::Kitty,
        Some("png") => Mode::Png,
        Some("scroll") => Mode::Scroll,
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let url = args.get(2).expect(usage).clone();
    let arg = args.get(3).cloned().unwrap_or_else(|| "probe.png".into());

    let code = zebkit::run_with_args(["zebkit-probe", "--headless", "--zebkit-cookie-storage=file"], move || {
        mark("engine ready");
        zebkit::after(TIMEOUT, || {
            mark("TIMEOUT");
            zebkit::quit();
        });
        let view = View::new_offscreen(None, WIDTH, HEIGHT, 1.0).expect("off-screen view");
        let loaded = Rc::new(Cell::new(false));
        let done = Rc::new(Cell::new(false));
        let frames = Rc::new(Cell::new(0u32));

        // Scroll mode: when the wheel event was sent, and the latencies so far.
        let sent: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let latencies: Rc<std::cell::RefCell<Vec<f64>>> = Rc::default();
        let wheel = {
            let (view, sent) = (view.clone(), sent.clone());
            Rc::new(move || {
                sent.set(Some(Instant::now()));
                view.send_wheel(640.0, 400.0, 0.0, 200.0, false, WheelPhase::None, Modifiers::default());
            })
        };

        if mode == Mode::Scroll {
            let page = view.clone();
            let (sent, latencies, wheel) = (sent.clone(), latencies.clone(), wheel.clone());
            view.on_frame(move |frame| {
                let Some(at) = sent.take() else { return };
                let got = at.elapsed().as_secs_f64() * 1000.0;
                let t = Instant::now();
                let rgba = frame_rgba(&frame);
                let png = encode_image(ImageFormat::Png, &Image { width: frame.width(), height: frame.height(), rgba }, 0).unwrap();
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png).len();
                let prep = t.elapsed().as_secs_f64() * 1000.0;
                eprintln!("  wheel -> frame {got:6.1} ms | copy+PNG+base64 {prep:5.1} ms ({} KB to terminal) | damage {:?}", b64 / 1024, frame.damage().len());
                latencies.borrow_mut().push(got);
                drop(frame);
                if latencies.borrow().len() < 10 {
                    let wheel = wheel.clone();
                    zebkit::after(Duration::from_millis(100), move || wheel());
                } else {
                    let mut v = latencies.borrow().clone();
                    v.sort_by(f64::total_cmp);
                    eprintln!("  wheel -> frame: min {:.1} median {:.1} max {:.1} ms", v[0], v[v.len() / 2], v[v.len() - 1]);
                    page.evaluate("scrollY", |y| {
                        eprintln!("  page scrollY after 10 x 200 px wheel: {y:?}");
                        zebkit::quit();
                    });
                }
            });
        } else if mode != Mode::Text {
            let (loaded, done, frames, arg) = (loaded.clone(), done.clone(), frames.clone(), arg.clone());
            view.on_frame(move |frame| {
                frames.set(frames.get() + 1);
                if frames.get() == 1 {
                    mark("first frame (before load finished)");
                }
                if loaded.get() && !done.get() {
                    done.set(true);
                    capture(&frame, mode, &arg);
                    zebkit::quit();
                }
            });
        }

        let page = view.clone();
        view.on_load(move |event, detail| {
            mark(&format!("load {event:?} {detail}"));
            match event {
                LoadEvent::Finished if !loaded.get() => {
                    loaded.set(true);
                    if mode == Mode::Text {
                        page.evaluate("document.body ? document.body.innerText : ''", |result| {
                            mark("text extracted");
                            match result {
                                Ok(json) => {
                                    let text: String = serde_json::from_str(&json).unwrap_or(json);
                                    eprintln!("               {} chars, {} lines", text.len(), text.lines().count());
                                    println!("{text}");
                                }
                                Err(e) => eprintln!("evaluate failed: {e:?}"),
                            }
                            zebkit::quit();
                        });
                    } else if mode == Mode::Scroll {
                        page.set_focus(true);
                        let wheel = wheel.clone();
                        zebkit::after(Duration::from_millis(1500), move || wheel());
                    } else {
                        page.request_frame();
                    }
                }
                LoadEvent::Failed => zebkit::quit(),
                _ => {}
            }
        });
        view.load_url(&url);
        mark("load requested");
        std::mem::forget(view);
    });
    mark(&format!("engine stopped (exit {code})"));
    std::process::exit(code);
}
