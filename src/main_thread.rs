//! The proof that code runs on the main thread of the game, as a value: [`MainThread`]. A
//! table that only the main thread may touch lives in a [`MainCell`], which opens only for a
//! holder of the proof. A call without one does not compile.
//!
//! The proof is made at the places that are the main thread by construction: the dispatch
//! of a port on its main-thread branch (`shadow::dispatch`), a Lua function behind the
//! thread gate (`lua::main_thread_gate`), and the frame hook. A path that checks the thread
//! itself takes `here()`. The engine's cell worker never gets one: a port's Rust does not
//! run there, and the worker's events reach the tables through a queue (`worker_events`).
//!
//! What the proof does not do: two `get` calls with one proof would give two `&mut` to one
//! table. The rule is one open at a time (no call back into the module while a table is
//! open). `get` gives a guard ([`crate::checked::Open`]); a second open while the first is
//! live panics.

use crate::checked::{Checked, Open};
use core::marker::PhantomData;

/// The holder runs on the main thread. Not [`Send`] and not [`Sync`]: it cannot leave the thread
/// it was made on.
pub struct MainThread(PhantomData<*mut ()>, #[allow(dead_code)] Option<crate::entry_trace::Mark>);

impl MainThread {
    /// The proof, where the caller is the main thread by construction. The three places:
    /// `shadow::dispatch_inner` after its thread check, `lua::main_thread_gate` behind its
    /// check, and `worker_events::drain` after its check.
    #[track_caller]
    pub unsafe fn claim() -> MainThread {
        debug_assert!(crate::shadow::on_main_thread(), "a claim of the main thread off it");
        MainThread(PhantomData, crate::entry_trace::entry())
    }

    /// The proof when the calling thread is the main thread, else [`None`]: for a path that
    /// any thread can reach, such as a call of the device.
    #[track_caller]
    pub fn here() -> Option<MainThread> {
        if crate::shadow::on_main_thread() {
            Some(MainThread(PhantomData, crate::entry_trace::entry()))
        } else {
            None
        }
    }

    /// A test runs on its own thread, which is its main thread.
    #[cfg(test)]
    pub fn for_test() -> MainThread {
        MainThread(PhantomData, None)
    }
}

/// A table of the main thread. `get` needs the proof.
pub struct MainCell<T>(Checked<T>);

// The cell is shared between threads as a static, but only the main thread opens it.
unsafe impl<T> Sync for MainCell<T> {}

impl<T> MainCell<T> {
    pub const fn new(value: T) -> MainCell<T> {
        MainCell(Checked::new(value))
    }

    /// The table, for a holder of the proof, until the guard ends. One open at a time (see
    /// the module).
    #[track_caller]
    #[inline(always)]
    pub fn get<'a>(&'a self, _: &'a MainThread) -> Open<'a, T> {
        // The proof says the main thread: no other thread opens the table.
        unsafe { self.0.open() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static CELL: MainCell<u32> = MainCell::new(1);

    #[test]
    fn the_cell_opens_for_the_proof() {
        let t = MainThread::for_test();
        *CELL.get(&t) += 1;
        assert_eq!(*CELL.get(&t), 2);
    }
}
