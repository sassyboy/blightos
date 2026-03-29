//
// BlightOS - A Tetris game to test the graphics and audio capabilities
//
#![no_std]

use rtlib::*;
use rtlib::stdio::*;
use rtlib::task::*;
use rtlib::fileio::*;
use rtlib::graphics::RGB;
use rtlib::graphics::framebuffer::*;
use rtlib::audio::Playback;
use rtlib::audio::wav::WaveAudio;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::AtomicU8;
use core::sync::atomic::Ordering;
use core::time::Duration;
use core::arch::asm;

const ARENA_WIDTH:  u32 = 15;
const ARENA_HEIGHT: u32 = 30;
const BLOCK_SIZE:   u32 = 15; // Size of each Tetris block in pixels

const REFRESH_DURATION: Duration = Duration::from_millis(10);

const TEROMINO_TYPES: usize = 8; // Number of different tetromino shapes
const TETROMINO_SHAPES: [[[u8; 4]; 4]; TEROMINO_TYPES] = [
    // I
    [[0, 0, 0, 0],
     [1, 1, 1, 1],
     [0, 0, 0, 0],
     [0, 0, 0, 0]],
    // o
    [[1, 1, 0, 0],
     [1, 1, 0, 0],
     [0, 0, 0, 0],
     [0, 0, 0, 0]],
    // O
    [[1, 1, 1, 0],
     [1, 1, 1, 0],
     [1, 1, 1, 0],
     [0, 0, 0, 0]],
    // T
    [[0, 1, 0, 0],
     [1, 1, 1, 0],
     [0, 0, 0, 0],
     [0, 0, 0, 0]],
    // S
    [[0, 1, 1, 0],
     [1, 1, 0, 0],
     [0, 0, 0, 0],
     [0, 0, 0, 0]],
    // Z
    [[1, 1, 0, 0],
     [0, 1, 1, 0],
     [0, 0, 0, 0],
     [0, 0, 0, 0]],
    // J
    [[1, 1, 1, 0],
     [1, 0, 0, 0],
     [0, 0, 0 ,0],
     [0 ,0 ,0 ,0]],
    // L
    [[1 ,1 ,1 ,0],
     [0 ,0 ,1 ,0],
     [0 ,0 ,0 ,0],
     [0 ,0 ,0 ,0]]
];

const COLOR_BACKGROUND: RGB = (0, 0, 0);
const COLORS: [RGB; TEROMINO_TYPES + 1] = [
    (0,   0,    0  ),   // Black (Background)
    (255, 0,    0  ),   // Red
    (0,   255,  0  ),   // Green
    (0,   0,    255),   // Blue
    (255, 255,  0  ),   // Yellow
    (255, 0,    255),   // Magenta
    (0,   255,  255),   // Cyan
    (255, 165,  0  ),   // Orange
    (139, 69,   19 )    // Brown
];

static INPUT: AtomicU8 = AtomicU8::new(0);
static VIDEO_LOOP: AtomicBool = AtomicBool::new(false);

#[no_mangle]
fn main() {
    let fb0 = Framebuffer::new();
    if let Some(mut fb) = fb0 {
        // Save the framebuffer content before drawing
        fb.save_frame();
        // Initialize the game state
        // Launch a task to run the Tetris game logic and rendering
        VIDEO_LOOP.store(true, Ordering::Relaxed);
        let rtask = Task::spawn(render_loop, 0, "RenderLoop");
        // Main loop to update the framebuffer with the latest game state
        loop {
            // Handle user input (e.g., keyboard events) to control the game
            let key =  stdio_read_byte();
            if key == 'q' as u8 {
                break; // Exit the game loop on 'q' key press
            } else if key == b'a' || key == b's' || key == b'd' || key == b'w'
                    || key == b'e' || key == b'\n' || key == b' '
                    || key == b'A' || key == b'S' || key == b'D' || key == b'W'
                    || key == b'E' {
                INPUT.store(key, Ordering::Relaxed);
            }
        }
        // Restore the original framebuffer content before exiting
        VIDEO_LOOP.store(false, Ordering::Relaxed);
        Task::join(rtask.unwrap().tid);
        fb.restore_frame();
    } else {
        println!("Failed to access the framebuffer.");
        exit(1);
    }
    exit(0);
}

