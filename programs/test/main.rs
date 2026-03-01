//
// BlightOS - User-space test suite
//

#![no_std]
extern crate alloc; 

use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::task::*;
use alloc::vec::Vec;
use alloc::boxed::Box;
use rtlib::heap::Malloc;

#[no_mangle]
extern "C" 
fn main() {
    println!("This program tests some of the basic functionalities of the \
            BlightOS user-space runtime library.");
    print!("[Test 1] Current Process/Task Information:");
    let t = Task::current();
    println!("TID: {}, PID: {}, name: {} - Running on CPU {}", t.tid, t.pid,
            t.name(), Task::current_cpu());
    let mut proc = Process::current();
    proc.get_info();
    println!("{:X?}", proc);

    println!("\n[Test 2] Spawning a new task:");
    let new_task = Task::spawn(|arg: usize| {
        println!("Hello from the new task! arg = {}", arg);
        let t = Task::current();
        println!("TID: {}, PID: {}, name: {} - Running on CPU {}",
                t.tid, t.pid, t.name(), Task::current_cpu());
    }, 12345, "TestTask1");
    if let Some(t) = new_task {
        println!("Task Information returned by Spawn: TID: {}, PID: {}, name: {}",
                    t.tid, t.pid, t.name());
        Task::join(t.tid);
        println!("Joined the task with TID {}", t.tid);
    } else {
        println!("\nFailed to spawn the task");
    }

    println!("\n[Test 3] Co-operative multitasking test:");
    let task_a = Task::spawn(|_arg: usize| {
        for i in 0..5 {
            print!("<B, i:{}>", i);
            Task::yield_now();
        }
    }, 5, "TaskB");
    for i in 0..5 {
        print!("<A, i:{}>", i);
        Task::yield_now();
    }
    Task::join(task_a.unwrap().tid);
    println!("");

    println!("\n[Test 4] Sleep test:");
    let task_c = Task::spawn(|_arg: usize| {
        println!("<Task C> Sleeping for five 500ms intervals...");
        for i in 0..5 {
            print!("<C, i:{}>", i);
            Task::sleep(core::time::Duration::from_millis(500));
        }
        println!("<Task C> Exiting now!");
    }, 5, "TaskC");
    for _i in 0..20 {
        print!("<A>");
        Task::sleep(core::time::Duration::from_millis(100));
    }
    Task::join(task_c.unwrap().tid);
    
    println!("\n[Test 4] Heap test 1: 2MBs of small allocations");
    rtlib::heap::Malloc::init();
    let alloc_size = 64;
    let total_alloc_size = 2 * 1024 * 1024; // 2 MB
    let alloc_count = total_alloc_size / alloc_size;
    println!("Allocating {} blocks of {} bytes each (total: {} bytes). \
                Physical memory before allocations:", 
                alloc_count, alloc_size, total_alloc_size);
    println!("Heap base: {:#x}, Heap size: {} bytes", Malloc::heap_base(),
                                                        Malloc::heap_size());
    txt_dump("machine:/ram");
    println!("");
    {
        let mut ptrs: Vec<Box<[u8; 64]>> = Vec::with_capacity(alloc_count);
        for _i in 0..alloc_count {
            let ptr = Box::new([0u8; 64]);
            if ptr.is_empty() {
                println!("Failed to allocate {} bytes", alloc_size);
                break;
            }   
            ptrs.push(ptr);
        }
    }
    

    let released = rtlib::heap::Malloc::release_unused_memory();
    println!("Deallocated all blocks. Released {} bytes of unused heap memory \
                back to kernel.", released);
    println!("Heap base: {:#x}, Heap size: {} bytes", Malloc::heap_base(),
                                                        Malloc::heap_size());
    println!("Physical memory after deallocations:");
    txt_dump("machine:/ram");

    println!("\nAll tests completed.");
    proc.get_info();
    println!("{:X?}", proc);
    exit(0);
}

fn txt_dump(path: &str) {
    let mut buff: [u8; 64] = [0; 64];
    match fopen(path) {
        Some(fd)    => {
            loop {
                let cnt = fread(fd, &mut buff);
                if cnt > 0 {
                    print!("{}", str::from_utf8(&buff[0..cnt]).unwrap());
                } else {
                    break;
                }
            }
            fclose(fd);
        },
        None        => {
            print!("\nPath {} doesn't exist", path);
        }
    }
}


