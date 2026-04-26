use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Set to true by main before spawning threads; read by parser to decide
/// whether to inject graphical registers.
pub static GFX_ENABLED: AtomicBool = AtomicBool::new(false);

use minifb::{Key, Window, WindowOptions};

use crate::value::Value;

// ── Shared state ──────────────────────────────────────────────────────────────

pub struct GfxState {
    /// Pixel buffer in 0x00RRGGBB format, row-major.
    pub pixel_buf: Vec<u32>,
    pub xsz: u32,
    pub ysz: u32,
    /// Window display dimensions (set by wsz/hsz registers).
    pub wsz: u32,
    pub hsz: u32,
    pub needs_open: bool,
    pub needs_refresh: bool,
    pub needs_close: bool,
    pub toggle_fullscreen: bool,
    pub is_open: bool,
}

impl GfxState {
    fn new() -> Self {
        Self {
            pixel_buf: Vec::new(), // allocated on first refresh to avoid 1.92 MB upfront cost
            xsz: 800,
            ysz: 600,
            wsz: 800,
            hsz: 600,
            needs_open: false,
            needs_refresh: false,
            needs_close: false,
            toggle_fullscreen: false,
            is_open: false,
        }
    }
}

static GFX_STATE: OnceLock<Arc<Mutex<GfxState>>> = OnceLock::new();
static GFX_KB: OnceLock<Arc<AtomicI32>> = OnceLock::new();

pub fn state() -> Arc<Mutex<GfxState>> {
    GFX_STATE.get_or_init(|| Arc::new(Mutex::new(GfxState::new()))).clone()
}

pub fn kb() -> Arc<AtomicI32> {
    GFX_KB.get_or_init(|| Arc::new(AtomicI32::new(0))).clone()
}

// ── Color conversion ──────────────────────────────────────────────────────────

pub fn value_to_argb(v: &Value) -> u32 {
    match v.flatten() {
        Value::I(i) => int_to_rgb(i),
        Value::F(f) => {
            let c = (f.abs().min(1.0) * 255.0) as u32;
            (c << 16) | (c << 8) | c
        }
        Value::S(s) => string_to_rgb(&s),
        _ => 0,
    }
}

fn int_to_rgb(i: i32) -> u32 {
    if (100..=999).contains(&i) {
        let r = ((i / 100) as f32 / 9.0 * 255.0) as u32;
        let g = (((i / 10) % 10) as f32 / 9.0 * 255.0) as u32;
        let b = ((i % 10) as f32 / 9.0 * 255.0) as u32;
        (r << 16) | (g << 8) | b
    } else {
        (i as u32) & 0x00FF_FFFF
    }
}

fn string_to_rgb(s: &str) -> u32 {
    let s = s.trim();
    // Try hex: #rrggbb or rrggbb
    let hex = s.trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            return v & 0x00FF_FFFF;
        }
    }
    // Named colours (small common set)
    match s.to_lowercase().as_str() {
        "black" => 0x000000,
        "white" => 0xFFFFFF,
        "red" => 0xFF0000,
        "green" => 0x008000,
        "lime" => 0x00FF00,
        "blue" => 0x0000FF,
        "yellow" => 0xFFFF00,
        "cyan" | "aqua" => 0x00FFFF,
        "magenta" | "fuchsia" => 0xFF00FF,
        "orange" => 0xFFA500,
        "gray" | "grey" => 0x808080,
        "silver" => 0xC0C0C0,
        "maroon" => 0x800000,
        "navy" => 0x000080,
        "purple" => 0x800080,
        "teal" => 0x008080,
        _ => 0,
    }
}

// ── Key mapping (minifb → AWT VK codes) ──────────────────────────────────────