struct GameState {
    // Current falling piece state
    cur_shape:  usize,
    cur_rot:    usize,
    cur_x:      i32,
    cur_y:      i32,
    has_piece:  bool,
    move_down:  bool,
    redraw:    bool,
}
impl GameState {
    fn new() -> Self {
        GameState {
            cur_shape: 0,
            cur_rot: 0,
            cur_x: 0,
            cur_y: 0,
            has_piece: false,
            move_down: false,
            redraw: true,
        }
    }
}

enum StepResult {
    Continue,
    BlockPlaced,
    RowCleared,
    GameOver,
}

// Returns false if the game should end, true to continue
fn game_logic(arena: &mut [[u8; ARENA_WIDTH as usize]; ARENA_HEIGHT as usize],
                                        state: &mut GameState) -> StepResult {
    let mut outcome = StepResult::Continue;
    // spawn piece if needed
    if !state.has_piece {
        state.cur_shape = rand() % TEROMINO_TYPES;
        state.cur_rot = 0;
        state.cur_x = (ARENA_WIDTH as i32 / 2) - 2;
        state.cur_y = 0;
        // immediate spawn collision -> game over
        if collides(arena, state.cur_shape, state.cur_rot, state.cur_x, state.cur_y) {
            // end game
            VIDEO_LOOP.store(false, Ordering::Relaxed);
            return StepResult::GameOver;
        }
        state.has_piece = true;
    }

    // handle input
    let ev = INPUT.load(Ordering::Relaxed);
    if ev != 0 {
        // clear input after reading
        INPUT.store(0, Ordering::Relaxed);
        match ev {
            b'a' => {
                // move left
                if !collides(arena, state.cur_shape, state.cur_rot,
                                                state.cur_x - 1, state.cur_y) {
                    state.cur_x -= 1;
                    state.redraw = true;
                }
            },
            b'd' => {
                // move right
                if !collides(arena, state.cur_shape, state.cur_rot,
                                                state.cur_x + 1, state.cur_y) {
                    state.cur_x += 1;
                    state.redraw = true;
                }
            },
            b'e' => {
                // rotate anti-clockwise (user requested)
                let new_rot = (state.cur_rot + 3) & 3;
                if !collides(arena, state.cur_shape, new_rot, state.cur_x, state.cur_y) {
                    state.cur_rot = new_rot;
                    state.redraw = true;
                }
            },
            b'w' => {
                // rotate clockwise
                let new_rot = (state.cur_rot + 1) & 3;
                if !collides(arena, state.cur_shape, new_rot, state.cur_x, state.cur_y) {
                    state.cur_rot = new_rot;
                    state.redraw = true;
                }
            },
            b' '|b'\n'|b's' => {
                // soft drop (user requested)
                state.move_down = true;
            },
            _ => {}
        }
    }

    // attempt to move down
    if state.move_down {
        if collides(arena, state.cur_shape, state.cur_rot, state.cur_x, state.cur_y + 1) {
            // can't move down -> place piece
            place_piece(arena, state.cur_shape, state.cur_rot, state.cur_x, state.cur_y);
            // clear full rows
            if clear_full_rows(arena) {
                outcome = StepResult::RowCleared;
            } else {
                outcome = StepResult::BlockPlaced;
            }
            state.has_piece = false;
        } else {
            // can move down
            state.cur_y += 1;
        }
        state.redraw = true;
    }
    outcome
}

// Returns whether the cell at (r, c) in the piece's local 4x4 grid is occupied
// for the given shape and rotation
fn shape_cell(shape: usize, rot: usize, r: usize, c: usize) -> u8 {
    let base = &TETROMINO_SHAPES[shape];
    if shape == 1 || shape == 2 {
        // O pieces doesn't rotate
        return base[r][c];
    }
    match rot & 3 {
        0 => base[r][c],
        1 => base[3 - c][r],            // 90° CW
        2 => base[3 - r][3 - c],        // 180°
        3 => base[c][3 - r],            // 270° CW
        _ => 0,
    }
}

// Test collision of piece at (nx, ny) with rotation nrot
fn collides(arena: &[[u8; ARENA_WIDTH as usize]; ARENA_HEIGHT as usize],
                shape: usize, nrot: usize, nx: i32, ny: i32) -> bool {
    for pr in 0..4 {
        for pc in 0..4 {
            if shape_cell(shape, nrot, pr, pc) == 1 {
                let ax = nx + pc as i32;
                let ay = ny + pr as i32;
                if ax < 0 || ay < 0 ||
                    ax >= ARENA_WIDTH as i32 || ay >= ARENA_HEIGHT as i32 {
                    return true; // out of bounds -> collision
                }
                if arena[ay as usize][ax as usize] != 0 {
                    return true; // hits existing block
                }
            }
        }
    }
    false
}

