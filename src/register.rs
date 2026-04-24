use std::cell::RefCell;
use std::collections::VecDeque;
use std::env;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, Write};
use std::rc::Rc;

use crate::channel::PinChannel;
use crate::value::Value;

// ── File-system types ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum FileMode {
    Str,   // UTF-8 text: reads/writes one line at a time
    Int,   // raw little-endian i32 (4 bytes)
    Float, // raw little-endian f32 (4 bytes)
}

pub struct FileReadState {
    pub mode: FileMode,
    pub file: Option<BufReader<std::fs::File>>,
    pub byte_pos: u64,
}

pub struct FileWriteState {
    pub mode: FileMode,
    pub file: Option<BufWriter<std::fs::File>>,
    pub byte_pos: u64,
}

impl FileReadState {
    pub fn new() -> Self {
        Self {
            mode: FileMode::Str,
            file: None,
            byte_pos: 0,
        }
    }
}

impl FileWriteState {
    pub fn new() -> Self {
        Self {
            mode: FileMode::Str,
            file: None,
            byte_pos: 0,
        }
    }
}

pub enum Register {
    Null,
    Plain(Value),
    Clock {
        speed: i32,
        active: bool,
    },
    Pin(PinChannel),
    Offset(i32),
    SizedMemory {
        mem: Vec<Value>,
        offset: Rc<RefCell<Register>>,
    },
    UnsizedMemory {
        mem: Vec<Value>,
        offset: Rc<RefCell<Register>>,
    },
    Stdout(TapeState),
    Stderr(TapeState),
    Stdin(StdinState),
    Rng(RngState),
    FileReadControl(Rc<RefCell<FileReadState>>),
    FileWriteControl(Rc<RefCell<FileWriteState>>),
    FileReadTape(Rc<RefCell<FileReadState>>),
    FileWriteTape {
        state: Rc<RefCell<FileWriteState>>,
        tape: VecDeque<String>,
    },
    #[cfg(feature = "gfx")]
    Gfx {
        pixel_mem: Rc<RefCell<Register>>,
        pixel_offset: Rc<RefCell<Register>>,
        xsz: Rc<RefCell<Register>>,
        ysz: Rc<RefCell<Register>>,
        wsz: Rc<RefCell<Register>>,
        hsz: Rc<RefCell<Register>>,
        state: std::sync::Arc<std::sync::Mutex<crate::gfx::GfxState>>,
    },
}

pub struct TapeState {
    pub tape: VecDeque<String>,
    pub is_err: bool,
}

pub struct StdinState {
    reader: BufReader<io::Stdin>,
    buf: String,
    closed: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RngKind {
    Int,
    Float,
    String,
}

pub struct RngState {
    state: u64,
    pub kind: RngKind,
    pub seed: i32,
    initialized: bool,
}

impl RngState {
    pub fn new() -> Self {
        Self {
            state: 0,
            kind: RngKind::Int,
            seed: 0,
            initialized: false,
        }
    }

    fn ensure_init(&mut self) {
        if !self.initialized {
            self.seed_time_based();
        }
    }

