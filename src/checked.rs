//! The open check of the plugin's tables: [`Checked`] is the inside of
//! `main_thread::MainCell` and `sync::LockCell`. An open gives an [`Open`] guard, which keeps
//! the place of the open until it ends. A second open of a table while the first is live is a
//! fault (two `&mut` to one table): the check finds it at the second open and logs both
//! places, once for each pair.

use core::cell::{Cell, UnsafeCell};
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::ptr::NonNull;

pub struct Checked<T> {
    value: UnsafeCell<T>,
    /// The place of the open that is live now, or [`None`].
    opened_at: Cell<Option<&'static Location<'static>>>,
}

impl<T> Checked<T> {
    pub const fn new(value: T) -> Checked<T> {
        Checked { value: UnsafeCell::new(value), opened_at: Cell::new(None) }
    }

    /// Opens the table. The caller holds the proof that no other thread opens it now.
    ///
    /// # Safety
    ///
    /// No other thread opens or uses this table while the guard lives. The proof of it is the
    /// caller's: the main thread for `main_thread::MainCell`, the lock for `sync::LockCell`. The
    /// caller also does not open the table again while the guard lives: a second open gives a
    /// second `&mut` to the table, which the check logs but does not stop.
    #[track_caller]
    #[inline(always)]
    pub unsafe fn open(&self) -> Open<'_, T> {
        let here = Location::caller();
        let before = self.opened_at.replace(Some(here));
        if let Some(first) = before {
            second_open(first, here);
        }
        Open { value: unsafe { NonNull::new_unchecked(self.value.get()) }, opened_at: &self.opened_at, before, table: PhantomData }
    }
}

/// An open table, or a part of it ([`Open::map`]). The table closes when the guard ends.
pub struct Open<'a, T: ?Sized> {
    value: NonNull<T>,
    opened_at: &'a Cell<Option<&'static Location<'static>>>,
    /// The open before this one: [`None`] unless this one is a second open.
    before: Option<&'static Location<'static>>,
    table: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Open<'a, T> {
    /// The guard of a part of the table: the table stays open until it ends.
    #[inline(always)]
    pub fn map<U: ?Sized>(self, part: impl FnOnce(&mut T) -> &mut U) -> Open<'a, U> {
        let this = ManuallyDrop::new(self);
        let value = NonNull::from(part(unsafe { &mut *this.value.as_ptr() }));
        Open { value, opened_at: this.opened_at, before: this.before, table: PhantomData }
    }

    /// As [`Open::map`] for a part that can be missing: without it the table closes.
    #[inline(always)]
    pub fn filter_map<U: ?Sized>(self, part: impl FnOnce(&mut T) -> Option<&mut U>) -> Option<Open<'a, U>> {
        let value = NonNull::from(part(unsafe { &mut *self.value.as_ptr() })?);
        let this = ManuallyDrop::new(self);
        Some(Open { value, opened_at: this.opened_at, before: this.before, table: PhantomData })
    }
}

impl<T: ?Sized> Deref for Open<'_, T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &T {
        unsafe { self.value.as_ref() }
    }
}

impl<T: ?Sized> DerefMut for Open<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.value.as_mut() }
    }
}

impl<T: ?Sized> Drop for Open<'_, T> {
    #[inline(always)]
    fn drop(&mut self) {
        self.opened_at.set(self.before);
    }
}

/// The pairs of places that were logged.
static LOGGED: std::sync::Mutex<Vec<(usize, usize)>> = std::sync::Mutex::new(Vec::new());
static SECOND_OPENS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cold]
#[inline(never)]
fn second_open(first: &'static Location<'static>, second: &'static Location<'static>) {
    SECOND_OPENS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pair = (first as *const _ as usize, second as *const _ as usize);
    let Ok(mut logged) = LOGGED.lock() else { return };
    if logged.contains(&pair) {
        return;
    }
    logged.push(pair);
    drop(logged);
    crate::log!("tables: a table open at {first} is opened again at {second}");
}

/// The number of second opens since the start, for the report.
pub fn second_opens() -> u32 {
    SECOND_OPENS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_closes_when_its_guard_ends() {
        let cell = Checked::new(1u32);
        unsafe {
            *cell.open() += 1;
            assert!(cell.opened_at.get().is_none());
            *cell.open() += 1;
            assert_eq!(*cell.open(), 3);
        }
        assert!(cell.opened_at.get().is_none());
    }

    #[test]
    fn a_second_open_while_the_first_is_live_is_found() {
        let cell = Checked::new(1u32);
        unsafe {
            let first = cell.open();
            let place = cell.opened_at.get().unwrap();
            assert_eq!(place.line(), line!() - 2);
            let before = second_opens();
            let second = cell.open();
            assert!(second_opens() > before);
            drop(second);
            // The first open is the live one again.
            assert_eq!(cell.opened_at.get().unwrap().line(), place.line());
            drop(first);
        }
        assert!(cell.opened_at.get().is_none());
    }
}
