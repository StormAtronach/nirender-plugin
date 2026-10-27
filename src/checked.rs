//! The open check of the plugin's tables: [`Checked`] is the inside of
//! `main_thread::MainCell` and `sync::LockCell`. An open gives an [`Open`] guard, which keeps
//! the place of the open until it ends. A second open of a table while the first is live
//! would give two `&mut` to one table: the check refuses it with a panic that names both
//! places, as `RefCell::borrow_mut` does.

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

    /// Opens the table. A second open while the guard of the first lives panics, and the
    /// table stays as the first open has it.
    ///
    /// # Safety
    ///
    /// No other thread opens or uses this table while the guard lives. The proof of it is the
    /// caller's: the main thread for `main_thread::MainCell`, the lock for `sync::LockCell`.
    #[track_caller]
    #[inline(always)]
    pub unsafe fn open(&self) -> Open<'_, T> {
        let here = Location::caller();
        if let Some(first) = self.opened_at.get() {
            second_open(first, here);
        }
        self.opened_at.set(Some(here));
        Open { value: unsafe { NonNull::new_unchecked(self.value.get()) }, opened_at: &self.opened_at, table: PhantomData }
    }
}

/// An open table, or a part of it ([`Open::map`]). The table closes when the guard ends.
pub struct Open<'a, T: ?Sized> {
    value: NonNull<T>,
    opened_at: &'a Cell<Option<&'static Location<'static>>>,
    table: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Open<'a, T> {
    /// The guard of a part of the table: the table stays open until it ends.
    #[inline(always)]
    pub fn map<U: ?Sized>(self, part: impl FnOnce(&mut T) -> &mut U) -> Open<'a, U> {
        let this = ManuallyDrop::new(self);
        let value = NonNull::from(part(unsafe { &mut *this.value.as_ptr() }));
        Open { value, opened_at: this.opened_at, table: PhantomData }
    }

    /// As [`Open::map`] for a part that can be missing: without it the table closes.
    #[inline(always)]
    pub fn filter_map<U: ?Sized>(self, part: impl FnOnce(&mut T) -> Option<&mut U>) -> Option<Open<'a, U>> {
        let value = NonNull::from(part(unsafe { &mut *self.value.as_ptr() })?);
        let this = ManuallyDrop::new(self);
        Some(Open { value, opened_at: this.opened_at, table: PhantomData })
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
        self.opened_at.set(None);
    }
}

#[cold]
#[inline(never)]
fn second_open(first: &'static Location<'static>, second: &'static Location<'static>) -> ! {
    panic!("tables: a table open at {first} is opened again at {second}");
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
    fn a_part_keeps_the_table_open_until_it_ends() {
        let cell = Checked::new([1u32, 2]);
        unsafe {
            let part = cell.open().map(|pair| &mut pair[1]);
            assert!(cell.opened_at.get().is_some());
            drop(part);
            assert!(cell.opened_at.get().is_none());
            assert!(cell.open().filter_map(|pair| pair.get_mut(5)).is_none());
        }
        assert!(cell.opened_at.get().is_none());
    }

    #[test]
    #[should_panic(expected = "is opened again at")]
    fn a_second_open_while_the_first_is_live_panics() {
        let cell = Checked::new(1u32);
        unsafe {
            let _first = cell.open();
            let _second = cell.open();
        }
    }
}
