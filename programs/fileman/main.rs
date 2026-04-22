//
// BlightOS - File Manager
//
#![no_std]
extern crate alloc;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::format;
use alloc::vec::Vec;
use alloc::vec;
use rtlib::task::Spinlock;
use rtlib::*;
use rtlib::env::current_dir;
use rtlib::syscall::SyscallRsvdFDs;
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

static CUR_DIR: Spinlock<String> = Spinlock::new(String::new());
static WMSG:    Spinlock<WindowMessage> = Spinlock::new(WindowMessage::None);
static WMSG_ARG:Spinlock<String> = Spinlock::new(String::new());

#[no_mangle]
fn main() {
    let cur_path = current_dir().expect("Error: current path not available!");
    *CUR_DIR.lock() = String::from(cur_path);
    let Some(screen_sz) = screen_size() else {
        println!("Can't find the screen dimensions!");
        return;
    };

    // Draw the main window
    let wpos = Rect { left: (screen_sz.width - 1024) / 2,
                        top: 50,
                        width: 1024,
                        height: 310 };

    // Label 1
    let lbl1 = Label::new(
        format!("Path: {}", CUR_DIR.lock().clone().as_str()),
        Rect { left: 5, top: 5, width: wpos.width - 10, height: 25 },
        false
    );
    let lbl1_pos = lbl1.get_position().clone();

    
    //
    // ListView for the files and directories
    //
    let lst_files_pos = Rect {
                        left: 5, 
                        top: lbl1_pos.top + lbl1_pos.height + 10,
                        width: 1020 - 10,
                        height: 200
    };
    let mut lst_files = ListView::new(lst_files_pos, true, false);
    lst_files.add_column(String::from("Type"), 100);
    lst_files.add_column(String::from("Name"), 300);
    lst_files.add_column(String::from("Size"), 150);
    lst_files.add_column(String::from("Attributes"), 180);
    
    // Populate the list view with the directory entries
    let entries = enum_directory(&Path::from(CUR_DIR.lock().as_str()));
    for entry in entries {
        lst_files.add_item(entry);
    }
    // Event handlers
    lst_files.register_event(ListViewEvent::OnKeyPress(
        |lv: &mut ListView, kbde: KeyboardEvent| {
            if kbde.released {
                return;
            }
            match kbde.key {
                Key::Enter => {
                    if let Some(selected) = lv.get_selected_item() {
                        let name = &selected[1];
                        let ftype = &selected[0];
                        if ftype == "DIR" || ftype == "MNT" {
                            let entries = go_to_child_directory(name.as_str());
                            lv.clear_items();
                            for entry in entries {
                                lv.add_item(entry);
                            }
                        } else {
                            // Leave a message for the main window to open
                            // the text editor as a modal window
                            let mut current_dir = CUR_DIR.lock().clone();
                            if !current_dir.ends_with("/") {
                                current_dir.push_str("/");
                            }
                            let file_path = format!("{}{}",
                                                current_dir.as_str(), name);
                            *WMSG_ARG.lock() = file_path;
                            *WMSG.lock() = WindowMessage::ShowTextEditor;
                        }
                    }
                },
                Key::Backspace => {
                    let entries = go_to_parent_directory();
                    lv.clear_items();
                    for entry in entries {
                        lv.add_item(entry);
                    }
                },
                Key::F4 => {
                    exit(0);
                },
                _ => { }
            }
        }
    ));
    //
    // Status label
    //
    let lbl_status = Label::new(
        String::from("Use arrow keys to navigate, Enter to open, Backspace to go up, F4 to exit"),
        Rect { left: lst_files_pos.left,
                top: lst_files_pos.top + lst_files_pos.height + 5,
                width: lst_files_pos.width,
                height: lbl1_pos.height
        },
        false
    );
    //
    // Exit Button
    //
    let mut btn_exit = Button::new(
        String::from("Exit (F4)"),
        Rect {
            left: wpos.width - 150 - 10,
            top: lst_files_pos.top + lst_files_pos.height + 5,
            width: 150,
            height: 30 
        },
        false
    );
    btn_exit.register_event(ButtonEvent::OnClick(
        |btn: &mut Button| {
            if btn.get_text() == "Exit (F4)" {
                btn.set_text(String::from("Really?"));
            } else if btn.get_text() == "Really?" {
                // Need a way to post messages for the parent window!
                exit(0);
            }
        }
    ));
    let mut win_main = Window::new();
    let _ = win_main.init(String::from("File Manager"), wpos);
    win_main.add_widget(Box::new(lbl1));         // 0
    win_main.add_widget(Box::new(lst_files));    // 1
    win_main.add_widget(Box::new(lbl_status));   // 2
    win_main.add_widget(Box::new(btn_exit));     // 3
    win_main.register_event(WindowEvent::OnKeyPress(
        |win, kbde| {
            if kbde.key == Key::F4 && kbde.released {
                win.close();
            }
        }
    ));
    
    win_main.show(main_window_loop);
    println!("Main window exited!");
}

