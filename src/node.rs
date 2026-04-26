use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::time::Instant;

use crate::instruction::{Executor, Instruction, RegRef};
use crate::register::Register;

#[cfg(feature = "dbg")]
use std::sync::mpsc;
#[cfg(feature = "dbg")]
use crate::debugger::state::{DebugCommand, NodeUpdate, Snapshot};
#[cfg(feature = "dbg")]
use crate::value::FlatValue;

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

#[cfg(feature = "dbg")]
pub struct DebugBridge {
    pub cmd_rx: mpsc::Receiver<DebugCommand>,
    pub update_tx: mpsc::Sender<NodeUpdate>,
    pub node_index: usize,
    pub pc_to_instr_repr: Vec<String>,
}

#[cfg(feature = "dbg")]
impl Node {
    pub fn run_debug(mut self, bridge: DebugBridge) {
        let program = std::mem::take(&mut self.program);
        let len = program.len();
        if len == 0 {
            let _ = bridge.update_tx.send(NodeUpdate {
                node_index: bridge.node_index,
                snapshot: Snapshot {
                    step_index: 0,
                    pc: 0,
                    registers: Vec::new(),
                    instruction_repr: "<empty program>".to_string(),
                },
                is_paused: true,
                is_terminated: true,
            });
            return;
        }

        let mut step_index: u64 = 0;
        self.send_snapshot(&bridge, &program, step_index, true);

        loop {
            match bridge.cmd_rx.recv() {
                Err(_) => break,
                Ok(DebugCommand::Quit) => break,
                Ok(DebugCommand::StepForward) => {
                    if !self.running {
                        self.send_terminated(&bridge, &program, step_index);
                        break;
                    }
                    let advanced = self.execute_one_step(&program, len);
                    if advanced { step_index += 1; }
                    let terminated = !self.running;
                    self.send_snapshot(&bridge, &program, step_index, true);
                    if terminated { break; }
                }
                Ok(DebugCommand::Continue) => {
                    self.run_continuous(&program, len, &bridge, &mut step_index);
                    if !self.running { break; }
                }
                Ok(DebugCommand::Pause) => {
                    self.send_snapshot(&bridge, &program, step_index, true);
                }
            }
        }
    }

    /// Returns true if an instruction was actually executed, false if blocked on XBus.
    fn execute_one_step(&mut self, program: &[(bool, Instruction)], len: usize) -> bool {
        if self.disabled.contains(&self.pc) {
            self.pc = (self.pc + 1) % len;
            return true;
        }
        let (run_once, instr) = &program[self.pc];
        let run_once = *run_once;
        let exec_pc = self.pc;
        self.jumped = false;
        // try_execute returns false when the instruction is blocked on an XBus
        // channel that has no value yet (or is full). Leave PC unchanged so the
        // TUI can step the other node(s) to unblock us, then retry.
        if !instr.try_execute(self) {
            return false;
        }
        if run_once {
            self.disabled.insert(exec_pc);
        }
        if !self.jumped && self.running {
            self.pc = (self.pc + 1) % len;
        }
        true
    }

    fn run_continuous(
        &mut self,
        program: &[(bool, Instruction)],
        len: usize,
        bridge: &DebugBridge,
        step_index: &mut u64,
    ) {
        loop {
            if !self.running {
                self.send_terminated(bridge, program, *step_index);
                return;
            }
            match bridge.cmd_rx.try_recv() {
                Ok(DebugCommand::Pause) | Ok(DebugCommand::Quit) => {
                    self.send_snapshot(bridge, program, *step_index, true);
                    return;
                }
                _ => {}
            }
            if self.execute_one_step(program, len) {
                *step_index += 1;
            }
            if *step_index % 100 == 0 {
                self.send_snapshot(bridge, program, *step_index, false);
            }
        }
    }

    fn collect_registers(&self) -> Vec<(String, FlatValue)> {
        let mut regs: Vec<(String, FlatValue)> = self
            .registers
            .iter()
            .filter_map(|(k, rc)| {
                let borrow = rc.borrow();
                // Skip XBus Pin registers — their .get() blocks waiting for a sender.
                if matches!(&*borrow, Register::Pin(ch) if ch.is_xbus()) {
                    return None;
                }
                // Show DebugStdin pending state without consuming its buffer.
                #[cfg(feature = "dbg")]
                if let Register::DebugStdin(shared) = &*borrow {
                    let label = match &shared.inner.lock().unwrap().pending {
                        Some(crate::register::StdinRequest::Bytes(n)) => format!("<waiting {} bytes>", n),
                        Some(crate::register::StdinRequest::Pattern(p)) => format!("<waiting {:?}>", p),
                        None => "<ready>".to_string(),
                    };
                    return Some((k.clone(), FlatValue::S(label)));
                }
                drop(borrow);
                let val = rc.borrow_mut().get();
                Some((k.clone(), FlatValue::from_value(val)))
            })
            .collect();
        regs.sort_by(|a, b| {
            // acc first, then alphabetical
            match (a.0.as_str(), b.0.as_str()) {
                ("acc", _) => std::cmp::Ordering::Less,
                (_, "acc") => std::cmp::Ordering::Greater,
                _ => a.0.cmp(&b.0),
            }
        });
        regs
    }

    fn send_snapshot(
        &self,
        bridge: &DebugBridge,
        _program: &[(bool, Instruction)],
        step_index: u64,
        is_paused: bool,
    ) {
        let instr_repr = bridge
            .pc_to_instr_repr
            .get(self.pc)
            .cloned()
            .unwrap_or_else(|| format!("<pc={}>", self.pc));
        let snap = Snapshot {
            step_index,
            pc: self.pc,
            registers: self.collect_registers(),
            instruction_repr: instr_repr,
        };
        let _ = bridge.update_tx.send(NodeUpdate {
            node_index: bridge.node_index,
            snapshot: snap,
            is_paused,
            is_terminated: false,
        });
    }

    fn send_terminated(
        &self,
        bridge: &DebugBridge,
        _program: &[(bool, Instruction)],
        step_index: u64,
    ) {
        let snap = Snapshot {
            step_index,
            pc: self.pc,
            registers: self.collect_registers(),
            instruction_repr: "<terminated>".to_string(),
        };
        let _ = bridge.update_tx.send(NodeUpdate {
            node_index: bridge.node_index,
            snapshot: snap,
            is_paused: true,
            is_terminated: true,
        });
    }
}
