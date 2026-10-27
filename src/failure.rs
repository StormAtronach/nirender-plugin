//! The failure of the helper after the game has its Direct3D 8 (the failure-path design,
//! sections 3 to 6): the helper died (a crash, its own watchdog, the plugin's end of it after
//! 10 s without a read), or it lost its graphics device. No Direct3D 9 device exists then,
//! so the game runs on without a picture: the game is saved first, then one message box says
//! what happened, and the player quits.
//!
//! The path: `found` keeps the cause (at the end of the frame that found it, under the
//! device lock). The Lua side of the plugin's mod (failure.lua) asks `host_state` at each
//! frame, saves the game (`tes3.saveGame`, tried again for 10 s when the engine refuses) and
//! calls `handled` with the name of the save: the box goes up then, on the main thread,
//! outside the lock. When no Lua call came [`LUA_WAIT_SECONDS`] after the cause was found
//! (the mod is missing), or the Lua side asked and did not answer in
//! [`LUA_ANSWER_SECONDS`] (an error in its handler at each frame), `after_present` shows the
//! box itself, with "No save was possible".
//!
//! The box runs on a thread of its own (`show`), and it is a stop box: when it closes (one OK,
//! one Alt+F4 on it, or the time of `MGE_XE_BOX_SECONDS=<n>` in a test) its thread posts
//! `WM_CLOSE` to the game's window, and the game closes as from its own close button (the
//! failure-path design, section 5 "The close rule"). The save is made before the box.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::capture::{ticks, ticks_per_second};
use crate::stream_format::{EXIT_NOT_ANSWERING, EXIT_STALLED, STALL_SECONDS};

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(window: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    fn PostMessageW(window: isize, message: u32, w: usize, l: isize) -> i32;
}

/// `MessageBoxTimeoutW` of user32: a box that closes itself after the milliseconds. Exported
/// since Windows XP, not in the documented API: taken by `GetProcAddress` (the failure-path
/// design, section 5.1). Its answer on the time limit is not trusted: the elapsed time says
/// what closed the box (`closed_line`).
type MessageBoxTimeout = unsafe extern "system" fn(isize, *const u16, *const u16, u32, u16, u32) -> i32;

/// The caption of the box, and the name of its window for the switch that closes it.
const CAPTION: &str = "MGE XE";
/// The seconds that the Lua side has after `found` to ask for the state, before the box goes
/// up without a save (the mod of the plugin is missing).
const LUA_WAIT_SECONDS: i64 = 3;
/// The seconds that the Lua side has after its first ask to call `handled`: the 10 s that
/// failure.lua tries the save again (`RETRY_SECONDS` there), and 5 s more. After them the box
/// goes up without a save: the handler of the mod fails at each frame, or it hangs.
const LUA_ANSWER_SECONDS: i64 = 15;

/// What stopped the helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The process ended with an exit code that is not one of its own: the fast-fail code of
    /// a panic (0xc0000409), or the exception code of a fault.
    Crashed(u32),
    /// The watchdog of the helper ended it (`EXIT_STALLED`) after a step of that many seconds:
    /// its limit of the moment, which it writes as its allowance word (5 s, 30 s while its
    /// extension loads).
    Stalled(u64),
    /// The plugin ended it (`EXIT_NOT_ANSWERING`) after that many seconds without a read of the
    /// ring while records waited (`capture.rs`, the limit from the helper's allowance).
    NotAnswering(u64),
    /// The helper lost its graphics device and said so (`MESSAGE_LOST_DEVICE`); it lives on.
    LostDevice,
}

impl Cause {
    /// The cause behind an exit code of the helper that is not a refusal of its own;
    /// `allowance` is the helper's allowance word (`stream_format::RING_ALLOWANCE`), 0 before
    /// its frames start, when its watchdog has its first limit.
    pub fn of_exit_code(code: u32, allowance: u32) -> Cause {
        match code {
            EXIT_STALLED => Cause::Stalled(if allowance == 0 { STALL_SECONDS } else { u64::from(allowance) }),
            EXIT_NOT_ANSWERING => Cause::NotAnswering(NOT_ANSWERING_AFTER.load(Ordering::Relaxed)),
            other => Cause::Crashed(other),
        }
    }

    /// The exit code for the Lua side: 0 for a lost device, whose helper lives on.
    pub fn code(self) -> u32 {
        match self {
            Cause::Crashed(code) => code,
            Cause::Stalled(_) => EXIT_STALLED,
            Cause::NotAnswering(_) => EXIT_NOT_ANSWERING,
            Cause::LostDevice => 0,
        }
    }