fn main_window_loop(win: &mut Window) -> bool {
    if win.process_event() {
        let lbl1 = win.borrow_widget_mut::<Label>(0).unwrap();
        lbl1.set_text(format!("Path: {}", CUR_DIR.lock().clone().as_str()));
        win.render();
    }
    // Any messages from event handlers?
    let mut wmsg = WMSG.lock();
    match *wmsg {
        WindowMessage::ShowTextEditor => {
            run_text_editor(win, &Path::from( WMSG_ARG.lock().as_str()));
            *wmsg = WindowMessage::None;
        },
        WindowMessage::None => {}
    }     
    
    true // Continue
}

fn go_to_child_directory(dir_name: &str) -> Vec<Vec<String>> {
    let mut cur_path = CUR_DIR.lock();
    if cur_path.is_empty() {
        // This is a mount point. Go to the root of the mount point
        let new_path = format!("{}:/", dir_name);
        cur_path.clear();
        cur_path.push_str(new_path.as_str());
    } else {
        if cur_path.ends_with("/") {
            cur_path.pop();
        }
        let new_path = format!("{}/{}/", cur_path.as_str(), dir_name.trim());
        cur_path.clear();
        cur_path.push_str(new_path.as_str());
    }
    enum_directory(&Path::from(cur_path.as_str()))
}

fn go_to_parent_directory() -> Vec<Vec<String>> {
    let mut cur_path = CUR_DIR.lock();
    if cur_path.ends_with("/") {
        cur_path.pop();
    }
    if cur_path.ends_with(":") || cur_path.is_empty() {
        // This is a mount-point. Can't go back up. Just List the mount points
        cur_path.clear();
        return enum_mount_points();
    }
    if let Some(pos) = cur_path.rfind("/") {
        if pos == 0 {
            cur_path.clear();
            cur_path.push_str("/");
        } else {
            cur_path.truncate(pos);
        }
    }
    enum_directory(&Path::from(cur_path.as_str()))
}

fn enum_mount_points() -> Vec<Vec<String>> {
    let mut entries = Vec::new();
    let mut buff: [u8; 4096] = [0; 4096];
    let Ok(len) = fread(SyscallRsvdFDs::SystemResources as usize,
                                                        0, &mut buff) else {
        panic!("Failed to read system resources");
    };
    let entries_str = str::from_utf8(&buff[0..len]).unwrap().split("\n");
    for mnt in entries_str {
        if mnt.trim().is_empty() {
            continue;
        }
        entries.push(vec![
            String::from("MNT"),
            String::from(mnt),
            String::from("-"),
            String::from("System Resource"),
        ]);
    }
    entries
}

