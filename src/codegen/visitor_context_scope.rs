use std::cell::Cell;
use std::thread::LocalKey;

struct Entry<T> {
    identity: usize,
    data: *const (),
    read: unsafe fn(*const ()) -> T,
    previous: *const Entry<T>,
}

pub(crate) struct CallbackAccessorScope<T> {
    active: Cell<*const Entry<T>>,
}

impl<T: 'static> CallbackAccessorScope<T> {
    pub(crate) const fn new() -> Self {
        Self {
            active: Cell::new(std::ptr::null()),
        }
    }

    pub(crate) fn with<F, R>(scope: &'static LocalKey<Self>, identity: usize, read: F, call: impl FnOnce() -> R) -> R
    where
        F: Fn() -> T,
    {
        scope.with(|state| {
            let entry = Entry {
                identity,
                data: std::ptr::from_ref(&read).cast(),
                read: read_erased::<F, T>,
                previous: state.active.get(),
            };
            state.active.set(std::ptr::from_ref(&entry));
            let _reset = ScopeReset {
                active: &state.active,
                previous: entry.previous,
            };
            call()
        })
    }

    pub(crate) fn read(&self, identity: usize) -> Option<T> {
        let mut entry = self.active.get();
        // The list contains only live with frames on this thread. Each guard restores
        // its predecessor before the entry or reader can drop. ~keep
        while let Some(active) = unsafe { entry.as_ref() } {
            if active.identity == identity {
                return Some(unsafe { (active.read)(active.data) });
            }
            entry = active.previous;
        }
        None
    }
}

unsafe fn read_erased<F: Fn() -> T, T>(data: *const ()) -> T {
    // with stores a pointer to the same F used to select this function. ~keep
    let read = unsafe { &*data.cast::<F>() };
    read()
}

struct ScopeReset<'a, T> {
    active: &'a Cell<*const Entry<T>>,
    previous: *const Entry<T>,
}

impl<T> Drop for ScopeReset<'_, T> {
    fn drop(&mut self) {
        self.active.set(self.previous);
    }
}