    /// The words of the WHY line of the box, after "It".
    pub fn word(self) -> String {
        match self {
            // The process ended without a reason of its own while the game ran: a kill
            // (`taskkill`, the task manager) or an exit that no code of the helper names.
            Cause::Crashed(0) => "ended without a reason (exit code 0)".to_string(),
            Cause::Crashed(code) => format!("crashed (exit code {code:#x})"),
            Cause::Stalled(seconds) => format!("stalled for {seconds} s (exit code {EXIT_STALLED})"),
            Cause::NotAnswering(seconds) => format!("did not answer for {seconds} s (exit code {EXIT_NOT_ANSWERING})"),
            Cause::LostDevice => "lost its graphics device".to_string(),
        }
    }
}

/// The failure that was found, and where its path stands.
struct Failure {
    cause: Cause,
    /// The ticks of the performance counter when the cause was found.
    found_at: i64,
    /// The ticks of the first ask of the Lua side for the state, or 0 before it: it is there,
    /// and it calls `handled`.
    asked_at: i64,
    box_shown: bool,
}

static FAILURE: Mutex<Option<Failure>> = Mutex::new(None);

/// The seconds without a read after which the plugin ended the helper, for the word of
/// [`Cause::NotAnswering`]: set by `capture.rs` before `end_host`.
static NOT_ANSWERING_AFTER: AtomicU64 = AtomicU64::new(0);

/// The plugin ends the helper after `seconds` without a read: the word of the box names them.
pub fn not_answering_after(seconds: u64) {
    NOT_ANSWERING_AFTER.store(seconds, Ordering::Relaxed);
}

/// Keeps the first cause that is found. The log names it.
pub fn found(cause: Cause) {
    let mut failure = FAILURE.lock().unwrap();
    if failure.is_some() {
        return;
    }
    crate::log!("failure: the host program {}; the game is saved, then the message box", cause.word());
    *failure = Some(Failure { cause, found_at: ticks(), asked_at: 0, box_shown: false });
}

/// For `nirender.hostState()`: the exit code and the word of the cause that was found and
/// not handled yet; `None` while the helper is well, and after the box.
pub fn host_state() -> Option<(u32, String)> {
    let mut failure = FAILURE.lock().unwrap();
    let failure = failure.as_mut().filter(|failure| !failure.box_shown)?;
    if failure.asked_at == 0 {
        failure.asked_at = ticks();
    }
    Some((failure.cause.code(), failure.cause.word()))
}

/// For `nirender.failureHandled(saveName, why)`: the Lua side saved the game as `save`, or
/// could not (`why`, for example "in this state"). The box goes up, from Lua's frame on the
/// main thread, outside the device lock.
pub fn handled(save: Option<String>, why: Option<String>) {
    if let Some(text) = take_box(&now_line(save.as_deref(), why.as_deref())) {
        show(&text);
    }
}

/// After a Present, outside the device lock: the box without a save, when the Lua side did
/// not ask in time (the mod of the plugin is missing), or asked and did not answer in time.
pub fn after_present() {
    let now = ticks();
    let second = ticks_per_second();
    let late = {
        let failure = FAILURE.lock().unwrap();
        failure.as_ref().filter(|failure| !failure.box_shown).and_then(|failure| lua_late(failure.found_at, failure.asked_at, now, second))
    };
    let Some(why) = late else { return };
    crate::log!("failure: {why}: the box without a save");
    if let Some(text) = take_box(&now_line(None, Some(why))) {
        show(&text);
    }
}

/// The reason for the box without a save, when the Lua side is late at `now`: no ask in
/// [`LUA_WAIT_SECONDS`] after the cause was found, or no answer in [`LUA_ANSWER_SECONDS`]
/// after its first ask; else `None`.
fn lua_late(found_at: i64, asked_at: i64, now: i64, second: i64) -> Option<&'static str> {
    if asked_at == 0 {
        (now - found_at >= LUA_WAIT_SECONDS * second).then_some("without the mod of the plugin")
    } else {
        (now - asked_at >= LUA_ANSWER_SECONDS * second).then_some("because the mod of the plugin did not answer")
    }
}

/// Marks the box as shown, one time, and gives its text; `None` when it was shown. The
/// state's lock is free again before the modal box goes up.
fn take_box(now: &str) -> Option<String> {
    let mut failure = FAILURE.lock().unwrap();
    let failure = failure.as_mut().filter(|failure| !failure.box_shown)?;
    failure.box_shown = true;
    Some(box_text(failure.cause, now))
}

