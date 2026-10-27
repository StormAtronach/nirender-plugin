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
//! (the mod is missing), `after_present` shows the box itself, with "No save was possible".
//!
//! The box is modal on the game's thread: a close of the game waits until the box is closed
//! (the run 20261027-122536-triage-crash3 sat in it for minutes). `MGE_XE_BOX_SECONDS=<n>`
//! closes it after n seconds, for a test.

use std::sync::Mutex;

use crate::capture::{ticks, ticks_per_second};
use crate::stream_format::{EXIT_NOT_ANSWERING, EXIT_STALLED, STALL_SECONDS};

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(window: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

/// `MessageBoxTimeoutW` of user32: a box that closes itself after the milliseconds, with the
/// answer [`MB_TIMEDOUT`]. Exported since Windows XP, not in the documented API: taken by
/// `GetProcAddress` (the failure-path design, section 5.1).
type MessageBoxTimeout = unsafe extern "system" fn(isize, *const u16, *const u16, u32, u16, u32) -> i32;
const MB_TIMEDOUT: i32 = 32000;

/// The caption of the box, and the name of its window for the switch that closes it.
const CAPTION: &str = "MGE XE";
/// The seconds that the Lua side has after `found` to save and call `handled`, before the
/// box goes up without a save.
const LUA_WAIT_SECONDS: i64 = 3;
/// The seconds the ring wait of the plugin gives the helper before it ends it (`capture.rs`
/// `RING_WAIT_SECONDS`), for the word of the box.
const RING_WAIT_SECONDS: u64 = 10;

/// What stopped the helper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The process ended with an exit code that is not one of its own: the fast-fail code of
    /// a panic (0xc0000409), or the exception code of a fault.
    Crashed(u32),
    /// The watchdog of the helper ended it (`EXIT_STALLED`).
    Stalled,
    /// The plugin ended it after `RING_WAIT_SECONDS` without a read of the ring
    /// (`EXIT_NOT_ANSWERING`).
    NotAnswering,
    /// The helper lost its graphics device and said so (`MESSAGE_LOST_DEVICE`); it lives on.
    LostDevice,
}

impl Cause {
    /// The cause behind an exit code of the helper that is not a refusal of its own.
    pub fn of_exit_code(code: u32) -> Cause {
        match code {
            EXIT_STALLED => Cause::Stalled,
            EXIT_NOT_ANSWERING => Cause::NotAnswering,
            other => Cause::Crashed(other),
        }
    }

    /// The exit code for the Lua side: 0 for a lost device, whose helper lives on.
    pub fn code(self) -> u32 {
        match self {
            Cause::Crashed(code) => code,
            Cause::Stalled => EXIT_STALLED,
            Cause::NotAnswering => EXIT_NOT_ANSWERING,
            Cause::LostDevice => 0,
        }
    }

    /// The words of the WHY line of the box, after "It".
    pub fn word(self) -> String {
        match self {
            Cause::Crashed(code) => format!("crashed (exit code {code:#x})"),
            Cause::Stalled => format!("stalled for {STALL_SECONDS} s (exit code {EXIT_STALLED})"),
            Cause::NotAnswering => format!("did not answer for {RING_WAIT_SECONDS} s (exit code {EXIT_NOT_ANSWERING})"),
            Cause::LostDevice => "lost its graphics device".to_string(),
        }
    }
}

/// The failure that was found, and where its path stands.
struct Failure {
    cause: Cause,
    /// The ticks of the performance counter when the cause was found.
    found_at: i64,
    /// The Lua side asked for the state: it is there, and it calls `handled`.
    lua_asked: bool,
    box_shown: bool,
}

static FAILURE: Mutex<Option<Failure>> = Mutex::new(None);

/// Keeps the first cause that is found. The log names it.
pub fn found(cause: Cause) {
    let mut failure = FAILURE.lock().unwrap();
    if failure.is_some() {
        return;
    }
    crate::log!("failure: the host program {}; the game is saved, then the message box", cause.word());
    *failure = Some(Failure { cause, found_at: ticks(), lua_asked: false, box_shown: false });
}

/// For `nirender.hostState()`: the exit code and the word of the cause that was found and
/// not handled yet; `None` while the helper is well, and after the box.
pub fn host_state() -> Option<(u32, String)> {
    let mut failure = FAILURE.lock().unwrap();
    let failure = failure.as_mut().filter(|failure| !failure.box_shown)?;
    failure.lua_asked = true;
    Some((failure.cause.code(), failure.cause.word()))
}

/// For `nirender.failureHandled(saveName, why)`: the Lua side saved the game as `save`, or
/// could not (`why`, for example "in this state"). The box goes up, from Lua's frame on the
/// main thread, outside the device lock.
pub fn handled(save: Option<String>, why: Option<String>) {
    let mut failure = FAILURE.lock().unwrap();
    let Some(failure) = failure.as_mut().filter(|failure| !failure.box_shown) else {
        return;
    };
    show(failure, &now_line(save.as_deref(), why.as_deref()));
}