fn key_to_awt(k: Key) -> i32 {
    match k {
        Key::A => 65,  Key::B => 66,  Key::C => 67,  Key::D => 68,
        Key::E => 69,  Key::F => 70,  Key::G => 71,  Key::H => 72,
        Key::I => 73,  Key::J => 74,  Key::K => 75,  Key::L => 76,
        Key::M => 77,  Key::N => 78,  Key::O => 79,  Key::P => 80,
        Key::Q => 81,  Key::R => 82,  Key::S => 83,  Key::T => 84,
        Key::U => 85,  Key::V => 86,  Key::W => 87,  Key::X => 88,
        Key::Y => 89,  Key::Z => 90,
        Key::Key0 => 48, Key::Key1 => 49, Key::Key2 => 50, Key::Key3 => 51,
        Key::Key4 => 52, Key::Key5 => 53, Key::Key6 => 54, Key::Key7 => 55,
        Key::Key8 => 56, Key::Key9 => 57,
        Key::F1 => 112,  Key::F2 => 113,  Key::F3 => 114,  Key::F4 => 115,
        Key::F5 => 116,  Key::F6 => 117,  Key::F7 => 118,  Key::F8 => 119,
        Key::F9 => 120,  Key::F10 => 121, Key::F11 => 122, Key::F12 => 123,
        Key::Up => 38,   Key::Down => 40, Key::Left => 37,  Key::Right => 39,
        Key::Space => 32,   Key::Enter => 10,
        Key::Escape => 27,  Key::Backspace => 8,
        Key::Tab => 9,      Key::Delete => 127,
        Key::Home => 36,    Key::End => 35,
        Key::PageUp => 33,  Key::PageDown => 34,
        Key::Insert => 155,
        Key::LeftShift | Key::RightShift => 16,
        Key::LeftCtrl  | Key::RightCtrl  => 17,
        Key::LeftAlt   | Key::RightAlt   => 18,
        Key::CapsLock => 20,
        Key::NumPad0 => 96,  Key::NumPad1 => 97,  Key::NumPad2 => 98,
        Key::NumPad3 => 99,  Key::NumPad4 => 100, Key::NumPad5 => 101,
        Key::NumPad6 => 102, Key::NumPad7 => 103, Key::NumPad8 => 104,
        Key::NumPad9 => 105,
        _ => 0,
    }
}

// ── Main window loop (must run on the OS main thread on macOS/Windows) ────────

pub fn run_main_loop() {
    let state_arc = state();
    let kb_arc = kb();
    let mut window: Option<Window> = None;

    loop {
        let (needs_open, needs_close, needs_refresh, toggle_fs, wsz, hsz, xsz, ysz) = {
            let s = state_arc.lock().unwrap();
            (s.needs_open, s.needs_close, s.needs_refresh,
             s.toggle_fullscreen, s.wsz, s.hsz, s.xsz, s.ysz)
        };

        if needs_close {
            window = None;
            let mut s = state_arc.lock().unwrap();
            s.is_open = false;
            s.needs_close = false;
        }

        if needs_open && window.is_none() {
            match Window::new(
                "SIO",
                wsz as usize,
                hsz as usize,
                WindowOptions {
                    resize: true,
                    ..WindowOptions::default()
                },
            ) {
                Ok(w) => {
                    window = Some(w);
                    let mut s = state_arc.lock().unwrap();
                    s.is_open = true;
                    s.needs_open = false;
                }
                Err(e) => {
                    eprintln!("gfx: failed to open window: {}", e);
                    let mut s = state_arc.lock().unwrap();
                    s.needs_open = false;
                }
            }
        }

        if let Some(ref mut w) = window {
            if !w.is_open() {
                std::process::exit(0);
            }

            // Live keyboard poll: kb0 reflects what is held *right now*.
            // If no key is held the value is 0, so a SIO program reading
            // `mov kb0 dat` gets 0 between presses and never sees a stale code.
            let held = w.get_keys();
            let code = held.first().map(|&k| key_to_awt(k)).unwrap_or(0);
            kb_arc.store(code, Ordering::SeqCst);

            if toggle_fs {
                // minifb doesn't expose a direct fullscreen toggle on all platforms;
                // we do a best-effort resize to full-screen dimensions.
                // TODO: use WindowOptions::borderless + screen size when needed.
                state_arc.lock().unwrap().toggle_fullscreen = false;
            }

            if needs_refresh {
                let mut s = state_arc.lock().unwrap();
                let expected = (xsz * ysz) as usize;
                if s.pixel_buf.len() != expected {
                    s.pixel_buf.resize(expected, 0);
                }
                let buf: Vec<u32> = s.pixel_buf.clone();
                drop(s);
                if let Err(e) = w.update_with_buffer(&buf, xsz as usize, ysz as usize) {
                    eprintln!("gfx: update_with_buffer: {}", e);
                }
                state_arc.lock().unwrap().needs_refresh = false;
            } else {
                w.update();
            }
        }

        std::thread::sleep(Duration::from_millis(4)); // ~250 fps cap
    }
}
