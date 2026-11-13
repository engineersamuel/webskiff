//! `g e`: edits the page's text field in $VISUAL or $EDITOR (else vi).
//!
//! The editor runs on a worker thread, so the engine keeps running. Terminal
//! input stops while it runs, and the worker holds the stdout lock, so the
//! renderer writes nothing until the terminal is back.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Set while the editor has the terminal: the input thread does not read.
pub static INPUT_PAUSED: AtomicBool = AtomicBool::new(false);
/// Set by the input thread once it saw `INPUT_PAUSED` and stopped reading.
pub static INPUT_IDLE: AtomicBool = AtomicBool::new(false);

/// The editor's result, as the page should take it.
pub enum Edited {
    Failed,
    Unchanged,
    Text(String),
}

/// A new, private file with the field's text.
pub fn temp_file(text: &str) -> std::io::Result<PathBuf> {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("zweb-edit-{}-{n}.txt", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    file.write_all(text.as_bytes())?;
    Ok(path)
}

/// Runs the editor on `path` (blocks: call on a worker thread).
pub fn run(path: &Path, original: &str, single_line: bool, enhanced: bool) -> Edited {
    INPUT_PAUSED.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_millis(500);
    while !INPUT_IDLE.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let lock = std::io::stdout().lock();
    crate::restore_terminal(enhanced);
    let status = std::process::Command::new("sh")
        .args(["-c", "${VISUAL:-${EDITOR:-vi}} \"$1\"", "sh"])
        .arg(path)
        .status();
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crate::enter_screen(enhanced);
    drop(lock);
    match (status, std::fs::read_to_string(path)) {
        (Ok(s), Ok(raw)) if s.success() => result(&raw, original, single_line),
        _ => Edited::Failed,
    }
}

/// The edited text as the field takes it: a trailing newline the editor
/// added dropped, and a one-line field's lines joined with spaces.
fn result(raw: &str, original: &str, single_line: bool) -> Edited {
    if raw == original {
        return Edited::Unchanged;
    }
    let text = if original.ends_with('\n') { raw } else { raw.strip_suffix('\n').unwrap_or(raw) };
    let text = if single_line { text.replace('\n', " ") } else { text.to_string() };
    if text == original { Edited::Unchanged } else { Edited::Text(text) }
}

/// Picks the field (the focused one, else the one left with Esc), keeps it
/// for the write-back, and returns {text, input}, "password", "readonly" or null.
pub const READ_JS: &str = concat!(
    "(()=>{const ok=",
    editable!(),
    ";let e=document.activeElement;if(!(ok(e)&&e.isConnected))e=window.__zwebLastEdit;\
     if(!(ok(e)&&e.isConnected))return null;\
     if(e.tagName==='INPUT'&&e.type.toLowerCase()==='password')return 'password';\
     if(e.readOnly||e.disabled)return 'readonly';\
     window.__zwebEditTarget=e;return {text:e.isContentEditable?e.innerText:e.value,input:e.tagName==='INPUT'}})()"
);

/// Replaces the kept field's text with `t` as typing would; false if it is
/// gone or no longer writable.
pub fn write_js(text: &str) -> String {
    let t = serde_json::to_string(text).unwrap_or_default();
    format!(
        "(t=>{{const e=window.__zwebEditTarget;if(!e||!e.isConnected||e.readOnly||e.disabled)return false;e.focus();\
         if(e.isContentEditable)document.execCommand('selectAll');else e.select();\
         if(!document.execCommand('insertText',false,t)){{\
         if(e.isContentEditable)e.innerText=t;else Object.getOwnPropertyDescriptor(\
         (e.tagName==='TEXTAREA'?HTMLTextAreaElement:HTMLInputElement).prototype,'value').set.call(e,t);\
         e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}))}}\
         return true}})({t})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(e: Edited) -> Option<String> {
        match e {
            Edited::Text(t) => Some(t),
            _ => None,
        }
    }

    #[test]
    fn drops_one_newline_and_joins_lines_of_an_input() {
        assert_eq!(text(result("a\nb\n\n", "", false)).as_deref(), Some("a\nb\n"));
        assert_eq!(text(result("a\nb\n", "", true)).as_deref(), Some("a b"));
        assert!(matches!(result("same\n", "same", true), Edited::Unchanged));
    }

    #[test]
    fn keeps_a_newline_the_field_had() {
        assert!(matches!(result("original\n", "original\n", false), Edited::Unchanged));
        assert_eq!(text(result("changed\n", "original\n", false)).as_deref(), Some("changed\n"));
        assert_eq!(text(result("changed\n", "original", false)).as_deref(), Some("changed"));
        assert_eq!(text(result("a\nb\n", "a", true)).as_deref(), Some("a b"));
    }

    #[test]
    fn temp_file_is_private_and_new() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("x").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
        let _ = std::fs::remove_file(&path);
        assert_eq!(mode & 0o777, 0o600);
    }
}
