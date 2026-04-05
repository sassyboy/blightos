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

    println!("\n[Test 3] Co-operative multitasking test:");
    co_op_sched_test();

    println!("\n[Test 4] Sleep test:");
    sleep_test();

    println!("\n[Test 5] Heap test 1: 2MBs of small allocations");
    heap_test();

    wait_for_enter();

    // Test framebuffer access
    println!("\n[Test 6] Framebuffer Access Test:");
    framebuffer_test();

    // PNG Loading test
    println!("\n[Test 7] PNG Loading Test:");
    png_test();

    wait_for_enter();

    // Audio Playback test
    println!("\n[Test 8] Audio Playback Test:");
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
}

fn heap_test() {
    rtlib::heap::Malloc::init();
    let alloc_size = 64;
    let total_alloc_size = 2 * 1024 * 1024; // 2 MB
    let alloc_count = total_alloc_size / alloc_size;
    println!("Allocating {} blocks of {} bytes each (total: {} bytes). \
                Physical memory before allocations:", 
                alloc_count, alloc_size, total_alloc_size);
    println!("Heap base: {:#x}, Heap size: {} bytes", Malloc::heap_base(),
                                                        Malloc::heap_size());
    txt_dump(&Path::from("machine:/ram"));
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
    txt_dump(&Path::from("machine:/ram"));
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

fn wait_for_enter() {
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
