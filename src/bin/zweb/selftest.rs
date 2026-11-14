//! `zweb --selftest`: drives the real key mapping, hint script, clicks,
//! popups, dialogs and frame encoding against a test page, headless.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use flate2::{write::ZlibEncoder, Compression};
use zebkit::offscreen::{Modifiers, MouseButton, MouseEvent as ZMouse};
use zebkit::page::DialogPolicy;
use zebkit::{LoadEvent, View};

use crate::app::{visual_script, yank, FIND_JS, HINTS_JS, VISUAL_START_JS, YANK_JS};
use crate::config::Action;
use crate::{keys, render};

const PAGE: &str = r#"<!doctype html><title>t</title><style>body{margin:0;font:16px sans-serif}
#f{position:absolute;left:10px;top:10px;width:200px;height:20px}
#b,#s,#w{position:absolute;left:10px;width:100px;height:30px}#b{top:60px}#s{top:110px}#w{top:160px}</style>
<form onsubmit="document.title='submitted:'+f.value;return false"><input id=f></form>
<button id=b onclick="this.textContent='clicked'">button</button>
<select id=s><option>one</option><option>two</option></select>
<button id=w onclick="window.open('about:blank')">popup</button>
<a id=t1 href=#a onclick="window.followed=1;return false" style="position:absolute;top:2000px">find target</a>
<a id=t2 href=#b onclick="window.followed=2;return false" style="position:absolute;top:2100px">other target</a>
<p id=v style="position:absolute;top:300px">alpha bravo charlie</p>"#;

#[derive(Default)]
struct Ctx {
    caret: Cell<bool>,
    popup: Cell<bool>,
    opened: Cell<bool>,
    dialog: Cell<Option<u64>>,
    measure: Cell<bool>,
    fails: Cell<i32>,
    view: RefCell<Option<View>>,
}