// Place piece into arena (make blocks permanent)
fn place_piece(arena: &mut [[u8; ARENA_WIDTH as usize]; ARENA_HEIGHT as usize],
                                shape: usize, rot: usize, nx: i32, ny: i32) {
    for pr in 0..4 {
        for pc in 0..4 {
            if shape_cell(shape, rot, pr, pc) == 1 {
                let ax = (nx + pc as i32) as usize;
                let ay = (ny + pr as i32) as usize;
                if ay < ARENA_HEIGHT as usize && ax < ARENA_WIDTH as usize {
                    arena[ay][ax] = (shape + 1) as u8;
                }
            }
        }
    }
}

// Clear full rows and shift above rows down
// Returns true if any rows were cleared
fn clear_full_rows(arena: &mut [[u8; ARENA_WIDTH as usize]; ARENA_HEIGHT as usize])
                                                                    -> bool {
    let mut cleared = false;
    let mut write_row = (ARENA_HEIGHT as usize).wrapping_sub(1);
    for read_row in (0..ARENA_HEIGHT as usize).rev() {
        let full = arena[read_row].iter().all(|&b| b != 0);
        if !full {
            // move this row to write_row
            if write_row != read_row {
                arena[write_row] = arena[read_row];
            }
            if write_row > 0 { write_row -= 1; } else { /* at top */ }
        } else {
            cleared = true;
        }
    }
    // fill remaining top rows with zeros
    for r in 0..=write_row {
        arena[r] = [0u8; ARENA_WIDTH as usize];
    }
    cleared
}

fn render_loop(_args: usize){
    // Load the WAV audio files
    let Ok(snd_col) = WaveAudio::from_path(&Path::from("res/sfx/click.wav"))
    else {
        println!("Failed to load res/sfx/click.wav");
        return;
    };
    let Ok(snd_clear) = WaveAudio::from_path(&Path::from("res/sfx/boom.wav"))
    else {
        println!("Failed to load res/sfx/boom.wav");
        return;
    };
    let Ok(snd_gover) = WaveAudio::from_path(&Path::from("res/sfx/gover.wav"))
    else {
        println!("Failed to load res/sfx/gover.wav");
        return;
    };

    let Some(mut fb) = Framebuffer::new() else {
        println!("Failed to access the framebuffer.");
        return;
    };
    // Calculate the top-left corner of the arena to center it on the screen
    let x0 = (fb.width - ARENA_WIDTH * BLOCK_SIZE) / 2;
    let y0 = (fb.height - ARENA_HEIGHT * BLOCK_SIZE) / 2;
    draw_arena(&mut fb, y0, x0);
        
    let mut arena = [[0; ARENA_WIDTH as usize]; ARENA_HEIGHT as usize];
    let mut state = GameState::new();
    let mut game_speed = 1; // up to 10
    let mut fall_tick = 0;
    let mut game_tick = 0;

    while VIDEO_LOOP.load(Ordering::Relaxed) {
        Task::sleep(REFRESH_DURATION);
        fall_tick += 1;
        game_tick += 1;
        if fall_tick >= 200 / game_speed {
            state.move_down = true;
            fall_tick = 0;
        } else {
            state.move_down = false;
        }
        if game_tick % 1000 == 0 && game_speed < 10 {
            game_speed += 1; // increase speed every 1000 ticks (i.e., 10s)
        }
        
        // Run the game logic
        match game_logic(&mut arena, &mut state) {
            StepResult::GameOver => {
                play_sfx(&snd_gover, true);
                println!("Game Over!");
                break;
            },
            StepResult::RowCleared => {
                play_sfx(&snd_clear, false);
            },
            StepResult::BlockPlaced => {
                play_sfx(&snd_col, false);
            },
            StepResult::Continue => {}
        }
        if state.redraw {
            // Draw the current state of the arena
            for row in 0..ARENA_HEIGHT {
                for col in 0..ARENA_WIDTH {
                    let block_type = arena[row as usize][col as usize];
                    if block_type != 0 {
                        draw_block(&mut fb, y0, x0, row, col, 
                                            COLORS[block_type as usize], true);
                    } else {
                        draw_block(&mut fb, y0, x0, row, col,
                                            COLOR_BACKGROUND, false);
                    }
                }
            }
            // Put the piece into the SHAPE buffer for the renderer to read
            draw_tetromino(&mut fb, y0, x0, state.cur_shape, state.cur_rot,
                        state.cur_y as u32, state.cur_x as u32,
                        COLORS[state.cur_shape + 1]);
            fb.update();
            state.redraw = false;
        }
    }
}