/// After a Present, outside the device lock: the box without a save, when no Lua call came
/// in time (the mod of the plugin is missing).
pub fn after_present() {
    let mut failure = FAILURE.lock().unwrap();
    let Some(failure) = failure.as_mut().filter(|failure| !failure.box_shown && !failure.lua_asked) else {
        return;
    };
    if ticks() - failure.found_at < LUA_WAIT_SECONDS * ticks_per_second() {
        return;
    }
    crate::log!("failure: no call of the mod of the plugin in {LUA_WAIT_SECONDS} s: the box without a save");
    show(failure, &now_line(None, Some("without the mod of the plugin")));
}

/// Shows the box, one time (the failure-path design, section 5.1): each line of its text goes
/// into the log first, prefixed `box: `; with `MGE_XE_BOX_SECONDS=<n>` the box closes itself
/// after n seconds, and the log says whether a click or the time closed it.
fn show(failure: &mut Failure, now: &str) {
    failure.box_shown = true;
    let text = box_text(failure.cause, now);
    for line in text.lines().filter(|line| !line.is_empty()) {
        crate::log!("box: {line}");
    }
    let seconds = std::env::var("MGE_XE_BOX_SECONDS").ok().and_then(|value| value.parse::<u32>().ok()).unwrap_or(0);
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, caption) = (wide(&text), wide(CAPTION));
    const MB_ICONERROR: u32 = 0x10;
    const MB_SETFOREGROUND: u32 = 0x1_0000;
    const MB_TOPMOST: u32 = 0x4_0000;
    let kind = MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST;
    // SAFETY: both strings are zero-ended UTF-16 that live until the call returns; the
    // function of user32 has the signature of `MessageBoxTimeout` when it is there.
    let answer = unsafe {
        match (seconds, message_box_timeout()) {
            (0, _) => MessageBoxW(0, text.as_ptr(), caption.as_ptr(), kind),
            (_, Some(with_timeout)) => with_timeout(0, text.as_ptr(), caption.as_ptr(), kind, 0, seconds * 1000),
            (_, None) => {
                crate::log!("box: not shown: this Windows has no MessageBoxTimeoutW");
                return;
            }
        }
    };
    if answer == MB_TIMEDOUT {
        crate::log!("box: closed after {seconds} s, no click");
    } else {
        crate::log!("box: closed by a click");
    }
}

/// `MessageBoxTimeoutW`, when user32 has it.
unsafe fn message_box_timeout() -> Option<MessageBoxTimeout> {
    let user32: Vec<u16> = "user32.dll".encode_utf16().chain([0]).collect();
    let module = crate::win::GetModuleHandleW(user32.as_ptr());
    let function = crate::win::GetProcAddress(module, c"MessageBoxTimeoutW".as_ptr().cast());
    (!function.is_null()).then(|| core::mem::transmute::<*mut core::ffi::c_void, MessageBoxTimeout>(function))
}

/// The NOW line of the box: what the player does.
fn now_line(save: Option<&str>, why: Option<&str>) -> String {
    match (save, why) {
        (Some(save), _) => format!("The game was saved as \"{save}\": quit the game, start it again and load it."),
        (None, Some(why)) => format!("No save was possible {why}: quit the game and start it again."),
        (None, None) => "No save was possible: quit the game and start it again.".to_string(),
    }
}

/// The text of the box: WHAT, WHY, NOW, WHERE (the failure-path design, section 5).
fn box_text(cause: Cause, now: &str) -> String {
    format!(
        "The wgpu renderer of MGE XE (numidium.exe) has stopped.\n\
         It {}.\n\n\
         {now}\n\n\
         The folder of the game has the details: the logs nirender-host.log, nirender.log and \
         mgeHost64.log, and the minidump numidium.dmp after a crash of the renderer.",
        cause.word()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_name_the_cause_and_the_code() {
        assert_eq!(Cause::of_exit_code(0xc000_0409).word(), "crashed (exit code 0xc0000409)");
        assert_eq!(Cause::of_exit_code(30), Cause::Stalled);
        assert_eq!(Cause::Stalled.word(), "stalled for 5 s (exit code 30)");
        assert_eq!(Cause::of_exit_code(31).word(), "did not answer for 10 s (exit code 31)");
        assert_eq!(Cause::LostDevice.word(), "lost its graphics device");
        assert_eq!(Cause::LostDevice.code(), 0);
        assert_eq!(Cause::Crashed(0xc000_0005).code(), 0xc000_0005);
    }

    #[test]
    fn the_box_has_the_four_parts() {
        let text = box_text(Cause::Stalled, &now_line(Some("MGE XE autosave 2026-10-27 12:00"), None));
        assert!(text.starts_with("The wgpu renderer of MGE XE (numidium.exe) has stopped.\nIt stalled for 5 s (exit code 30).\n\n"));
        assert!(text.contains("The game was saved as \"MGE XE autosave 2026-10-27 12:00\": quit the game, start it again and load it."));
        assert!(text.ends_with("the minidump numidium.dmp after a crash of the renderer."));
        assert_eq!(now_line(None, Some("in this state")), "No save was possible in this state: quit the game and start it again.");
        assert_eq!(now_line(None, None), "No save was possible: quit the game and start it again.");
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
        assert!(FAILURE.lock().unwrap().as_ref().is_some_and(|failure| failure.lua_asked && !failure.box_shown));
    }
}
