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

macro_rules! div_round_up {
    ($num:expr, $denom:expr) => {
        (($num + $denom - 1) / $denom)
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


///
/// ELF Loader
///
///
use alloc::{slice, sync::Arc};
use alloc::vec::Vec;
use crate::fs::MountPoint;
use crate::drivers::storage::IOCompletion;

#[derive(Debug)]
pub struct ELFSegment {
    pub p_type:     u32,
    pub p_flags:    u32,
    pub p_offset:   usize, // From the beginning of the file
    pub p_vaddr:    usize, // Virtual address this segment should be loaded at
    pub p_paddr:    usize, // Irrelevant to us
    pub p_filesz:   usize, // # of bytes in the file image of the segment; may be 0
    pub p_memsz:    usize, // # of bytes in the memory image of the segment
    pub p_align:    usize,
}
impl ELFSegment {
    pub const P_TYPE_LOAD:      u32 = 1;
    pub const P_FLAGS_EXEC:     u32 = 1;
    pub const P_FLAGS_WRITE:    u32 = 2;
    pub const P_FLAGS_READ:     u32 = 4;
}
pub struct ELFBinary{
    mnt:            Arc<MountPoint>,
    hnd:            usize,
    // From ELF Header
    elf_type:           u16,
    elf_machine:        u16,
    pub elf_entry:          usize, // Virt.Addr. to jump to
    elf_hdr_sz:         u16,   // Size of the ELF header

    // We only care about program headers that have to be loaded
    // (p_type == PT_LOAD) for now
    elf_prg_hdr_off:    usize, // Program Header Table
    elf_prg_hdr_cnt:    u16, // # Entries in Program Header Table
    elf_prg_hdr_entsz:  u16, // Size of a Program Header Table Entry in bytes

    // Don't really care about sections as the program headers already merges
    // them to represent the runtime image of the program!
    elf_sec_hdr_off:    usize, // Section Header Table
    elf_sec_hdr_cnt:    u16, // # Entries in Section Header Table
    elf_sec_hdr_entsz:  u16, // Size of a Section Header Table Entry in bytes

    pub segments:           Vec<ELFSegment>

}
impl ELFBinary {
    // ELF-64 Header
    // OFF    Size     Name
    // -----------------------------
    // 0      16x1     e_ident[16];
    // -----------------------------
    // 16     2        e_type;
    // 18     2        e_machine;
    // 20     4        e_version;
    // -----------------------------
    // 24     8        e_entry;
    // -----------------------------
    // 32     8        e_phoff;
    // -----------------------------
    // 40     8        e_shoff;
    // -----------------------------
    // 48     4        e_flags;
    // 52     2        e_ehsize;
    // 54     2        e_phentsize;
    // -----------------------------
    // 56     2        e_phnum;
    // 58     2        e_shentsize;
    // 60     2        e_shnum;
    // 62     2        e_shstrndx;
    //
    const ELF_CLASS_64:     u8 = 2;
    const ELF_TYPE_EXE:     u16 = 2;
    #[cfg(target_arch = "x86_64")]
    const ELF_MACHINE_X64:  u16 = 0x3E;
    #[cfg(target_arch = "aarch64")]
    const ELF_MACHINE_ARM:  u16 = 0xB7;
    
    // Owns the file handle and closes it when goes out of scope
    pub fn from_file(mnt: Arc<MountPoint>, file_handle: usize) -> Option<Self> {
        // klog!("Loading ELF from mnt {}, file handle: {})\n", mnt.name, file_handle);
        // Read and decode the header
        let mut buf = [0 as u8; 512];
        let u64b;
        unsafe {
            u64b = core::slice::from_raw_parts_mut(buf.as_ptr() as *mut u64, 512);
        }
        let ioc = mnt.fread(file_handle, 0, &mut buf);
        if let IOCompletion::Successful(_len) = ioc {
            if !(buf[0] == 0x7f && buf[1] == b'E' && buf[2] == b'L' &&
                buf[3] == b'F' && buf[4] == Self::ELF_CLASS_64) {
                klog!("Not an ELF-64 file!\n");
                return None;
            }
            let elf_type = (u64b[2] & 0xFFFF) as u16;
            let elf_machine = ((u64b[2] >> 16) & 0xFFFF) as u16;

            if elf_type != Self::ELF_TYPE_EXE {
                klog!("Not an Executable ELF-64\n");
                return None;
            }
            #[cfg(target_arch = "x86_64")]
            if elf_machine != Self::ELF_MACHINE_X64 {
                klog!("Not an x86_64 Executable!\n");
                return None;
            }
            #[cfg(target_arch = "aarch64")]
            if elf_machine != Self::ELF_MACHINE_ARM {
                klog!("Not an AARCH64 Executable!\n");
                return None;
            }

            let entry   = u64b[3];
            let phoff   = u64b[4];
            let shoff   = u64b[5];
            let ehsz    = (u64b[6] >> 32) & 0xFFFF;
            let phentsz = (u64b[6] >> 48) & 0xFFFF;
            let phcnt   = u64b[7] & 0xFFFF;
            let shensz  = (u64b[7] >> 16) & 0xFFFF;
            let shcnt   = (u64b[7] >> 32) & 0xFFFF;
            // Enumerate the loadable segments
            let mut segments: Vec<ELFSegment> = Vec::new();
            let ioc = mnt.fread(file_handle, phoff as usize, &mut buf);
            if let IOCompletion::Successful(len) = ioc {
                // TODO Consider checking for the end of buffer and reading more
                // if necessary
                if phcnt * phentsz > len as u64 {
                    panic!("Too many ELF segments! Support not implemented");
                }
                // klog!("Read {} bytes of the program headers\n", len);
                for i in 0..phcnt {
                    let offset = (i * (phentsz / 8)) as usize;
                    segments.push(ELFSegment {
                        p_type:     (u64b[offset + 0] & 0xFFFFFFFF) as u32,
                        p_flags:    (u64b[offset + 0] >> 32) as u32,
                        p_offset:   u64b[offset + 1] as usize,
                        p_vaddr:    u64b[offset + 2] as usize,
                        p_paddr:    u64b[offset + 3] as usize,
                        p_filesz:   u64b[offset + 4] as usize,
                        p_memsz:    u64b[offset + 5] as usize,
                        p_align:    u64b[offset + 6] as usize,
                    });
                }
                
            }
            // Return
            return Some(
                Self {
                    mnt:                mnt,
                    hnd:                file_handle,
                    elf_type:           elf_type,
                    elf_machine:        elf_machine,
                    elf_entry:          entry as usize,
                    elf_hdr_sz:         ehsz as u16,
                    elf_prg_hdr_off:    phoff as usize,
                    elf_prg_hdr_entsz:  phentsz as u16,
                    elf_prg_hdr_cnt:    phcnt as u16,
                    elf_sec_hdr_off:    shoff as usize,
                    elf_sec_hdr_entsz:  shensz as u16,
                    elf_sec_hdr_cnt:    shcnt as u16,
                    segments:           segments
                }
            );
        }
        None 
    }

    pub fn load_segment(&self, seg_index: usize, dest_addr: usize) -> usize {
        let mut xfer_len = 0;
        let mut left = self.segments[seg_index].p_memsz;
        let mut foff = self.segments[seg_index].p_offset;
        let mut dest = dest_addr;
        while left > 0 {
            let bufsz = core::cmp::min(512, left);
            let buf;
            unsafe {
                buf = slice::from_raw_parts_mut(dest as *mut u8, bufsz);
            }
            let ioc = self.mnt.fread(self.hnd, foff, buf);
            if let IOCompletion::Successful(len) = ioc {
                // klog!("<{:X}->{:X} len:{}>", foff, dest, len);
                foff    += len;
                dest    += len;
                xfer_len+= len;
                left    -= len;
            } else {
                break; // IO Error
            }
        }
        xfer_len
    }
    pub fn log_header(&self) {
        klog!("ELF: Type: {:X}, Machine: {:X}, Entry: {:X}, HeaderSz: {}\n", 
            self.elf_type, self.elf_machine, self.elf_entry, self.elf_hdr_sz);
        klog!("  {} Headers: Offset: {}, Entry Size:{}\n",
            self.elf_prg_hdr_cnt, self.elf_prg_hdr_off, self.elf_prg_hdr_entsz);
        klog!("  {} Sections: Offset: {}, Entry Size: {}\n",
            self.elf_sec_hdr_cnt, self.elf_sec_hdr_off, self.elf_sec_hdr_entsz);
    }
    
}

impl Drop for ELFBinary {
    fn drop(&mut self) {
        self.mnt.fclose(self.hnd);
    }
}