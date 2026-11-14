//! zweb: a web browser in the terminal, on zebkit's off-screen views.
//!
//! zweb [url] [--gfx kitty|shm|blocks] [--scale N]
//! zweb --selftest      checks input, hints, popups, dialogs and frame cost headless

/// A JS function: is the element a text field (editable)?
macro_rules! editable {
    () => {
        "(e=>!!e&&(e.isContentEditable||e.tagName==='TEXTAREA'||(e.tagName==='INPUT'&&\
         !/^(button|submit|reset|checkbox|radio|file|image|range|color)$/i.test(e.type))))"
    };
}

mod app;
mod cli;
mod config;
mod edit;
mod history;
mod input;
mod keys;
mod render;
mod selftest;
mod trace;

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{cursor, execute, terminal};

use app::{send, App, Msg, Options, APP};
use render::{Gfx, Renderer};

fn data_dir() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
        .join(".cache/zweb");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Engine logs go to a file: on the terminal they would break the screen.
/// Returns a copy of the real stderr, for panics.
fn redirect_stderr() -> i32 {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir().join("zweb.log"))
        .unwrap();
    unsafe {
        let saved = libc::dup(2);
        libc::dup2(log.as_raw_fd(), 2);
        saved
    }
}

fn enter_screen(enhanced: bool) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    execute!(
        out,
        terminal::EnterAlternateScreen,
        cursor::Hide,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    if enhanced {
        let _ = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    Ok(())
}

fn restore_terminal(enhanced: bool) {
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b_Ga=d,d=A,q=2\x1b\\");
    if enhanced {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        out,
        DisableBracketedPaste,
        DisableMouseCapture,
        cursor::Show,
        terminal::LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

/// Asks the terminal (raw mode) whether it shows Kitty images, and whether it
/// reads them from shared memory. The DA1 answer ends the wait.
fn query_graphics() -> (bool, bool) {
    let probe = render::shm_write(&[0, 0, 0]);
    let mut q = String::from("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    if let Some(name) = &probe {
        let b64 = base64::engine::general_purpose::STANDARD.encode(name);
        q += &format!("\x1b_Gi=32,s=1,v=1,a=q,t=s,f=24,S=3;{b64}\x1b\\");
    }
    q += "\x1b[c";
    let mut out = std::io::stdout();
    let _ = out.write_all(q.as_bytes());
    let _ = out.flush();
    let mut reply = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut stdin = std::io::stdin();
    while Instant::now() < deadline {
        let mut fds = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fds, 1, 50) } <= 0 {
            continue;
        }
        let mut buf = [0u8; 256];
        let n = stdin.read(&mut buf).unwrap_or(0);
        reply.extend_from_slice(&buf[..n]);
        let s = String::from_utf8_lossy(&reply);
        if let Some(i) = s.find("\x1b[?") {
            if s[i..].contains('c') {
                break;
            }
        }
    }
    if let Some(name) = probe {
        let c = std::ffi::CString::new(name).unwrap();
        unsafe { libc::shm_unlink(c.as_ptr()) };
    }
    let s = String::from_utf8_lossy(&reply);
    (s.contains("i=31;OK"), s.contains("i=32;OK"))
}

fn main() {
    let command = cli::parse(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("webskiff: {error}\nRun webskiff --help for usage.");
        std::process::exit(2);
    });
    let (address, requested_gfx, scale, placeholders) = match command {
        cli::Command::Help => {
            println!("{}", cli::HELP);
            return;
        }
        cli::Command::Version => {
            println!(
                "webskiff {} (zebkit {}; {}; {}; {})",
                env!("CARGO_PKG_VERSION"),
                zebkit::version(),
                zebkit::engine_version(),
                env!("WEBSKIFF_BUILD_TARGET"),
                env!("WEBSKIFF_BUILD_COMMIT")
            );
            return;
        }
        cli::Command::Selftest => std::process::exit(selftest::run()),
        cli::Command::Browse {
            address,
            gfx,
            scale,
            placeholders,
        } => (address, gfx, scale, placeholders),
    };
    trace::init();
    let url = match address.as_deref().map(app::to_url).transpose() {
        Ok(url) => url.unwrap_or_else(|| app::home().into()),
        Err(error) => {
            eprintln!("webskiff: cannot open file: {error}");
            std::process::exit(1);
        }
    };
    let config = match config::Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("webskiff: {error}");
            std::process::exit(1);
        }
    };

    let saved_stderr = redirect_stderr();
    terminal::enable_raw_mode().expect("raw mode");
    let (kitty, shm) = query_graphics();
    let known_kitty = matches!(
        std::env::var("TERM_PROGRAM").as_deref(),
        Ok("herdr" | "ghostty" | "WezTerm")
    ) || std::env::var("KITTY_WINDOW_ID").is_ok();
    let gfx = requested_gfx.unwrap_or({
        if shm {
            Gfx::KittyShm
        } else if kitty || known_kitty {
            Gfx::KittyDirect
        } else {
            Gfx::Blocks
        }
    });
    let enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
    enter_screen(enhanced).unwrap();
    eprintln!("zweb start: kitty={kitty} shm={shm} known={known_kitty} gfx={gfx:?} enhanced_keys={enhanced}");

    std::panic::set_hook(Box::new(move |info| {
        restore_terminal(enhanced);
        let msg = format!("zweb panicked: {info}\n");
        unsafe { libc::write(saved_stderr, msg.as_ptr().cast(), msg.len()) };
        eprint!("{msg}");
    }));

    let renderer = Arc::new(Renderer::new(gfx));
    let render_thread = {
        let r = renderer.clone();
        std::thread::spawn(move || r.run())
    };

    let profile = format!("--user-data-dir={}", data_dir().join("profile").display());
    let r = renderer.clone();
    // Cookie key in a file of the profile, not the macOS Keychain: a rebuilt
    // binary would make the Keychain ask again, and loads wait for the answer.
    let mut engine_args: Vec<String> = [
        "zweb",
        "--headless",
        profile.as_str(),
        "--zebkit-cookie-storage=file",
        // One frame per scroll step, not ~300 ms of animation frames to send.
        "--disable-smooth-scrolling",
        // Frames sooner after input: no wait for a GPU vsync, and the page
        // drawn in memory, so its capture needs no read back from the GPU.
        "--disable-gpu-vsync",
        "--disable-gpu-compositing",
        // As macOS shows them when not scrolling: none. A moving thumb would
        // also change rows that scrolling otherwise only moves.
        "--hide-scrollbars",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    // For experiments: more Chromium switches.
    if let Ok(extra) = std::env::var("ZWEB_ARGS") {
        engine_args.extend(extra.split_whitespace().map(String::from));
    }
    let code = zebkit::run_with_args(engine_args, move || {
        APP.with(|a| {
            let mut app = App::new(
                r,
                Options {
                    scale,
                    placeholders,
                    enhanced,
                },
                config,
            );
            app.relayout();
            app.open_tab(Some(&url), None);
            *a.borrow_mut() = Some(app);
        });
        // Terminal input on its own thread, each event posted to the engine's.
        // No reads while an external editor has the terminal (edit.rs).
        std::thread::spawn(|| loop {
            if edit::INPUT_PAUSED.load(Ordering::SeqCst) {
                edit::INPUT_IDLE.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            edit::INPUT_IDLE.store(false, Ordering::SeqCst);
            match event::poll(Duration::from_millis(50))
                .and_then(|ready| ready.then(event::read).transpose())
            {
                Ok(Some(ev)) => {
                    trace!("input read {ev:?}");
                    zebkit::post(move || send(Msg::Term(ev)))
                }
                Ok(None) => {}
                Err(_) => return,
            }
        });
        fn tick() {
            send(Msg::Tick);
            zebkit::after(Duration::from_millis(500), tick);
        }
        tick();
    });
    renderer.stop();
    let _ = render_thread.join();
    let _out = std::io::stdout().lock();
    restore_terminal(enhanced);
    eprintln!("zweb stop: exit {code}");
    std::process::exit(code);
}