type Act = Box<dyn Fn(&Rc<Ctx>, &View)>;
enum Check {
    Js(&'static str, &'static str),
    Flag(fn(&Ctx) -> bool),
    None,
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn press(view: &View, k: KeyEvent) {
    keys::send(view, &keys::to_dom(&k).expect("mapped key"));
}

fn typed(view: &View, text: &str) {
    for c in text.chars() {
        press(view, key(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn click(view: &View, x: f32, y: f32) {
    view.send_mouse(ZMouse::Move, x, y, MouseButton::None, 0, Modifiers::NONE);
    view.send_mouse(ZMouse::Down, x, y, MouseButton::Left, 1, Modifiers::NONE);
    view.send_mouse(ZMouse::Up, x, y, MouseButton::Left, 1, Modifiers::NONE);
}

fn steps() -> Vec<(&'static str, Act, Check)> {
    vec![
        (
            "click focuses the input",
            Box::new(|_, v| click(v, 110.0, 20.0)),
            Check::Js("document.activeElement.id", "\"f\""),
        ),
        (
            "caret callback fires",
            Box::new(|_, _| {}),
            Check::Flag(|c| c.caret.get()),
        ),
        (
            "typing text",
            Box::new(|_, v| typed(v, "Hi there")),
            Check::Js("f.value", "\"Hi there\""),
        ),
        (
            "Backspace",
            Box::new(|_, v| press(v, key(KeyCode::Backspace, KeyModifiers::NONE))),
            Check::Js("f.value", "\"Hi ther\""),
        ),
        (
            "arrow keys move the caret",
            Box::new(|_, v| {
                press(v, key(KeyCode::Left, KeyModifiers::NONE));
                press(v, key(KeyCode::Left, KeyModifiers::NONE));
                typed(v, "X");
            }),
            Check::Js("f.value", "\"Hi thXer\""),
        ),
        (
            "paste (bracketed paste -> IME commit)",
            Box::new(|_, v| v.ime_commit(" ✓ok")),
            Check::Js("f.value", "\"Hi thX ✓oker\""),
        ),
        (
            "Ctrl-A selects all, then typing replaces",
            Box::new(|_, v| {
                press(v, key(KeyCode::Char('a'), KeyModifiers::CONTROL));
                typed(v, "z");
            }),
            Check::Js("f.value", "\"z\""),
        ),
        (
            "Enter submits the form",
            Box::new(|_, v| press(v, key(KeyCode::Enter, KeyModifiers::NONE))),
            Check::Js("document.title", "\"submitted:z\""),
        ),
        (
            "hint script finds 4 targets and its click works",
            Box::new(|_, v| {
                let page = v.clone();
                v.evaluate(HINTS_JS, move |r| {
                    let list: Vec<serde_json::Value> = r
                        .ok()
                        .and_then(|j| serde_json::from_str::<String>(&j).ok())
                        .and_then(|j| serde_json::from_str(&j).ok())
                        .unwrap_or_default();
                    println!(
                        "      hints: {}",
                        list.iter()
                            .map(|h| h["k"].as_str().unwrap_or("?"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                    let rendered = list.iter().all(|h| h["k"].is_string());
                    if let Some(h) = list
                        .iter()
                        .find(|h| (h["y"].as_f64().unwrap_or(0.0) - 75.0).abs() < 10.0)
                    {
                        click(
                            &page,
                            h["x"].as_f64().unwrap() as f32,
                            h["y"].as_f64().unwrap() as f32,
                        );
                    }
                    page.evaluate(
                        &format!("window.hintCount={}", if rendered { list.len() } else { 0 }),
                        |_| {},
                    );
                });
            }),
            Check::Js(
                "[window.hintCount, b.textContent, !!window.__zwebHop].join()",
                "\"4,clicked,true\"",
            ),
        ),
        (
            "select opens a popup layer",
            Box::new(|_, v| {
                v.evaluate("__zwebHop.clear()", |_| {});
                click(v, 60.0, 125.0)
            }),
            Check::Flag(|c| c.popup.get()),
        ),
        (
            "keys go to the popup: Down, Enter picks 'two'",
            Box::new(|_, v| {
                press(v, key(KeyCode::Down, KeyModifiers::NONE));
                press(v, key(KeyCode::Enter, KeyModifiers::NONE));
            }),
            Check::Js("s.value", "\"two\""),
        ),
        (
            "popup closed after choosing",
            Box::new(|_, _| {}),
            Check::Flag(|c| !c.popup.get()),
        ),
        (
            "window.open becomes a new view",
            Box::new(|_, v| click(v, 60.0, 175.0)),
            Check::Flag(|c| c.opened.get()),
        ),
        (
            "confirm() is held for the app",
            Box::new(|_, v| v.evaluate("setTimeout(()=>{window.r=confirm('ok?')},10)", |_| {})),
            Check::Flag(|c| c.dialog.get().is_some()),
        ),
        (
            "answering it returns true to the page",
            Box::new(|c, v| {
                v.respond_dialog(c.dialog.get().unwrap_or(0), true, None);
            }),
            Check::Js("window.r", "true"),
        ),
        (
            "accepted find marks every match yellow",
            Box::new(|_, v| {
                v.evaluate(FIND_JS, |_| {});
                v.evaluate("__zwebFind.preview('target')", |_| {});
                v.evaluate("__zwebFind.finish(true, 'target')", |_| {});
            }),
            Check::Js("CSS.highlights.get('zweb-find').size", "2"),
        ),
        (
            "n moves to the next match and focuses its link",
            Box::new(|_, v| {
                let lib = FIND_JS.split("\n// SEARCH SESSION").next().unwrap();
                v.evaluate(&format!("{lib};__zwebSearch.step('target', false)"), |_| {});
            }),
            Check::Js("document.activeElement.id", "\"t2\""),
        ),
        (
            "Enter opens the link at the match",
            Box::new(|_, v| press(v, key(KeyCode::Enter, KeyModifiers::NONE))),
            Check::Js("window.followed", "2"),
        ),
        (
            "visual mode: from a find match, w extends over the word, y yanks it as OSC 52",
            Box::new(|_, v| {
                // A find match on "bravo", as window.find leaves it.
                v.evaluate(
                    "const r=new Range();r.setStart(v.firstChild,6);r.setEnd(v.firstChild,11);\
                    getSelection().removeAllRanges();getSelection().addRange(r)",
                    |_| {},
                );
                v.evaluate(VISUAL_START_JS, |_| {});
                v.evaluate(&visual_script(Action::WordForward).unwrap(), |_| {});
                let page = v.clone();
                v.evaluate(YANK_JS, move |r| {
                    let text = r
                        .ok()
                        .and_then(|j| serde_json::from_str::<String>(&j).ok())
                        .unwrap_or_default();
                    let osc = yank(&text).0.unwrap_or_default();
                    let payload = std::str::from_utf8(&osc)
                        .ok()
                        .and_then(|s| s.strip_prefix("\x1b]52;c;")?.strip_suffix('\x07'))
                        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
                        .unwrap_or_default();
                    let js = format!(
                        "window.yanked={}",
                        serde_json::to_string(&String::from_utf8_lossy(&payload)).unwrap()
                    );
                    page.evaluate(&js, |_| {});
                });
            }),
            Check::Js("window.yanked", "\"bravo\""),
        ),
        (
            "frame cost at 1280x800 CSS, scale 1",
            Box::new(|c, v| {
                c.measure.set(true);
                v.evaluate("document.body.innerHTML='<h1>'+'Lorem ipsum dolor sit amet '.repeat(300)+'</h1>'", |_| {});
                v.set_size(1280, 800, 1.0);
            }),
            Check::None,
        ),
        (
            "frame cost at 1280x800 CSS, scale 2",
            Box::new(|c, v| {
                c.measure.set(true);
                v.set_size(1280, 800, 2.0);
            }),
            Check::None,
        ),
    ]
}

fn run_step(ctx: Rc<Ctx>, mut list: Vec<(&'static str, Act, Check)>) {
    if list.is_empty() {
        let fails = ctx.fails.get();
        println!(
            "{}",
            if fails == 0 {
                "ALL PASSED".to_string()
            } else {
                format!("{fails} FAILED")
            }
        );
        zebkit::quit();
        return;
    }
    let (name, act, check) = list.remove(0);
    let view = ctx.view.borrow().clone().unwrap();
    act(&ctx, &view);
    zebkit::after(Duration::from_millis(700), move || {
        let c2 = ctx.clone();
        let report = move |ok: bool, got: String| {
            println!(
                "  {} {name}{}",
                if ok { "PASS" } else { "FAIL" },
                if ok {
                    String::new()
                } else {
                    format!(" (got {got})")
                }
            );
            if !ok {
                c2.fails.set(c2.fails.get() + 1);
            }
            let _ = std::io::stdout().flush();
        };
        match check {
            Check::Js(js, want) => {
                let c = ctx.clone();
                view.evaluate(js, move |r| {
                    let got = r.unwrap_or_else(|e| format!("error {e}"));
                    let ok = got == want;
                    report(ok, got);
                    run_step(c, list);
                });
            }
            Check::Flag(f) => {
                let ok = f(&ctx);
                report(ok, "false".into());
                run_step(ctx.clone(), list);
            }
            Check::None => run_step(ctx.clone(), list),
        }
    });
}

fn measure(frame: zebkit::offscreen::Frame) {
    let t = Instant::now();
    let rgb = render::rgb(&frame);
    let t_rgb = t.elapsed();
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    z.write_all(&rgb).unwrap();
    let zl = z.finish().unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&zl);
    let t_direct = t.elapsed();
    let t2 = Instant::now();
    let name = render::shm_write(&rgb);
    let t_shm = t_rgb + t2.elapsed();
    if let Some(n) = name {
        let c = std::ffi::CString::new(n).unwrap();
        unsafe { libc::shm_unlink(c.as_ptr()) };
    }
    println!(
        "      frame {}x{}: direct (RGB+zlib+base64) {:.1} ms, {} KB on the pipe | shared memory {:.1} ms, {} B on the pipe",
        frame.width(),
        frame.height(),
        t_direct.as_secs_f64() * 1000.0,
        b64.len() / 1024,
        t_shm.as_secs_f64() * 1000.0,
        60
    );
}

pub fn run() -> i32 {
    let ctx = Rc::new(Ctx::default());
    let c = ctx.clone();
    zebkit::run_with_args(
        ["zweb", "--headless", "--zebkit-cookie-storage=file"],
        move || {
            let timeout_ctx = c.clone();
            zebkit::after(Duration::from_secs(60), move || {
                timeout_ctx.fails.set(timeout_ctx.fails.get() + 1);
                println!("TIMEOUT");
                zebkit::quit();
            });
            let view = View::new_offscreen(None, 800, 600, 1.0).expect("off-screen view");
            view.set_focus(true);
            view.set_dialog_policy(DialogPolicy::Hold);
            let k = c.clone();
            view.on_caret(move |r| k.caret.set(r.is_some()));
            let k = c.clone();
            view.on_popup(move |r| k.popup.set(r.is_some()));
            let k = c.clone();
            view.on_window_open(move |_v, _| k.opened.set(true));
            let k = c.clone();
            view.on_dialog(move |d| k.dialog.set(Some(d.id)));
            let k = c.clone();
            view.on_frame(move |f| {
                if f.layer() == zebkit::offscreen::Layer::View
                    && f.width() >= 1280
                    && k.measure.replace(false)
                {
                    measure(f);
                }
            });
            let started = Rc::new(Cell::new(false));
            let k = c.clone();
            view.on_load(move |e, _| {
                if e == LoadEvent::Finished && !started.replace(true) {
                    println!("zweb self-test");
                    run_step(k.clone(), steps());
                }
            });
            let url = format!(
                "data:text/html;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(PAGE)
            );
            view.load_url(&url);
            *c.view.borrow_mut() = Some(view);
        },
    );
    ctx.fails.get()
}