/// Shows the box (the failure-path design, section 5.1): each line of its text goes into the
/// log first, prefixed `box: `, on the caller's thread; with `MGE_XE_BOX_SECONDS=<n>` the box
/// closes itself after n seconds, and the log says whether a click or the time closed it.
///
/// The box runs on a thread of its own, with no owner window, and nobody waits for it (the
/// failure-path design, section 5 "The thread rule"): a modal box on the game's main thread
/// pumps messages into the game's window procedure (a crash in `restartRenderer` at start-up,
/// Hob's run 1c) and holds a close of the game until it is closed (the run
/// 20261027-122536-triage-crash3). On its own thread the box leaves the game's thread to pump
/// its messages: Alt+F4 closes the game at once, with the box up or not.
fn show(text: &str) {
    for line in text.lines().filter(|line| !line.is_empty()) {
        crate::log!("box: {line}");
    }
    // 0 to an hour, as the other boxes take the switch (`own_device::message_box`).
    // At most an hour: the timeout of the box is in ms, and a larger value overflows it.
    let seconds = crate::switches::parsed("MGE_XE_BOX_SECONDS", "a number of seconds", |value| value.trim().parse::<u32>().ok()).map_or(0, |n| n.min(3600));
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(text), wide(CAPTION));
    std::thread::spawn(move || {
        const MB_ICONERROR: u32 = 0x10;
        const MB_SETFOREGROUND: u32 = 0x1_0000;
        const MB_TOPMOST: u32 = 0x4_0000;
        let kind = MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST;
        let shown = Instant::now();
        // SAFETY: both strings are zero-ended UTF-16 that the thread owns until the call
        // returns; the function of user32 has the signature of `MessageBoxTimeout` when it is
        // there.
        unsafe {
            match (seconds, message_box_timeout()) {
                (0, _) => MessageBoxW(0, text.as_ptr(), caption.as_ptr(), kind),
                (_, Some(with_timeout)) => with_timeout(0, text.as_ptr(), caption.as_ptr(), kind, 0, seconds * 1000),
                (_, None) => {
                    crate::log!("box: not shown: this Windows has no MessageBoxTimeoutW");
                    0
                }
            }
        };
        crate::log!("{}", closed_line(shown.elapsed(), seconds));
        close_the_game();
    });
}

/// The stop box closed: the game closes, as from its own close button. The game is blind
/// after the failure, and the save is made before the box.
fn close_the_game() {
    const WM_CLOSE: u32 = 0x10;
    let window = crate::own_device::game_window();
    if window == 0 {
        crate::log!("box: the game has no window to close");
        return;
    }
    // SAFETY: a post of a message to a window of this process; a window that is gone takes none.
    let posted = unsafe { PostMessageW(window as isize, WM_CLOSE, 0, 0) } != 0;
    crate::log!("box: the game is closed ({})", if posted { "WM_CLOSE to its window" } else { "the post failed" });
}

/// The log line after the box: a box with a time limit that stayed up for the limit was
/// closed by the time, whatever its answer (`MessageBoxTimeoutW` gave the OK answer on the
/// time limit in the run 20261027-130530-c1-stall).
fn closed_line(waited: Duration, limit_seconds: u32) -> String {
    let limit = Duration::from_secs(u64::from(limit_seconds));
    if limit_seconds > 0 && waited + Duration::from_millis(500) >= limit {
        format!("box: closed after {limit_seconds} s, no click (the time limit)")
    } else {
        format!("box: closed by a click after {:.1} s", waited.as_secs_f64())
    }
}

/// `MessageBoxTimeoutW`, when user32 has it.
unsafe fn message_box_timeout() -> Option<MessageBoxTimeout> {
    let user32: Vec<u16> = "user32.dll".encode_utf16().chain([0]).collect();
    let module = crate::win::GetModuleHandleW(user32.as_ptr());
    let function = crate::win::GetProcAddress(module, c"MessageBoxTimeoutW".as_ptr().cast());
    (!function.is_null()).then(|| core::mem::transmute::<*mut core::ffi::c_void, MessageBoxTimeout>(function))
}

/// The NOW line of the box: what the player does. The game has no picture any more: the box
/// closes the game when it closes.
fn now_line(save: Option<&str>, why: Option<&str>) -> String {
    match (save, why) {
        (Some(save), _) => format!("The game was saved as \"{save}\". Close this box: the game closes. Start it again and load the save."),
        (None, Some(why)) => format!("No save was possible {why}. Close this box: the game closes. Start it again."),
        (None, None) => "No save was possible. Close this box: the game closes. Start it again.".to_string(),
    }
}

