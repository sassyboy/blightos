//
// BlightOS - User-space test suite
//

#![no_std]
extern crate alloc; 
use rtlib::time::TimeStampCounter;
use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::task::*;
use alloc::vec::Vec;
use alloc::boxed::Box;
use rtlib::heap::Malloc;
use rtlib::graphics::framebuffer::*;
use rtlib::graphics::png::PngImage;
use rtlib::audio::{Playback, beeper::*, wav::*};

#[no_mangle]
fn main() {
    println!("This program tests some of the basic functionalities of the \
            BlightOS user-space runtime library.");
    
    println!("\n[Test 1] Current Process/Task Information:");
    task_info_test();

    println!("\n[Test 2] Spawning a new task:");
    task_spawn_test();

    println!("\n[Test 3] Co-operative multitasking:");
    co_op_sched_test();

    println!("\n[Test 4] Sleep:");
    sleep_test();

    println!("\n[Test 5] No-Execute (NX) Protection:");
    stack_exec_test();

    wait_for_enter_and_clear();

    println!("\n[Test 6] Heap management:");
    heap_test();

    println!("\n[Test 7] Dynamic stack growth:");
    stack_growth_test();

    wait_for_enter_and_clear();

    // Test framebuffer access
    println!("\n[Test 8] Framebuffer access:");
    framebuffer_test();

    // PNG Loading test
    println!("\n[Test 9] PNG loading:");
    png_test();

    wait_for_enter_and_clear();

    // Audio Playback test
    println!("\n[Test 10] Audio playback:");
    beeper_test(50, 4);
    wav_audio_test(&Path::from("res/sfx/click.wav"));
    println!("\nLarger WAV file test:");
    wav_audio_test(&Path::from("res/music/doomesk.wav"));

    // All tests done
    println!("\nAll tests completed.");
    let mut proc = Process::current();
    proc.get_info();
    println!("{:X?}", proc);
    exit(0);
}

fn task_info_test() {
    let t = Task::current();
    println!("TID: {}, PID: {}, name: {} - Running on CPU {}", t.tid, t.pid,
            t.name(), Task::current_cpu());
    let mut proc = Process::current();
    proc.get_info();
    println!("{:X?}", proc);
}

fn co_op_sched_test() {
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
}

fn task_spawn_test() {
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
}

fn sleep_test() {
    let mut tsc = TimeStampCounter::new();
    let t0 = tsc.current_as_nanos();
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
    let t1 = tsc.current_as_nanos();
    println!("\nTotal elapsed time according to TSC: {} ms",
                (t1 - t0) as f64 / 1_000_000.0);
}

fn heap_test() {
    rtlib::heap::Malloc::init();
    let alloc_size = 64;
    let total_alloc_size = 20 * 1024 * 1024; // 20 MB
    let alloc_count = total_alloc_size / alloc_size;
    println!("20 MBs of small allocations");
    println!("Heap base: {:#x}, Heap size: {} bytes, Free list:",
                Malloc::heap_base(), Malloc::heap_size());
    rtlib::heap::Malloc::dump_free_list();
    println!("Physical memory before allocations:");
    txt_dump(&Path::from("machine:/ram"));
    // Allocate multiple blocks of memory
    println!("\n1) Allocating {} blocks of {} bytes each (total: {} bytes).", 
                alloc_count, alloc_size, total_alloc_size);
    {
        let mut ptrs: Vec<Box<[u8; 64]>> = Vec::new();
        for _i in 0..alloc_count {
            let mut ptr = Box::new([0u8; 64]);
            for j in 0..64 {
                ptr[j] = j as u8;
            }
            if ptr.is_empty() {
                println!("Failed to allocate {} bytes", alloc_size);
                break;
            }   
            ptrs.push(ptr);
        }

        // Verify the allocated memory
        println!("\n2) Verifying the allocated memory...");
        for (i, ptr) in ptrs.iter().enumerate() {
            for j in 0..64 {
                if ptr[j] != j as u8 {
                    println!("Memory corruption detected at block {}, byte {}!", 
                            i, j);
                    return;
                }
            }
        }
    }
    
    // All blocks should be deallocated when `ptrs` goes out of scope
    println!("\n3) Deallocated all blocks. Free list:");
    rtlib::heap::Malloc::dump_free_list();
    println!("Physical memory after deallocation:");
    txt_dump(&Path::from("machine:/ram"));

    // Release unused memory back to the OS
    println!("\n4) Releasing unused memory... Current heap size: {} bytes",
                Malloc::heap_size());
    let released = rtlib::heap::Malloc::release_unused_memory();
    println!("Released {} bytes back to the OS.", released);
    
    println!("\nHeap base: {:#x}, Heap size: {} bytes. Free list:",
                Malloc::heap_base(), Malloc::heap_size());
    rtlib::heap::Malloc::dump_free_list();
    println!("Physical memory after heap release:");
    txt_dump(&Path::from("machine:/ram"));
}