    fn seed_time_based(&mut self) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b97f4a7c15);
        self.state = if t == 0 { 0x9e3779b97f4a7c15 } else { t };
        self.initialized = true;
        let k = (self.next_u64() % 3) as u8;
        self.kind = match k {
            0 => RngKind::Int,
            1 => RngKind::Float,
            _ => RngKind::String,
        };
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64
        let mut x = self.state;
        if x == 0 {
            x = 0x9e3779b97f4a7c15;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_int(&mut self, lo: i32, hi: i32) -> i32 {
        let span = (hi - lo) as u32;
        if span == 0 {
            return lo;
        }
        let v = (self.next_u64() as u32) % span;
        lo + v as i32
    }

    fn next_float(&mut self) -> f32 {
        let v = (self.next_u64() >> 40) as u32;
        (v as f32) / ((1u32 << 24) as f32)
    }
}

impl StdinState {
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(io::stdin()),
            buf: String::new(),
            closed: false,
        }
    }

    fn prepare(&mut self, n: usize) {
        if self.closed || n == 0 {
            return;
        }
        let mut bytes = vec![0u8; n];
        match self.reader.read(&mut bytes) {
            Ok(0) => self.closed = true,
            Ok(k) => {
                for b in &bytes[..k] {
                    self.buf.push(*b as char);
                }
            }
            Err(_) => self.closed = true,
        }
    }

    fn search(&mut self, pattern: &str) {
        if self.closed || pattern.is_empty() {
            return;
        }
        let pat_bytes = pattern.as_bytes().to_vec();
        let pat_len = pat_bytes.len();
        let mut tail: VecDeque<u8> = VecDeque::with_capacity(pat_len);
        loop {
            let mut one = [0u8];
            match self.reader.read(&mut one) {
                Ok(0) => {
                    self.closed = true;
                    return;
                }
                Ok(_) => {
                    self.buf.push(one[0] as char);
                    if tail.len() == pat_len {
                        tail.pop_front();
                    }
                    tail.push_back(one[0]);
                    if tail.len() == pat_len && tail.iter().copied().eq(pat_bytes.iter().copied()) {
                        return;
                    }
                }
                Err(_) => {
                    self.closed = true;
                    return;
                }
            }
        }
    }
}

