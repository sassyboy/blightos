///
/// BlightOS Kernel
///
/// Utility Module
///   Use this in the absence of the std library.
///

use crate::arch::*;
use crate::sched::Preemption;
pub use core::fmt::Write;

pub struct ConsoleOut;
impl Write for ConsoleOut {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        kearly_console::print_str(_s.as_bytes());
        Ok(())
    }
}

pub struct DebugOut;
impl Write for DebugOut {
    fn write_str(&mut self, _s: &str) -> core::fmt::Result {
        kdebug_console::print_str(_s.as_bytes());
        Ok(())
    }
}

pub static KLOG_LOCK: Spinlock<usize> = Spinlock::new(0);
macro_rules! klog {
    ($($arg:tt)*) => {
        let kllock = KLOG_LOCK.lock();
        let mut kern_console = ConsoleOut{};
        let _ = write!(&mut kern_console, $($arg)*);
        drop(kllock);
    };
}

pub fn dump_memory(base: usize, qwords: usize) {
    unsafe {
        let mut datap: *mut usize = base as *mut usize;
        for _ in 0..qwords {
            klog!("{:X}: {:016X}\n", datap as usize, *datap);
            datap = datap.wrapping_add(1);
        }
    }
}

pub fn dump_memory_columns(base: usize, qwords: usize, ncols: usize) {
    unsafe {
        let mut datap: *mut usize = base as *mut usize;
        for i in 0..qwords {
            if i % ncols == 0 {
                klog!("{:X}: ", datap as usize);
            }
            klog!("{:016X} ", datap.read_volatile());
            if i % ncols == ncols - 1 {
                klog!("\n");
            }
            datap = datap.wrapping_add(1);
        }
        klog!("\n");
    }
}

pub fn dump_memory_ascii(base: usize, nbytes: usize) {
    let ptr = base as *const u8; // Cast to byte pointer
    // Calculate size and create a slice
    let byte_array = unsafe {
        core::slice::from_raw_parts(ptr, nbytes)
    };
    klog!("{}\n", str::from_utf8(byte_array).unwrap());
}

//
// Basic arithmetics
//
macro_rules! round_up {
    ($num:expr, $multiple:expr) => {
        (($num + $multiple - 1) / $multiple) * $multiple
    };
}

macro_rules! round_down {
    ($num:expr, $multiple:expr) => {
        ($num / $multiple) * $multiple
    };
}

///
/// Provides a convenient way of a defining `percpu` global variables, i.e.,
/// variables of which there is one copy per CPU. Reading/Writing from/to a
/// `percpu` variable accesses the instance of the variable belonging to the
/// CPU currently performing the operation.
/// 
/// Example:
/// ```
/// percpu_global!{
///     pub MY_PERCPU_COUNTER : usize = 0;
///     MY_CUSTOM_VAR: some_struct = some_struct::new();
/// }
/// 
/// ... 
/// 
/// // Copying a value into a percpu var:
/// MY_PERCPU_COUNTER.write(x); 
/// 
/// // Accessing the percpu var via an immutable reference
/// klog!("My counter is {}", MY_PERCPU_COUNTER.borrow());
/// 
/// // Accessing the percpu var via a mutable reference
/// let my_custom_var = MY_CUSTOM_VAR.borrow_mut(); // Get a mutable reference
/// my_custom_var.field_x = x;
/// my_custom_var.func_x(arg1, etc);
/// ```
///
macro_rules! percpu_global {
    ($($svis:vis $name:ident: $type:ty = $value:expr;)*) => {
        $(
            #[used]
            #[no_mangle]
            #[link_section = ".percpu_global"]
            $svis static $name: PerCpuGlobal<$type> = PerCpuGlobal::new($value);
        )*
    };
}

///
/// Provides a type-safe/thread-safe encapsulation of `percpu` variables defined 
/// using the `percpu_global!` macro.
/// 
/// Example:
/// ```
/// percpu_global!{
///     pub MY_PERCPU_COUNTER : usize = 0;
///     MY_CUSTOM_VAR: some_struct = some_struct::new();
/// }
/// 
/// ... 
/// 
/// // Copying a value into a percpu var:
/// MY_PERCPU_COUNTER.write(x); 
/// 
/// // Accessing the percpu var via an immutable reference
/// klog!("My counter is {}", MY_PERCPU_COUNTER.borrow());
/// 
/// // Accessing the percpu var via a mutable reference
/// let my_custom_var = MY_CUSTOM_VAR.borrow_mut(); // Get a mutable reference
/// my_custom_var.field_x = x;
/// my_custom_var.func_x(arg1, etc);
/// ```
///
pub struct PerCpuGlobal<T>{
    var: T
}
impl<T> PerCpuGlobal<T> {
    pub const fn new(val: T) -> Self {
        Self {
            var: val
        }
    }

    ///
    /// Copies the value of `val` into the percpu variable while holding the
    /// preemption lock to avoid data inconsistency due to the CPU switching
    /// to another task that accesses the same percpu variable.
    /// 
    pub fn write(&self, val: T) {
        // Scheduling a new task in the middle of accessing a percpu variable
        // can lead to state inconsistencies and undefined behavior
        let _lock = Preemption::lock();
        *(percpu_borrow_mut(&(self.var))) = val;
    }

    ///
    /// Returns a <b>immutable</b> reference to the percpu variable.
    /// The <b>caller must hold the preemption lock</b> to ensure data
    /// consistency.
    /// 
    pub fn borrow(&self) -> &T {
        percpu_borrow(&(self.var))
    }

    ///
    /// Returns a <b>mutable</b> reference to the percpu variable.
    /// The <b>caller must hold the preemption lock</b> to ensure data
    /// consistency.
    /// 
    pub fn borrow_mut(&self) -> &mut T {
        percpu_borrow_mut(&(self.var))
    }
}


//
// Synchronization
//

use core::{
 cell::UnsafeCell, ops::{Deref, DerefMut}, sync::atomic::{AtomicBool, Ordering}
};

pub struct Spinlock<T> {
    is_locked:  AtomicBool,
    data:       UnsafeCell<T>,
}

pub struct SpinlockCriticalSection<'a, T: 'a> {
    sl:     &'a Spinlock<T>,
    ints:   bool,
}

unsafe impl<T> Send for Spinlock<T> {}
unsafe impl<T> Sync for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            is_locked:  AtomicBool::new(false),
            data:       UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinlockCriticalSection<'_, T> {
        loop {
            let ie = crate::arch::cpu_ints_enabled();
            crate::arch::cpu_disable_ints();
            if self.is_locked.swap(true, Ordering::AcqRel) == false {
                return SpinlockCriticalSection { sl: self, ints: ie };
            }
            crate::arch::cpu_restore_ints(ie);

            while self.is_locked.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinlockCriticalSection<'_, T>> {
        let ie = crate::arch::cpu_ints_enabled();
        crate::arch::cpu_disable_ints();
        if !self.is_locked.swap(true, Ordering::AcqRel) {
            // is_locked was false and now we have atomically swapped it to true,
            // so no one else has access to this data.
            return Some(SpinlockCriticalSection { sl: self, ints: ie });
        }
        crate::arch::cpu_restore_ints(ie);
        None
    }
}

impl<'a, T: 'a> Drop for SpinlockCriticalSection<'a, T> {
    fn drop(&mut self) {
        self.sl.is_locked.store(false, Ordering::Release);
        crate::arch::cpu_restore_ints(self.ints);
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