fn enum_directory(path: &Path) -> Vec<Vec<String>> {
    let mut entries = Vec::new();
    let mut buff: [u8; 4096] = [0; 4096];
    // Open the directory
    let Ok(dir) = File::from_path(path, File::MODE_READ) else {
        println!("Path {} doesn't exist", path.as_str());
        return entries;
    };
    // Read the directory entries
    // Format: ([Name],[Size in hex],[flags in hex]\n)*
    let Ok(len) = dir.enum_dir(&mut buff) else {
        println!("Failed to read directory {}", path.as_str());
        return entries;
    };
    let entries_str = str::from_utf8(&buff[0..len]).unwrap().split("\n");
    for entry in entries_str {
        let fields: Vec<&str> = entry.split(",").collect();
        if fields.len() != 3 {
            continue;
        }
        let fname = String::from(fields[0]);
        if fname.trim().eq(".") || fname.trim().eq("..") {
            continue;
        }
        let flags = usize::from_str_radix(fields[2], 16).unwrap();
        let ftype = String::from(
            if flags & File::FLG_DIRECTORY != 0 {
                "DIR"
            } else {
                "FILE"
            }
        );
        let fsize = usize::from_str_radix(fields[1], 16).unwrap();
        let fsize_str = if flags & File::FLG_DIRECTORY != 0 {
            String::from("-")
        } else {
            if fsize >= 1024 * 1024 {
                format!("{:.1} MB", fsize as f64 / (1024.0 * 1024.0))
            } else if fsize >= 1024 {
                format!("{:.1} KB", fsize as f64 / 1024.0)
            } else {
                format!("{} B", fsize)
            }
        };
        let mut attr_str = String::new();
        if flags & File::FLG_SYSTEM != 0 {
            attr_str += "SYS ";
        }
        if flags & File::FLG_DEVICE != 0 {
            attr_str += "DEV ";
        }
        if flags & File::FLG_HIDDEN != 0 {
            attr_str += "H ";
        }
        if flags & File::FLG_ARCHIVE != 0 {
            attr_str += "A ";
        }
        if flags & File::FLG_PERM_READ != 0 {
            attr_str += "R ";
        }
        if flags & File::FLG_PERM_WRITE != 0 {
            attr_str += "W ";
        }
        if flags & File::FLG_PERM_EXEC != 0 {
            attr_str += "X ";
        }
        entries.push(vec![ftype, fname, fsize_str, attr_str]);
    }
    entries
}

fn run_text_editor(parent_win: &mut Window, path: &Path) {
    let cur_path = path.as_str();
 // Draw the main window
    let theme = Theme::new();
    let wpos = Rect { left: 150, top: 280, width: 1025, height: 600 };
    let mut win_edit = Window::new();
    let _ = win_edit.init(String::from("Text Editor"), wpos);
    // Label 1
    let mut lbl1 = Label::new(
        format!("{} (Press ESC to close)", cur_path),
        Rect { left: 5, top: 5, width: 0, height: 0 },
        false
    );
    lbl1.fit_to_text(&theme);
    let lbl1_pos = lbl1.get_position().clone();
    
    // Text editor
    let mut txt1 = TextEdit::new(
        Rect { left: 5, 
                top: lbl1_pos.top + lbl1_pos.height + 5,
                width: 1000,
                height: 500
        }, true, false
    );
    // Load the file content into the text editor
    let content = read_file(path);
    txt1.set_text(content);
    let txt1_pos = txt1.get_position().clone();

    // Status label
    let lbl_status = Label::new(
        String::from("Line: 1, Col: 1"),
        Rect { left: txt1_pos.left,
                top: txt1_pos.top + txt1_pos.height + 5,
                width: txt1_pos.width,
                height: lbl1_pos.height
        },
        false
    );
    
    win_edit.add_widget(Box::new(lbl1));         // 0
    win_edit.add_widget(Box::new(txt1));         // 1
    win_edit.add_widget(Box::new(lbl_status));   // 2
    win_edit.register_event(WindowEvent::OnKeyPress(
        |win, kbde| {
            if kbde.key == Key::Escape && kbde.released {
                win.close();
            }
        }
    ));
    parent_win.show_modal_window(&mut win_edit, text_editor_event_loop);
}

fn text_editor_event_loop(win: &mut Window) -> bool {
    if win.process_event() {
        let txt1 = win.borrow_widget_ref::<TextEdit>(1).unwrap();
        let (row, col) = txt1.get_cursor_position();

        let lbl_status = win.borrow_widget_mut::<Label>(2).unwrap();
        lbl_status.set_text(format!("Line: {}, Col: {}", row + 1, col + 1));
        win.render();
    }
    true
}

fn read_file(path: &Path) -> String {
    let mut buff: [u8; 4096] = [0; 4096];
    let fort = File::from_path(path, File::MODE_READ);
    let Ok(mut file) = fort else {
        let err = fort.err().unwrap();
        println!("Can't open |{}| - {:?}", path.as_str(), err);
        return String::new();
    };
    let mut content = String::new();
    loop {
        let rdrt = file.read(&mut buff);
        let Ok(len) = rdrt else {
            let err = rdrt.err().unwrap();
            println!("Failed to read file {} - {:?}", path.as_str(), err);
            return String::new();
        };
        if len > 0 {
            content.push_str(str::from_utf8(&buff[0..len]).unwrap());
        } else {
            break;
        }
    }
    content
}