fn stack_exec_test() {
    println!("Attempting to execute code on the stack...");
    let sgt_task = Task::spawn(|_arg: usize| {
        #[cfg(target_arch = "x86_64")]
        {
            let code: [u8; 2] = [0xC3, 0x00]; // x86_64 'ret' instruction
            let code_ptr = code.as_ptr() as usize;
            let func: extern "C" fn() = unsafe {
                core::mem::transmute(code_ptr)
            };
            // This should cause a page fault due to NX bit protection
            func();
        }
        #[cfg(target_arch = "aarch64")]
        {
            let code: [u8; 4] = [0xC0, 0x03, 0x5F, 0xD6]; // aarch64 'ret' inst.
            let code_ptr = code.as_ptr() as usize;
            let func: extern "C" fn() = unsafe {
                core::mem::transmute(code_ptr)
            };
            // This should cause a page fault due to NX bit protection
            func();
        }
    }, 0, "StackExecTest");
    Task::join(sgt_task.unwrap().tid);
}

fn stack_growth_test() {
    println!("Initial stack size is 16KB - Max per-task stack size is 16MB");
    txt_dump(&Path::from("machine:/ram"));
    let sgt_task = Task::spawn(|_arg: usize| {
        print!("  Allocating 8KB on the stack...");
        stack_growth_test_recursive(2);
        print!("  Allocating 20KB on the stack...");
        stack_growth_test_recursive(5);
        print!("  Allocating 400KB on the stack...");
        stack_growth_test_recursive(100);
        print!("  Allocating 10MB on the stack...");
        stack_growth_test_recursive(2500);
        print!("  Allocating 15.6MB on the stack...");
        stack_growth_test_recursive(4000);
        // The next allocation should fail and result in a page fault
        print!("  Allocating 16MB on the stack...");
        stack_growth_test_recursive(4096);
    }, 0, "StackGrowthTest");
    Task::join(sgt_task.unwrap().tid);
    txt_dump(&Path::from("machine:/ram"));
}

fn stack_growth_test_recursive(depth: usize) {
    if depth == 0 {
        println!("Passed");
        return;
    }
    // 4KB buffer to consume stack space
    let mut buffer = [0u8; 4096]; 
    for i in 0..buffer.len() {
        buffer[i] = depth as u8;
    }
    stack_growth_test_recursive(depth - 1);
    // Validate the buffer
    for i in 0..buffer.len() {
        if buffer[i] != depth as u8 {
            println!("Stack corruption detected at depth {}!", depth);
            return;
        }
    }
}

fn framebuffer_test() {
if let Some(mut fb) = Framebuffer::new() {
        // Backup the current framebuffer content 
        println!("Saving the current framebuffer content...");
        if fb.save_frame() {
            println!("Framebuffer content saved successfully.");
            // Spawn a new task to draw a pattern on the framebuffer
            let _fb_task = Task::spawn(fb_test_task, 0, "FrameBufferTest");
            // Wait for a while to let the user see the pattern
            Task::join(_fb_task.unwrap().tid);
            // Restore the original framebuffer content before exiting
            println!("Restoring the original framebuffer content...");
            fb.restore_frame();
            println!("Framebuffer content restored to original state.");
        } else {
            println!("Failed to save the framebuffer content.");
        }
    } else {
        println!("Failed to access the framebuffer.");
    }
}