fn draw_arena(fb: &mut Framebuffer, y0: u32, x0: u32) {
    // Draw the Tetris arena (borders, grid, etc.) on the framebuffer
    for row in 0..=ARENA_HEIGHT+1 {
        for col in 0..=ARENA_WIDTH+1 {
            let fill: RGB;
            let thd: bool;
            if row == 0 || row == ARENA_HEIGHT+1 || 
                col == 0 || col == ARENA_WIDTH+1 {
                // Gray for borders
                fill = (128, 128, 128);
                thd = true;
            } else {
                // Black for empty space
                fill = (0, 0, 0);
                thd = false;
            };
            draw_block(fb, y0-BLOCK_SIZE, x0-BLOCK_SIZE, row, col, fill, thd);
        }
    }
}

fn draw_tetromino(fb: &mut Framebuffer, y0: u32, x0: u32, shape: usize, rot: usize, y: u32, x: u32, color: RGB) {
    for r in 0..4 as usize {
        for c in 0..4 as usize {
            let row = r as u32 + y as u32;
            let col = c as u32 + x as u32;
            if shape_cell(shape, rot, r, c) == 1 {
                draw_block(fb, y0, x0, row, col, color, true);
            }    
        }
    }       
}

// Draw a single Tetris block at the specified row and column with the given color
// y0, x0: top-left corner of the arena in pixels
// y, x: block coordinates within the arena in blocks
fn draw_block(fb: &mut Framebuffer, y0: u32, x0: u32, y: u32, x: u32, fill: RGB,
                                                        three_dim: bool) {
    let pixel_x = x0 + x * BLOCK_SIZE;
    let pixel_y = y0 + y * BLOCK_SIZE;
    for row in 0..BLOCK_SIZE {
        for col in 0..BLOCK_SIZE {
            if three_dim {
                // Add a simple shading effect for 3D blocks
                let shade = if row < BLOCK_SIZE / 2 && col < BLOCK_SIZE / 2 {
                    (fill.0.saturating_add(50),
                     fill.1.saturating_add(50),
                     fill.2.saturating_add(50)) // Lighter shade
                } else if row >= BLOCK_SIZE / 2 && col >= BLOCK_SIZE / 2 {
                    (fill.0.saturating_sub(50),
                     fill.1.saturating_sub(50),
                     fill.2.saturating_sub(50)) // Darker shade
                } else {
                    fill // Original color
                };
                fb.set_pixel(pixel_y + row, pixel_x + col, shade);
            } else {
                if row == 0 || row == BLOCK_SIZE - 1 || col == 0 || col == BLOCK_SIZE - 1 {
                    // Draw a border around the block
                    fb.set_pixel(pixel_y + row, pixel_x + col, 
                        (fill.0.saturating_add(50),
                         fill.1.saturating_add(50),
                         fill.2.saturating_add(50))); // Lighter shade
                } else {
                    fb.set_pixel(pixel_y + row, pixel_x + col, fill);
                }
            }
        }
    }
}

fn play_sfx(snd: &WaveAudio, sync: bool) {
    let mut playback = Playback::new();
    let _ = playback.play(&snd.data, sync);
}

fn read_timestamp() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        let (upper, lower): (u64, u64);
        unsafe {
            asm!("rdtsc", out("rdx")upper, out("rax")lower);
        }
        (upper << 32) | lower
    }
    #[cfg(target_arch = "aarch64")]
    {
        let cntvct: u64;
        unsafe {
            asm!("mrs {}, cntvct_el0", out(reg) cntvct);
        }
        cntvct
    }
}

fn rand() -> usize {
    // A simple pseudo-random number generator (e.g., linear congruential generator)
    let mut seed: usize = read_timestamp() as usize;
    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
    seed.wrapping_div(65536) % 32768
}