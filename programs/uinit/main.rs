//
// BlightOS - Initial user-space program
// 
// Sets up a desktop environment
//
#![no_std]
extern crate alloc;
use core::sync::atomic::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::format;
use rtlib::task::*;
use rtlib::*;
use rtlib::stdio::*;
use rtlib::fileio::*;
use rtlib::graphics::*;
use rtlib::gui::*;
use rtlib::hid::*;

#[derive(Clone, Copy)]
pub enum WindowMessage {
    None,
    ShowTextEditor
}

static PROGRAM_LAUNCHED: AtomicBool = AtomicBool::new(false);
static PROGRAM_PATH: Spinlock<String> = Spinlock::new(String::new());
static MOUSE: Spinlock<Mouse> = Spinlock::new(Mouse::new());

#[no_mangle]
fn main() {
    let Some(screen) = Framebuffer::get_framebuffer_info() else {
        println!("Error fetching the screen information!");
        return;
    };
    let wpos = Rect { left: 0,top: 0, width: screen.width,
                    height: screen.height };

    // Wall-paper widget
    let mut img_wallpaper = ImageBox::new(wpos.clone());
    img_wallpaper.load_from_file(&Path::from("res/wpaper1.png"));

    // Ram usage label
    let mut lbl_ram = Label::new(String::from("Memory Usage:"),
                        Rect{left: 5, top: screen.height - 25,
                             height: 25, width:200}, true);
    lbl_ram.set_text(get_memory_usage());
    lbl_ram.fit_to_text(&Theme::new());
    let mut lbl_ram_bg = Theme::new().accent;
    lbl_ram_bg.3 = 200;
    lbl_ram.set_bg_color(Some(lbl_ram_bg));

    // File Manager button
    let mut btn_fileman = Button::new(String::from("Machine"),
                        Rect{left: 10, top: 10, width: 85, height: 85}, false);
    btn_fileman.set_image_from_file(&Path::from("res/fileman.png"));
    btn_fileman.set_transparent_bg(true);
    btn_fileman.set_text_align(HorizontalAlignment::Center, 
                                VerticalAlignment::Bottom);
    btn_fileman.register_event(ButtonEvent::OnClick(on_btn_fileman));
    
    // Shell button
    let mut btn_shell   = Button::new(String::from("Shell"),
                        Rect{left: 10, top: 100, width: 85, height: 85}, false);
    btn_shell.set_image_from_file(&Path::from("res/shell.png"));
    btn_shell.set_transparent_bg(true);
    btn_shell.set_text_align(HorizontalAlignment::Center,
                                VerticalAlignment::Bottom);
    btn_shell.register_event(ButtonEvent::OnClick(on_btn_shell));

    // Tetris button
    let mut btn_tetris  = Button::new(String::from("Tetris"),
                        Rect{left: 10, top: 200, width: 85, height: 85}, false);
    btn_tetris.set_image_from_file(&Path::from("res/tetris.png"));
    btn_tetris.set_transparent_bg(true);
    btn_tetris.set_text_align(HorizontalAlignment::Center,
                                VerticalAlignment::Bottom);
    btn_tetris.register_event(ButtonEvent::OnClick(on_btn_tetris));

    {
        let mut mouse = MOUSE.lock();
        let _ = mouse.start_listening();
    }

    // The desktop window!
    let mut win_desktop = Window::new();
    let _ = win_desktop.init(String::from("BlightOS Desktop"), wpos);
    win_desktop.set_borderless(true);
    win_desktop.add_widget(Box::new(img_wallpaper));    // 0
    win_desktop.add_widget(Box::new(lbl_ram));          // 1
    win_desktop.add_widget(Box::new(btn_fileman));      // 2
    win_desktop.add_widget(Box::new(btn_shell));        // 3
    win_desktop.add_widget(Box::new(btn_tetris));       // 4
    win_desktop.register_event(WindowEvent::OnKeyPress(
        |win, kbde| {
            if kbde.key == Key::F4 && kbde.released && kbde.alt_pressed() {
                win.close();
            }
        }
    ));
    win_desktop.show(main_window_loop);
    println!("Main window exited!");
}

fn main_window_loop(win: &mut Window) -> bool {

    // Fetch mouse events
    // let mevents;
    // {
    //     let mut mouse = MOUSE.lock();
    //     mevents = mouse.fetch_events();
    // }
    if let Some(lbl_ram) = win.borrow_widget_mut::<Label>(1) {
        let txt = get_memory_usage();
    //     if let Some(me) = mevents.last(){
    //         txt += format!("  mouse: X:{}, Y:{} {}{}{}",
    //             me.x, me.y,
    //             if me.left_btn {"LB"} else {""},
    //             if me.middle_btn {"MB"} else {""},
    //             if me.right_btn {"RB"} else {""}
    //         ).as_str();
    //     } else {
    //         txt += "  No mouse";
    //     }
        lbl_ram.set_text(txt);
        lbl_ram.fit_to_text(&Theme::new());
    }
    
    if PROGRAM_LAUNCHED.load(Ordering::Relaxed) {
        // Launch a program!
        win.set_active(false);
        run_exec();
        win.flush_events();
        win.set_active(true);
        win.render();
    }
    if win.process_event() {
        win.render();
    }
    true // The window must live!!
}

fn on_btn_fileman(_btn: &mut Button) {
    queue_exec_run(Path::from("fileman.box").as_str(), "");
}

fn on_btn_shell(_btn: &mut Button){
    queue_exec_run(Path::from("shell.box").as_str(), "cls");
}

fn on_btn_tetris(_btn: &mut Button){
    queue_exec_run(Path::from("tetris.box").as_str(), "");
}

fn queue_exec_run(path: &str, args: &str) {
    let cmd_line;
    if args.is_empty() {
        cmd_line= String::from(path);
    } else {
        cmd_line = format!("{} {}", path, args);
    }
    PROGRAM_LAUNCHED.store(true, Ordering::Relaxed);
    *PROGRAM_PATH.lock() = cmd_line;
}

fn run_exec(){
    let cmd = PROGRAM_PATH.lock().clone();
    match Process::spawn(&cmd) {
        Ok(proc) => {
            proc.join();
        },
        Err(e) => {
            println!("Failed to execute {} due to {:?}", cmd, e);
        }
    }
    PROGRAM_LAUNCHED.store(false, Ordering::Relaxed);
}

fn get_memory_usage() -> String {
    let Ok(mut fram) = File::from_path(&Path::from("machine:/ram"),
                                            File::MODE_READ) else {
        return String::from("No RAM info");
    };
    let mut buf = [0u8; 256];
    let Ok(len) = fram.read(&mut buf) else {
        return String::from("No RAM info");
    };
    let mut total_frames = 0;
    let mut free_frames = 0;
    for line in str::from_utf8(&buf[0..len]).expect("No RAM info").split("\n") {
        let last_digit = line.find(" Frames").expect("bug");
        if line.starts_with("Total: ") {
            total_frames = usize::from_str_radix(&line[7..last_digit], 10)
                                                            .expect("bug");
        } else if line.starts_with("Free : "){
            free_frames = usize::from_str_radix(&line[7..last_digit], 10)
                                                            .expect("bug");
        }
    }
    format!(" {:.2}% Free ({:.3} MB) ", 
        free_frames as f32 / total_frames as f32 * 100.0,
        free_frames as f32 / 256.0
    )
}