fn fb_test_task(_arg: usize) {
    if let Some(mut fb) = Framebuffer::new() {
        // Draw a simple pattern on the framebuffer
        for row in 100..200 {
            for col in 100..200 {
                let color = if (row / 10 + col / 10) % 2 == 0 {
                    (255, 0, 0) // Red
                } else {
                    (0, 0, 255) // Blue
                };
                fb.set_pixel(row, col, color);
            }
        }
        fb.update();
        println!("Framebuffer Info - Width: {}, Height: {}, BPP: {}, Pitch: {}",
                    fb.width, fb.height, fb.bpp, fb.pitch);
        println!("Framebuffer pattern drawn. Check the display output. \
                    Press Enter to continue...");
        while stdio_read_byte() != b'\n' {}
    } else {
        println!("Failed to access the framebuffer.");
    }
}

fn png_test() {
    if let Some(mut fb) = Framebuffer::new() {
        png_test_load(&mut fb, &Path::from("res/test.png"),  300 , 0);
        png_test_load(&mut fb, &Path::from("res/testp.png"), 300, 300);
        png_test_load(&mut fb, &Path::from("res/testa.png"), 300, 600);
    } else {
        println!("Failed to access the framebuffer.");
    }
}

fn png_test_load(fb: &mut Framebuffer, path: &Path, y: u32, x: u32) {
    match PngImage::from_path(path) {
        Ok(mut png) => {
            println!("Loaded PNG image successfully: {}x{}, color type: {:?}",
                        png.img.width, png.img.height, png.img.color_type);
            match png.decode() {
                Ok(image) => {
                    let mut idx = 0;
                    for row in 0..png.img.height {
                        for col in 0..png.img.width {
                            let r = image[idx].0;
                            let g = image[idx].1;
                            let b = image[idx].2;
                            let a = image[idx].3;
                            if a == 0xFF {
                                fb.set_pixel(row + y, col + x, (r, g, b));
                            }
                            idx += 1;
                        }
                        fb.update();
                    }
                    
                },
                Err(e) => {
                    println!("Failed to get PNG frame: code {}, message: {}",
                                e.code as usize, e.message);
                }
            }
        },
        Err(e) => {
            println!("Failed to load PNG image: code {}, message: {}",
                        e.code as usize, e.message);
        }
    }
}

