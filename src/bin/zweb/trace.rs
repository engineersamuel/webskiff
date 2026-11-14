//! `ZWEB_TRACE=1`: times of each stage of an interaction, to the log.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

pub fn init() {
    START.get_or_init(Instant::now);
    ON.store(std::env::var_os("ZWEB_TRACE").is_some(), Ordering::Relaxed);
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn mark(what: std::fmt::Arguments) {
    if on() {
        let t = START
            .get()
            .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
        eprintln!("T {t:10.2} {what}");
    }
}

#[macro_export]
macro_rules! trace {
    ($($a:tt)*) => { $crate::trace::mark(format_args!($($a)*)) };
}

#[cfg(target_os = "macos")]
extern "C" {
    fn mach_absolute_time() -> u64;
}

/// Now in Chromium's TimeTicks microseconds.
#[cfg(target_os = "macos")]
pub fn ticks_us() -> i64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    let mut tb = Timebase { numer: 0, denom: 0 };
    unsafe {
        mach_timebase_info(&mut tb);
        (mach_absolute_time() as u128 * tb.numer as u128 / tb.denom as u128 / 1000) as i64
    }
}

#[cfg(target_os = "linux")]
pub fn ticks_us() -> i64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) };
    assert_eq!(result, 0, "cannot read the monotonic clock");
    time.tv_sec * 1_000_000 + time.tv_nsec / 1_000
}