/// The text of the box: WHAT, WHY, NOW, WHERE (the failure-path design, section 5).
fn box_text(cause: Cause, now: &str) -> String {
    format!(
        "The wgpu renderer of MGE XE (numidium.exe) has stopped.\n\
         It {}.\n\n\
         {now}\n\n\
         The folder of the game has the details: the logs nirender-host.log, mgeXE.log (its lines \
         that start with \"nirender: \") and mgeHost64.log, and the minidump numidium.dmp after a crash \
         of the renderer.",
        cause.word()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_name_the_cause_and_the_code() {
        assert_eq!(Cause::of_exit_code(0xc000_0409, 5).word(), "crashed (exit code 0xc0000409)");
        assert_eq!(Cause::of_exit_code(30, 5), Cause::Stalled(5));
        assert_eq!(Cause::of_exit_code(30, 0), Cause::Stalled(5));
        assert_eq!(Cause::Stalled(5).word(), "stalled for 5 s (exit code 30)");
        // A stall while the extension loads: the box names the watchdog's 30 s.
        assert_eq!(Cause::of_exit_code(30, 30).word(), "stalled for 30 s (exit code 30)");
        not_answering_after(10);
        assert_eq!(Cause::of_exit_code(31, 5).word(), "did not answer for 10 s (exit code 31)");
        assert_eq!(Cause::NotAnswering(35).word(), "did not answer for 35 s (exit code 31)");
        assert_eq!(Cause::of_exit_code(0, 5).word(), "ended without a reason (exit code 0)");
        assert_eq!(Cause::LostDevice.word(), "lost its graphics device");
        assert_eq!(Cause::LostDevice.code(), 0);
        assert_eq!(Cause::Crashed(0xc000_0005).code(), 0xc000_0005);
    }

    #[test]
    fn the_box_has_the_four_parts() {
        let text = box_text(Cause::Stalled(5), &now_line(Some("MGE XE autosave 2026-10-27 12:00"), None));
        assert!(text.starts_with("The wgpu renderer of MGE XE (numidium.exe) has stopped.\nIt stalled for 5 s (exit code 30).\n\n"));
        assert!(text.contains("The game was saved as \"MGE XE autosave 2026-10-27 12:00\". Close this box: the game closes. Start it again and load the save."));
        assert!(text.ends_with("the minidump numidium.dmp after a crash of the renderer."));
        assert_eq!(now_line(None, Some("in this state")), "No save was possible in this state. Close this box: the game closes. Start it again.");
        assert_eq!(now_line(None, None), "No save was possible. Close this box: the game closes. Start it again.");
    }

    #[test]
    fn the_time_limit_closes_the_box_whatever_its_answer() {
        assert_eq!(closed_line(Duration::from_millis(15_200), 15), "box: closed after 15 s, no click (the time limit)");
        assert_eq!(closed_line(Duration::from_millis(14_600), 15), "box: closed after 15 s, no click (the time limit)");
        assert_eq!(closed_line(Duration::from_millis(3_200), 15), "box: closed by a click after 3.2 s");
        assert_eq!(closed_line(Duration::from_millis(20_000), 0), "box: closed by a click after 20.0 s");
    }

    #[test]
    fn the_state_goes_from_found_to_the_lua_side_once() {
        // The tests of this module share the one state: this is the only one that sets it.
        assert_eq!(host_state(), None);
        found(Cause::Crashed(0xc000_0409));
        found(Cause::LostDevice);
        let (code, word) = host_state().expect("the cause");
        assert_eq!((code, word.as_str()), (0xc000_0409, "crashed (exit code 0xc0000409)"));
        // The Lua side is there: the plugin waits for its call and shows no box of its own.
        after_present();
        assert!(FAILURE.lock().unwrap().as_ref().is_some_and(|failure| failure.asked_at != 0 && !failure.box_shown));
    }

    #[test]
    fn a_lua_side_that_does_not_ask_or_does_not_answer_is_late() {
        let second = 1000;
        // No ask: late after 3 s from the cause.
        assert_eq!(lua_late(0, 0, 2_900, second), None);
        assert_eq!(lua_late(0, 0, 3_000, second), Some("without the mod of the plugin"));
        // An ask at 1 s: late 15 s after it, whatever the time of the cause.
        assert_eq!(lua_late(0, 1_000, 15_900, second), None);
        assert_eq!(lua_late(0, 1_000, 16_000, second), Some("because the mod of the plugin did not answer"));
    }
}