fn beeper_test(ms: u32, oct: u8) {
    let gen = WaveformGenerator::new();
    let mut playback = Playback::new();

    let capacity = playback.duration_to_bytes(ms * 7); // 7 notes, ms each
    let mut pcm : Vec<u8> = Vec::with_capacity(capacity);

    // TODO - ENABLE FPU and SIMD
    println!("Sine wave test - Press Enter to start...");
    while stdio_read_byte() != b'\n' {}
    pcm.append(&mut gen.generate(Note::C, oct, ms, Waveform::Sine));
    pcm.append(&mut gen.generate(Note::D, oct, ms, Waveform::Sine));
    pcm.append(&mut gen.generate(Note::E, oct, ms, Waveform::Sine));
    pcm.append(&mut gen.generate(Note::F, oct, ms, Waveform::Sine));
    pcm.append(&mut gen.generate(Note::G, oct, ms, Waveform::Sine));
    pcm.append(&mut gen.generate(Note::A, oct, ms, Waveform::Sine));
    pcm.append(&mut gen.generate(Note::B, oct, ms, Waveform::Sine));
    if let Err(e) = playback.play(pcm.as_slice(), true) {
        println!("Failed to play audio: code {}, message: {}",
                    e.code as usize, e.message);
    }
    pcm.clear();

    println!("Square wave test - Press Enter to start...");
    while stdio_read_byte() != b'\n' {}
    pcm.append(&mut gen.generate(Note::C, oct, ms, Waveform::Square));
    pcm.append(&mut gen.generate(Note::D, oct, ms, Waveform::Square));
    pcm.append(&mut gen.generate(Note::E, oct, ms, Waveform::Square));
    pcm.append(&mut gen.generate(Note::F, oct, ms, Waveform::Square));
    pcm.append(&mut gen.generate(Note::G, oct, ms, Waveform::Square));
    pcm.append(&mut gen.generate(Note::A, oct, ms, Waveform::Square));
    pcm.append(&mut gen.generate(Note::B, oct, ms, Waveform::Square));
    if let Err(e) = playback.play(pcm.as_slice(), true) {
        println!("Failed to play audio: code {}, message: {}",
                    e.code as usize, e.message);
    }
    pcm.clear();

    println!("Triangle wave test - Press Enter to start...");
    while stdio_read_byte() != b'\n' {}
    pcm.append(&mut gen.generate(Note::C, oct, ms, Waveform::Triangle));
    pcm.append(&mut gen.generate(Note::D, oct, ms, Waveform::Triangle));
    pcm.append(&mut gen.generate(Note::E, oct, ms, Waveform::Triangle));
    pcm.append(&mut gen.generate(Note::F, oct, ms, Waveform::Triangle));
    pcm.append(&mut gen.generate(Note::G, oct, ms, Waveform::Triangle));
    pcm.append(&mut gen.generate(Note::A, oct, ms, Waveform::Triangle));
    pcm.append(&mut gen.generate(Note::B, oct, ms, Waveform::Triangle));
    if let Err(e) = playback.play(pcm.as_slice(), true) {
        println!("Failed to play audio: code {}, message: {}",
                    e.code as usize, e.message);
    }
    pcm.clear();

    println!("Sawtooth wave test - Press Enter to start...");
    while stdio_read_byte() != b'\n' {}
    pcm.append(&mut gen.generate(Note::C, oct, ms, Waveform::Sawtooth));
    pcm.append(&mut gen.generate(Note::D, oct, ms, Waveform::Sawtooth));
    pcm.append(&mut gen.generate(Note::E, oct, ms, Waveform::Sawtooth));
    pcm.append(&mut gen.generate(Note::F, oct, ms, Waveform::Sawtooth));
    pcm.append(&mut gen.generate(Note::G, oct, ms, Waveform::Sawtooth));
    pcm.append(&mut gen.generate(Note::A, oct, ms, Waveform::Sawtooth));
    pcm.append(&mut gen.generate(Note::B, oct, ms, Waveform::Sawtooth));
    if let Err(e) = playback.play(pcm.as_slice(), true) {
        println!("Failed to play audio: code {}, message: {}",
                    e.code as usize, e.message);
    }
    
}

fn wav_audio_test(path: &Path) {
    println!("WAV audio test - Press Enter to start...");
    while stdio_read_byte() != b'\n' {}
    match WaveAudio::from_path(path) {
        Ok(wav) => {
            println!("Loaded WAV audio successfully: {} channels, {} bit depth, \
                        byte rate: {}, sample count: {}",
                        wav.channels, wav.bit_depth, wav.byte_rate, wav.sample_count);
            let mut playback = Playback::new();
            if let Err(e) = playback.play(&wav.data, true) {
                println!("Failed to play audio: code {}, message: {}",
                            e.code as usize, e.message);
            }
        },
        Err(e) => {
            println!("Failed to load WAV audio: code {}, message: {}",
                        e.code as usize, e.message);
        }
    }
}

fn wait_for_enter_and_clear() {
    println!("Press Enter to continue...");
    while stdio_read_byte() != b'\n' {}
    stdio_clear_screen();
}

fn txt_dump(path: &Path) {
    let mut buff: [u8; 64] = [0; 64];
    let fort = File::from_path(path, File::MODE_READ);
    let Ok(mut file) = fort else {
        let e = fort.err().unwrap();
        println!("Can't open {} - {:?}", path.as_str(), e);
        return;
    };
    loop {
        let rdrt = file.read(&mut buff);
        let Ok(len) = rdrt else {
            let e = rdrt.err().unwrap();
            println!("Can't read from {} - {:?}", path.as_str(), e);
            return;
        };
        if len > 0 {
            print!("{}", str::from_utf8(&buff[0..len]).unwrap());
        } else {
            break;
        }
    }
    println!("");
}
