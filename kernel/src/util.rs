//
// BlightOS Kernel
//
// Utility Module
//   Use this in the absence of the std library.
// 

use crate::arch::kearly_console;
pub use core::fmt::Write;

pub struct ConsoleOut;
impl Write for ConsoleOut {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        kearly_console::print_str(_s.as_bytes());
        Ok(())
    }
}

macro_rules! klog {
    ($($arg:tt)*) => {
        let mut kern_console = ConsoleOut{};
        let _ = write!(&mut kern_console, $($arg)*);
    };
}

//
// Raw memory manipulation
//
#[inline(never)]
pub unsafe fn raw_memcpy(dst: usize, src: usize, n: usize) {
    let mut destp : *mut u8 = dst as *mut u8;
    let mut srcp  : *mut u8 = src as *mut u8;

    for _i in 0..n {
        *destp = *srcp;
        destp  = destp.wrapping_add(1);
        srcp   = srcp.wrapping_add(1); 
    }
}


//
// Per-CPU
//
macro_rules! percpu_global {
    ($($svis:vis $name:ident: $type:ty = $value:expr;)*) => {
        $(
            #[used]
            #[no_mangle]
            #[link_section = ".percpu_global"]
            $svis static $name: $type = $value;
        )*
    };
}
pub(crate) use percpu_global;


//
// Synchronization
//

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

pub struct Spinlock<T> {
    is_locked: AtomicBool,
    data: UnsafeCell<T>,
}

pub struct SpinlockCriticalSection<'a, T: 'a> {
    sl: &'a Spinlock<T>,
}

unsafe impl<T> Send for Spinlock<T> {}
unsafe impl<T> Sync for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            is_locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinlockCriticalSection<'_, T> {
        loop {
            if !self.is_locked.swap(true, Ordering::Acquire) {
                return SpinlockCriticalSection { sl: self };
            }

            while self.is_locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinlockCriticalSection<'_, T>> {
        if !self.is_locked.swap(true, Ordering::AcqRel) {
            // is_locked was false and now we have atomically swapped it to true,
            // so no one else has access to this data.
            return Some(SpinlockCriticalSection { sl: self });
        }
        None
    }
}

impl<'a, T: 'a> Drop for SpinlockCriticalSection<'a, T> {
    fn drop(&mut self) {
        self.sl.is_locked.store(false, Ordering::Release);
    }
}

impl<'a, T> Deref for SpinlockCriticalSection<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.sl.data.get() }
    }
}

impl<'a, T> DerefMut for SpinlockCriticalSection<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.sl.data.get() }
    }
}
