//! The live check of the re-entrancy map (`moreFPS/docs/Numidium Project/notes/reentrancy`):
//! with `NIRENDER_ENTRY_TRACE=1` each way into the plugin that makes a proof (the proof of
//! the main thread, the device lock) and each call of the plugin into the engine notes its
//! place on a stack of its thread. An entry made while another entry is live is a nesting:
//! the log gets the outer entry, the newest call-out since it, and the inner entry, once for
//! each triple. A nesting with no call-out between the two entries is the plugin's own code.
//! Without the switch each place costs the read of one flag.

use core::cell::RefCell;
use core::panic::Location;
use std::sync::atomic::{AtomicBool, Ordering};

type Place = &'static Location<'static>;

static ON: AtomicBool = AtomicBool::new(false);
static READ: std::sync::Once = std::sync::Once::new();

thread_local! {
    /// The live entries and call-outs of this thread, in their order.
    static STACK: RefCell<Vec<(bool, Place)>> = const { RefCell::new(Vec::new()) };
}

/// The triples (outer entry, call-out or none, inner entry) that were logged.
static LOGGED: std::sync::Mutex<Vec<(usize, usize, usize)>> = std::sync::Mutex::new(Vec::new());

#[inline(always)]
fn on() -> bool {
    READ.call_once(|| ON.store(std::env::var("NIRENDER_ENTRY_TRACE").is_ok_and(|value| value == "1"), Ordering::Relaxed));
    ON.load(Ordering::Relaxed)
}

/// A live entry or call-out; it leaves the stack when it ends.
pub struct Mark(());

impl Drop for Mark {
    fn drop(&mut self) {
        let _ = STACK.try_with(|stack| stack.borrow_mut().pop());
    }
}

/// An entry into the plugin at the place of the caller.
#[track_caller]
#[inline(always)]
pub fn entry() -> Option<Mark> {
    // The place of the caller: a closure would give its own place.
    let here = Location::caller();
    on().then(|| push(true, here))
}

/// A call of the plugin into the engine at the place of the caller.
#[track_caller]
#[inline(always)]
pub fn call_out() -> Option<Mark> {
    let here = Location::caller();
    on().then(|| push(false, here))
}

#[cold]
#[inline(never)]
fn push(is_entry: bool, here: Place) -> Mark {
    let _ = STACK.try_with(|stack| {
        let mut stack = stack.borrow_mut();
        if is_entry {
            if let Some(outer) = stack.iter().rposition(|&(entry, _)| entry) {
                let call = stack[outer + 1..].iter().rev().find(|&&(entry, _)| !entry).map(|&(_, place)| place);
                note(stack[outer].1, call, here);
            }
        }
        stack.push((is_entry, here));
    });
    Mark(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logged() -> usize {
        LOGGED.lock().unwrap().len()
    }

    #[test]
    fn a_nested_entry_is_noted_with_the_call_out_between() {
        READ.call_once(|| {});
        ON.store(true, Ordering::Relaxed);
        let before = logged();
        let (outer, outer_line) = (entry(), line!());
        {
            let _call = call_out();
            let _inner = entry();
        }
        let after_first = logged();
        // The same triple again is not noted again.
        {
            let _call = call_out();
            let _inner = entry();
        }
        drop(outer);
        assert!(after_first > before);
        // The places are those of this test, not of the module's closures.
        let line = LOGGED.lock().unwrap().iter().any(|&(outer, _, _)| unsafe { &*(outer as *const Location) }.file().ends_with("entry_trace.rs") && unsafe { &*(outer as *const Location) }.line() == outer_line);
        assert!(line, "the outer entry is the test's own line");
        assert_eq!(STACK.with(|stack| stack.borrow().len()), 0);
        // Entries one after the other are no nesting.
        let count = logged();
        drop(entry());
        drop(entry());
        assert_eq!(logged(), count);
    }
}

fn note(outer: Place, call: Option<Place>, inner: Place) {
    let key = (outer as *const _ as usize, call.map_or(0, |place| place as *const _ as usize), inner as *const _ as usize);
    let Ok(mut logged) = LOGGED.lock() else { return };
    if logged.contains(&key) {
        return;
    }
    logged.push(key);
    drop(logged);
    match call {
        Some(call) => crate::log!("entries: {outer} -> call-out {call} -> {inner}"),
        None => crate::log!("entries: {outer} -> own code -> {inner}"),
    }
}