impl Register {
    pub fn put(&mut self, value: Value) {
        match self {
            Register::Null => {}
            Register::Plain(v) => *v = value.flatten(),
            Register::Clock { speed, active } => {
                let n = value.to_int();
                *active = n != -1;
                *speed = n.clamp(1, 6000);
            }
            Register::Pin(ch) => ch.send(value.flatten()),
            Register::Offset(o) => *o = value.to_int(),
            Register::SizedMemory { mem, offset } => {
                if mem.is_empty() {
                    return;
                }
                let off = offset_value(offset);
                let idx = modular_index(off, mem.len());
                mem[idx] = value.flatten();
                *offset.borrow_mut() = Register::Offset(idx as i32);
            }
            Register::UnsizedMemory { mem, offset } => {
                let mut off = offset_value(offset);
                while (mem.len() as i32) <= off {
                    mem.push(Value::Null);
                }
                if off < 0 && !mem.is_empty() {
                    off += mem.len() as i32;
                    *offset.borrow_mut() = Register::Offset(off);
                }
                if off >= 0 && (off as usize) < mem.len() {
                    mem[off as usize] = value.flatten();
                }
            }
            Register::Stdout(t) => tape_write(t, &value, false),
            Register::Stderr(t) => tape_write(t, &value, true),
            Register::Stdin(s) => {
                if s.closed {
                    return;
                }
                match value.flatten() {
                    Value::S(pat) => s.search(&pat),
                    v => s.prepare(v.to_int().unsigned_abs() as usize),
                }
            }
            Register::Rng(r) => match value.flatten() {
                Value::Null => {
                    r.seed_time_based();
                }
                Value::S(s) => {
                    let n = Value::S(s.clone()).to_int();
                    r.seed = n;
                    r.state = (n as u64).wrapping_mul(2654435761) | 1;
                    r.kind = RngKind::String;
                    r.initialized = true;
                }
                Value::F(f) => {
                    let n = (f * 9999.0) as i32;
                    r.seed = n;
                    r.state = (n as u64).wrapping_mul(2654435761) | 1;
                    r.kind = RngKind::Float;
                    r.initialized = true;
                }
                Value::I(i) => {
                    r.seed = i;
                    r.state = (i as u64).wrapping_mul(2654435761) | 1;
                    r.kind = RngKind::Int;
                    r.initialized = true;
                }
                Value::Reg(_) => unreachable!(),
            },
            Register::FileReadControl(state) => {
                let mut s = state.borrow_mut();
                match value.flatten() {
                    Value::Null => {
                        s.file = None;
                    }
                    Value::I(n) => {
                        if let Some(ref mut f) = s.file {
                            let pos = n.max(0) as u64;
                            if f.seek(io::SeekFrom::Start(pos)).is_ok() {
                                s.byte_pos = pos;
                            }
                        }
                    }
                    Value::S(ref sv) => match sv.as_str() {
                        "s" => s.mode = FileMode::Str,
                        "i" => s.mode = FileMode::Int,
                        "f" => s.mode = FileMode::Float,
                        path => match std::fs::File::open(path) {
                            Ok(f) => {
                                s.file = Some(BufReader::new(f));
                                s.byte_pos = 0;
                            }
                            Err(e) => eprintln!("frc: cannot open '{}': {}", path, e),
                        },
                    },
                    _ => {}
                }
            }
            Register::FileWriteControl(state) => {
                let mut s = state.borrow_mut();
                match value.flatten() {
                    Value::Null => {
                        if let Some(ref mut f) = s.file {
                            let _ = f.flush();
                        }
                        s.file = None;
                    }
                    Value::I(n) => {
                        if let Some(ref mut f) = s.file {
                            // Flush buffer then seek the inner File.
                            let _ = f.flush();
                            let pos = if n < 0 {
                                let _ = f.seek(io::SeekFrom::End(0));
                                f.stream_position().unwrap_or(0)
                            } else {
                                let p = n as u64;
                                let _ = f.seek(io::SeekFrom::Start(p));
                                p
                            };
                            s.byte_pos = pos;
                        }
                    }
                    Value::S(ref sv) => match sv.as_str() {
                        "s" => s.mode = FileMode::Str,
                        "i" => s.mode = FileMode::Int,
                        "f" => s.mode = FileMode::Float,
                        path => {
                            use std::fs::OpenOptions;
                            match OpenOptions::new()
                                .write(true)
                                .create(true)
                                .truncate(true)
                                .open(path)
                            {
                                Ok(f) => {
                                    s.file = Some(BufWriter::new(f));
                                    s.byte_pos = 0;
                                }
                                Err(e) => eprintln!("fwc: cannot open '{}': {}", path, e),
                            }
                        }
                    },
                    _ => {}
                }
            }
            Register::FileReadTape(_) => {} // reads are via get(); put is a no-op
            Register::FileWriteTape { state, tape } => {
                let mut s = state.borrow_mut();
                let mode = s.mode;
                if let Some(ref mut f) = s.file {
                    match mode {
                        FileMode::Str => {
                            let mut text = value.as_string();
                            let _ = f.write_all(text.as_bytes());
                            let _ = f.flush();
                            s.byte_pos += text.len() as u64;
                            if tape.len() >= 32 {
                                tape.pop_front();
                            }
                            tape.push_back(text);
                        }
                        FileMode::Int => {
                            let bytes = value.to_int().to_le_bytes();
                            let _ = f.write_all(&bytes);
                            let _ = f.flush();
                            s.byte_pos += 4;
                            if tape.len() >= 32 {
                                tape.pop_front();
                            }
                            tape.push_back(value.to_int().to_string());
                        }
                        FileMode::Float => {
                            let bytes = value.to_float().to_le_bytes();
                            let _ = f.write_all(&bytes);
                            let _ = f.flush();
                            s.byte_pos += 4;
                            if tape.len() >= 32 {
                                tape.pop_front();
                            }
                            tape.push_back(value.to_float().to_string());
                        }
                    }
                }
            }
            #[cfg(feature = "gfx")]
            Register::Gfx {
                pixel_mem,
                pixel_offset,
                xsz,
                ysz,
                wsz,
                hsz,
                state,
            } => {
                let cmd = value.to_int();
                match cmd {
                    1 => {
                        // Open window (first time) then refresh pixels.
                        let xw = xsz.borrow_mut().get().to_int().max(1) as usize;
                        let yw = ysz.borrow_mut().get().to_int().max(1) as usize;
                        let ww = wsz.borrow_mut().get().to_int().max(1) as u32;
                        let hw = hsz.borrow_mut().get().to_int().max(1) as u32;
                        // Resize pixel memory if dimensions changed.
                        if let Register::SizedMemory { ref mut mem, .. } = *pixel_mem.borrow_mut() {
                            let new_size = xw * yw;
                            if mem.len() != new_size {
                                mem.resize(new_size, Value::Null);
                            }
                        }
                        let buf = gfx_read_pixels(pixel_mem, pixel_offset, xw, yw);
                        let mut s = state.lock().unwrap();
                        s.xsz = xw as u32;
                        s.ysz = yw as u32;
                        s.wsz = ww;
                        s.hsz = hw;
                        s.pixel_buf = buf;
                        s.needs_open = true;
                        s.needs_refresh = true;
                    }
                    0 => {
                        // Refresh without reopening.
                        let xw = xsz.borrow_mut().get().to_int().max(1) as usize;
                        let yw = ysz.borrow_mut().get().to_int().max(1) as usize;
                        let buf = gfx_read_pixels(pixel_mem, pixel_offset, xw, yw);
                        let mut s = state.lock().unwrap();
                        s.xsz = xw as u32;
                        s.ysz = yw as u32;
                        s.pixel_buf = buf;
                        s.needs_refresh = true;
                    }
                    2 => {
                        state.lock().unwrap().toggle_fullscreen = true;
                    }
                    -1 => {
                        state.lock().unwrap().needs_close = true;
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn get(&mut self) -> Value {
        match self {
            Register::Null => Value::Null,
            Register::Plain(v) => v.clone(),
            Register::Clock { speed, .. } => Value::I(*speed),
            Register::Pin(ch) => ch.receive(),
            Register::Offset(o) => Value::I(*o),
            Register::SizedMemory { mem, offset } => {
                if mem.is_empty() {
                    return Value::Null;
                }
                let off = offset_value(offset);
                let idx = modular_index(off, mem.len());
                let v = mem[idx].clone();
                *offset.borrow_mut() = Register::Offset(idx as i32);
                v
            }
            Register::UnsizedMemory { mem, offset } => {
                let off = offset_value(offset);
                if mem.is_empty() {
                    return Value::Null;
                }
                if off < 0 {
                    let neg = ((-off) as usize) % mem.len();
                    mem.get(mem.len() - neg).cloned().unwrap_or(Value::Null)
                } else {
                    mem.get(off as usize).cloned().unwrap_or(Value::Null)
                }
            }
            Register::Stdout(t) | Register::Stderr(t) => {
                t.tape.pop_back().map(Value::S).unwrap_or(Value::Null)
            }
            Register::Stdin(s) => {
                if s.closed && s.buf.trim().is_empty() {
                    return Value::Null;
                }
                let out = std::mem::take(&mut s.buf);
                Value::S(out)
            }
            Register::Rng(r) => {
                r.ensure_init();
                match r.kind {
                    RngKind::Int => Value::I(r.next_int(0, 999)),
                    RngKind::Float => Value::F(r.next_float()),
                    RngKind::String => {
                        let len = r.seed.unsigned_abs() as usize;
                        let mut out = String::with_capacity(len);
                        for _ in 0..len {
                            let c = r.next_int(32, 127) as u8 as char;
                            out.push(c);
                        }
                        Value::S(out)
                    }
                }
            }
            Register::FileReadControl(state) => Value::I(state.borrow().byte_pos as i32),
            Register::FileWriteControl(state) => Value::I(state.borrow().byte_pos as i32),
            Register::FileReadTape(state) => {
                let mut s = state.borrow_mut();
                match s.mode {
                    FileMode::Str => {
                        if let Some(ref mut f) = s.file {
                            let mut line = String::new();
                            match f.read_line(&mut line) {
                                Ok(0) => Value::Null,
                                Ok(_) => {
                                    s.byte_pos += line.len() as u64;
                                    if line.ends_with('\n') {
                                        line.pop();
                                        if line.ends_with('\r') {
                                            line.pop();
                                        }
                                    }
                                    Value::S(line)
                                }
                                Err(_) => Value::Null,
                            }
                        } else {
                            Value::Null
                        }
                    }
                    FileMode::Int => {
                        if let Some(ref mut f) = s.file {
                            let mut buf = [0u8; 4];
                            match f.read_exact(&mut buf) {
                                Ok(()) => {
                                    s.byte_pos += 4;
                                    Value::I(i32::from_le_bytes(buf))
                                }
                                Err(_) => Value::Null,
                            }
                        } else {
                            Value::Null
                        }
                    }
                    FileMode::Float => {
                        if let Some(ref mut f) = s.file {
                            let mut buf = [0u8; 4];
                            match f.read_exact(&mut buf) {
                                Ok(()) => {
                                    s.byte_pos += 4;
                                    Value::F(f32::from_le_bytes(buf))
                                }
                                Err(_) => Value::Null,
                            }
                        } else {
                            Value::Null
                        }
                    }
                }
            }
            Register::FileWriteTape { tape, .. } => {
                tape.pop_back().map(Value::S).unwrap_or(Value::Null)
            }
            #[cfg(feature = "gfx")]
            Register::Gfx { state, .. } => {
                let is_open = state.lock().unwrap().is_open;
                Value::I(if is_open { 1 } else { -1 })
            }
        }
    }

    pub fn as_pin_channel(&self) -> Option<&PinChannel> {
        match self {
            Register::Pin(c) => Some(c),
            _ => None,
        }
    }
}

fn offset_value(offset: &Rc<RefCell<Register>>) -> i32 {
    match &*offset.borrow() {
        Register::Offset(o) => *o,
        other => other_as_int(other),
    }
}

fn other_as_int(r: &Register) -> i32 {
    match r {
        Register::Plain(v) => v.to_int(),
        _ => 0,
    }
}

fn modular_index(offset: i32, size: usize) -> usize {
    if size == 0 {
        return 0;
    }
    let s = size as i32;
    let idx = if offset == s {
        0
    } else if offset > s {
        offset % s
    } else if offset < 0 {
        let neg = (-offset) % s;
        if neg == 0 { 0 } else { s - neg }
    } else {
        offset
    };
    (idx as usize).min(size - 1)
}

fn tape_write(t: &mut TapeState, value: &Value, is_err: bool) {
    let s = value.as_string();
    if t.tape.len() >= 32 {
        t.tape.pop_front();
    }
    t.tape.push_back(s.clone());
    t.is_err = is_err;
    if is_err {
        let _ = io::stderr().write_all(s.as_bytes());
        let _ = io::stderr().flush();
    } else {
        let _ = io::stdout().write_all(s.as_bytes());
        let _ = io::stdout().flush();
    }
}

pub fn new_plain() -> Register {
    Register::Plain(Value::I(0))
}

pub fn new_default_map() -> std::collections::HashMap<String, Rc<RefCell<Register>>> {
    use std::collections::HashMap;
    let mut m: HashMap<String, Rc<RefCell<Register>>> = HashMap::new();
    m.insert("null".to_string(), Rc::new(RefCell::new(Register::Null)));
    m.insert(
        "clk".to_string(),
        Rc::new(RefCell::new(Register::Clock {
            speed: 500,
            active: true,
        })),
    );
    m.insert("acc".to_string(), Rc::new(RefCell::new(new_plain())));
    m.insert(
        "stdout".to_string(),
        Rc::new(RefCell::new(Register::Stdout(TapeState {
            tape: VecDeque::new(),
            is_err: false,
        }))),
    );
    m.insert(
        "stderr".to_string(),
        Rc::new(RefCell::new(Register::Stderr(TapeState {
            tape: VecDeque::new(),
            is_err: true,
        }))),
    );
    m.insert(
        "stdin".to_string(),
        Rc::new(RefCell::new(Register::Stdin(StdinState::new()))),
    );
    m.insert(
        "rng".to_string(),
        Rc::new(RefCell::new(Register::Rng(RngState::new()))),
    );

    // File I/O: frc+frt share one read state; fwc+fwt share one write state.
    let fread = Rc::new(RefCell::new(FileReadState::new()));
    let fwrite = Rc::new(RefCell::new(FileWriteState::new()));
    m.insert(
        "frc".to_string(),
        Rc::new(RefCell::new(Register::FileReadControl(Rc::clone(&fread)))),
    );
    m.insert(
        "frt".to_string(),
        Rc::new(RefCell::new(Register::FileReadTape(Rc::clone(&fread)))),
    );
    m.insert(
        "fwc".to_string(),
        Rc::new(RefCell::new(Register::FileWriteControl(Rc::clone(&fwrite)))),
    );
    m.insert(
        "fwt".to_string(),
        Rc::new(RefCell::new(Register::FileWriteTape {
            state: Rc::clone(&fwrite),
            tape: VecDeque::new(),
        })),
    );

    // Commandline args:
    let raw_args: Vec<Value> = env::args().filter(|s|!s.ends_with(".sio")).map(|s| Value::S(s)).collect();
    let args: Vec<Value> = raw_args[1..].to_vec();
    let argc: i32 = args
        .len()
        .try_into()
        .expect("Too many command line arguments!");
    let argc_reg = Rc::new(RefCell::new(Register::Plain(Value::I(argc))));
    m.insert("&args".to_string(), Rc::clone(&argc_reg));
    m.insert(
        "*args".to_string(),
        Rc::new(RefCell::new(Register::SizedMemory {
            mem: args,
            offset: Rc::clone(&argc_reg),
        })),
    );
    m
}

#[cfg(feature = "gfx")]
pub fn add_graphical_registers(m: &mut std::collections::HashMap<String, Rc<RefCell<Register>>>) {
    use crate::channel::PinChannel;
    use std::sync::Arc;
    use std::sync::atomic::AtomicI32;

    let wsz_rc = Rc::new(RefCell::new(Register::Plain(Value::I(800))));
    let hsz_rc = Rc::new(RefCell::new(Register::Plain(Value::I(600))));
    let xsz_rc = Rc::new(RefCell::new(Register::Plain(Value::I(800))));
    let ysz_rc = Rc::new(RefCell::new(Register::Plain(Value::I(600))));
    let offset_rc = Rc::new(RefCell::new(Register::Offset(0)));
    let pxl_rc = Rc::new(RefCell::new(Register::SizedMemory {
        mem: vec![Value::Null; 800 * 600],
        offset: Rc::clone(&offset_rc),
    }));
    let kb_arc: Arc<AtomicI32> = crate::gfx::kb();
    let kb_pin = Rc::new(RefCell::new(Register::Pin(PinChannel::Power(kb_arc))));

    let gfx_state = crate::gfx::state();
    let gfx_rc = Rc::new(RefCell::new(Register::Gfx {
        pixel_mem: Rc::clone(&pxl_rc),
        pixel_offset: Rc::clone(&offset_rc),
        xsz: Rc::clone(&xsz_rc),
        ysz: Rc::clone(&ysz_rc),
        wsz: Rc::clone(&wsz_rc),
        hsz: Rc::clone(&hsz_rc),
        state: gfx_state,
    }));

    m.insert("wsz".to_string(), wsz_rc);
    m.insert("hsz".to_string(), hsz_rc);
    m.insert("xsz".to_string(), xsz_rc);
    m.insert("ysz".to_string(), ysz_rc);
    m.insert("&pxl".to_string(), offset_rc);
    m.insert("*pxl".to_string(), pxl_rc);
    m.insert("kb0".to_string(), kb_pin);
    m.insert("gfx".to_string(), gfx_rc);
}

#[cfg(feature = "gfx")]
fn gfx_read_pixels(
    pixel_mem: &Rc<RefCell<Register>>,
    pixel_offset: &Rc<RefCell<Register>>,
    xw: usize,
    yw: usize,
) -> Vec<u32> {
    let saved = pixel_offset.borrow_mut().get();
    let mut buf = vec![0u32; xw * yw];
    for y in 0..yw {
        for x in 0..xw {
            let pos = x + y * xw;
            pixel_offset.borrow_mut().put(Value::I(pos as i32));
            let v = pixel_mem.borrow_mut().get();
            buf[pos] = crate::gfx::value_to_argb(&v);
        }
    }
    pixel_offset.borrow_mut().put(saved);
    buf
}
