use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::instruction::{Executor, Instruction, RegRef};
use crate::register::Register;

pub struct Node {
    #[allow(dead_code)]
    pub name: String,
    program: Vec<(bool, Instruction)>,
    registers: HashMap<String, RegRef>,
    jmp_table: HashMap<String, usize>,
    pc: usize,
    disabled: HashSet<usize>,
    running: bool,
    jumped: bool,
}

impl Node {
    pub fn new(
        name: String,
        program: Vec<(bool, Instruction)>,
        registers: HashMap<String, RegRef>,
        jmp_table: HashMap<String, usize>,
    ) -> Self {
        Self {
            name,
            program,
            registers,
            jmp_table,
            pc: 0,
            disabled: HashSet::new(),
            running: true,
            jumped: false,
        }
    }

    pub fn run(mut self) {
        let program = std::mem::take(&mut self.program);
        let len = program.len();
        if len == 0 {
            return;
        }
        while self.running {
            let (active, speed) = self.clock_info();
            let tick_start = Instant::now();

            if self.disabled.contains(&self.pc) {
                self.pc = (self.pc + 1) % len;
                continue;
            }

            let (run_once, instr) = &program[self.pc];
            let run_once = *run_once;
            let exec_pc = self.pc;
            self.jumped = false;
            instr.execute(&mut self);
            if run_once {
                self.disabled.insert(exec_pc);
            }
            if !self.jumped {
                self.pc = (self.pc + 1) % len;
            }

            if active && speed > 0 {
                let wait = Duration::from_nanos(1_000_000_000u64 / speed as u64);
                let elapsed = tick_start.elapsed();
                if wait > elapsed {
                    std::thread::sleep(wait - elapsed);
                }
            }
        }
    }

    fn clock_info(&self) -> (bool, i32) {
        if let Some(clk) = self.registers.get("clk") {
            if let Register::Clock { speed, active } = &*clk.borrow() {
                return (*active, *speed);
            }
        }
        (true, 500)
    }
}

impl Executor for Node {
    fn get_register(&self, id: &str) -> Option<RegRef> {
        self.registers.get(id).cloned()
    }

    fn jump_to(&mut self, label: &str) {
        if let Some(&pos) = self.jmp_table.get(label) {
            self.pc = pos;
            self.jumped = true;
        }
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn sleep(&self, duration: i32) {
        let (active, speed) = self.clock_info();
        if !active || speed <= 0 || duration <= 0 {
            return;
        }
        let wait_ns = (duration as i64).saturating_mul(1_000_000_000) / speed as i64;
        if wait_ns > 15_000_000 {
            std::thread::sleep(Duration::from_nanos(wait_ns as u64));
        }
    }
}
