//! The proof that code runs on the main thread of the game, as a value: `MainThread`. A
//! table that only the main thread may touch lives in a `MainCell`, which opens only for a
//! holder of the proof. A call without one does not compile.
//!
//! The proof is made at the places that are the main thread by construction: the dispatch
//! of a port on its main-thread branch (`shadow::dispatch`), a Lua function behind the
//! thread gate (`lua::main_thread_gate`), and the frame hook. A path that checks the thread
//! itself takes `here()`. The engine's cell worker never gets one: a port's Rust does not
//! run there, and the worker's events reach the tables through a queue (`worker_events`).
//!
//! What the proof does not do: two `get` calls with one proof give two `&mut` to one table.
//! That is the rule the code follows with each table (one open at a time, no call back
//! into the module while it is open), and it stays a rule, not a type.

use core::cell::UnsafeCell;
use core::marker::PhantomData;

/// The holder runs on the main thread. Not `Send` and not `Sync`: it cannot leave the thread
/// it was made on.
pub struct MainThread(PhantomData<*mut ()>);

impl MainThread {
    /// The proof, where the caller is the main thread by construction. The three places:
    /// `shadow::dispatch_inner` after its thread check, `lua::main_thread_gate` behind its
    /// check, and `worker_events::drain` after its check.
    pub unsafe fn claim() -> MainThread {
        debug_assert!(crate::shadow::on_main_thread(), "a claim of the main thread off it");
        MainThread(PhantomData)
    }

    /// The proof when the calling thread is the main thread, else `None`: for a path that
    /// any thread can reach, such as a call of the device.
    pub fn here() -> Option<MainThread> {
        crate::shadow::on_main_thread().then(|| MainThread(PhantomData))
    }

    /// A test runs on its own thread, which is its main thread.
    #[cfg(test)]
    pub fn for_test() -> MainThread {
        MainThread(PhantomData)
    }
}

/// A table of the main thread. `get` needs the proof.
pub struct MainCell<T>(UnsafeCell<T>);

// The cell is shared between threads as a static, but only the main thread opens it.
unsafe impl<T> Sync for MainCell<T> {}

impl<T> MainCell<T> {
    pub const fn new(value: T) -> MainCell<T> {
        MainCell(UnsafeCell::new(value))
    }

    /// The table, for a holder of the proof. One open at a time (see the module).
    #[inline(always)]
    pub fn get<'a>(&'a self, _: &'a MainThread) -> &'a mut T {
        // The proof says the main thread, and the rule says one open at a time.
        unsafe { &mut *self.0.get() }
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
