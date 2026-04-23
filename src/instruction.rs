use std::cell::RefCell;
use std::rc::Rc;

use crate::register::Register;
use crate::value::Value;

pub type RegRef = Rc<RefCell<Register>>;

pub enum Instruction {
    Nop,
    End,
    Mov(Value, RegRef),
    Swp(RegRef, RegRef),
    Jmp(String),
    Slp(Value),
    Slx(RegRef),
    Gen(RegRef, Value, Value),
    Inc(RegRef),
    Dec(RegRef),
    Add(Value),
    Sub(Value),
    Mul(Value),
    Div(Value),
    Not,
    Dgt(Value),
    Dst(Value, Value),
    Cst(Value),
    Test {
        kind: TestKind,
        left: Value,
        right: Value,
        pos: Vec<Instruction>,
        neg: Vec<Instruction>,
    },
}

#[derive(Clone, Copy)]
pub enum TestKind {
    Teq,
    Tgt,
    Tlt,
    Tcp,
}

pub trait Executor {
    fn get_register(&self, id: &str) -> Option<RegRef>;
    fn jump_to(&mut self, label: &str);
    fn stop(&mut self);
    fn sleep(&self, duration: i32);
}

impl Instruction {
    pub fn execute(&self, exec: &mut dyn Executor) {
        match self {
            Instruction::Nop => {}
            Instruction::End => {
                exec.stop();
                std::process::exit(0);
            }
            Instruction::Mov(src, dst) => {
                let v = src.flatten();
                dst.borrow_mut().put(v);
            }
            Instruction::Swp(a, b) => {
                let av = a.borrow_mut().get();
                let bv = b.borrow_mut().get();
                a.borrow_mut().put(bv);
                b.borrow_mut().put(av);
            }
            Instruction::Jmp(label) => exec.jump_to(label),
            Instruction::Slp(d) => exec.sleep(d.to_int()),
            Instruction::Slx(r) => {
                let borrow = r.borrow();
                if let Some(ch) = borrow.as_pin_channel() {
                    let ch = ch.clone();
                    drop(borrow);
                    ch.sleep_until_ready();
                }
            }
            Instruction::Gen(pin, on_dur, off_dur) => {
                let ch = pin
                    .borrow()
                    .as_pin_channel()
                    .cloned()
                    .expect("gen requires a pin register");
                ch.send(Value::I(100));
                exec.sleep(on_dur.to_int());
                ch.send(Value::I(0));
                exec.sleep(off_dur.to_int());
            }
            Instruction::Inc(r) => {
                let current = r.borrow_mut().get();
                let next = current.add(&Value::I(1));
                r.borrow_mut().put(next);
            }
            Instruction::Dec(r) => {
                let current = r.borrow_mut().get();
                let next = current.sub(&Value::I(1));
                r.borrow_mut().put(next);
            }
            Instruction::Add(v) => acc_update(exec, |acc| acc.add(v)),
            Instruction::Sub(v) => acc_update(exec, |acc| acc.sub(v)),
            Instruction::Mul(v) => acc_update(exec, |acc| acc.mul(v)),
            Instruction::Div(v) => acc_update(exec, |acc| acc.div(v)),
            Instruction::Not => acc_update(exec, |acc| acc.not()),
            Instruction::Dgt(v) => {
                let idx = v.to_int();
                acc_update(exec, |acc| acc.dgt(idx));
            }
            Instruction::Dst(i_val, v) => {
                let idx = i_val.to_int();
                let val = v.flatten();
                acc_update(exec, |acc| acc.dst(idx, &val));
            }
            Instruction::Cst(type_val) => {
                let ty = type_val.flatten();
                acc_update(exec, |acc| apply_cst(&acc, &ty));
            }
            Instruction::Test {
                kind,
                left,
                right,
                pos,
                neg,
            } => {
                match kind {
                    TestKind::Tcp => {
                        let cmp = left.compare(right);
                        let branch = if cmp == std::cmp::Ordering::Greater {
                            pos
                        } else if cmp == std::cmp::Ordering::Less {
                            neg
                        } else {
                            return;
                        };
                        for inst in branch {
                            inst.execute(exec);
                        }
                    }
                    _ => {
                        let result = match kind {
                            TestKind::Teq => left.compare(right) == std::cmp::Ordering::Equal,
                            TestKind::Tgt => left.compare(right) == std::cmp::Ordering::Greater,
                            TestKind::Tlt => left.compare(right) == std::cmp::Ordering::Less,
                            TestKind::Tcp => unreachable!(),
                        };
                        let branch = if result { pos } else { neg };
                        for inst in branch {
                            inst.execute(exec);
                        }
                    }
                }
            }
        }
    }
}

fn acc_update<F: FnOnce(Value) -> Value>(exec: &mut dyn Executor, f: F) {
    if let Some(acc) = exec.get_register("acc") {
        let current = acc.borrow_mut().get();
        let next = f(current);
        acc.borrow_mut().put(next);
    }
}

fn apply_cst(acc: &Value, ty: &Value) -> Value {
    match ty {
        Value::S(s) => match s.as_str() {
            "c" => match acc.flatten() {
                Value::I(i) => Value::S(
                    char::from_u32(i as u32)
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                ),
                Value::S(cs) => Value::I(cs.chars().next().map(|c| c as i32).unwrap_or(0)),
                _ => Value::I(-1),
            },
            "i" => Value::I(acc.to_int()),
            "f" => Value::F(acc.to_float()),
            "s" => Value::S(acc.as_string()),
            "rgb" => match acc.flatten() {
                Value::S(cs) => {
                    if cs.starts_with('#') && cs.len() >= 7 {
                        let r = u32::from_str_radix(&cs[1..3], 16).unwrap_or(0);
                        let g = u32::from_str_radix(&cs[3..5], 16).unwrap_or(0);
                        let b = u32::from_str_radix(&cs[5..7], 16).unwrap_or(0);
                        Value::S(format!("{} {} {}", r, g, b))
                    } else {
                        Value::S("0 0 0".to_string())
                    }
                }
                Value::I(c) => {
                    let r = (c >> 16) & 0xff;
                    let g = (c >> 8) & 0xff;
                    let b = c & 0xff;
                    Value::S(format!("{} {} {}", r / 255, g / 255, b / 255))
                }
                Value::F(c) => {
                    if (0.0..=1.0).contains(&c) {
                        let v = (c * 255.0) as i32;
                        Value::S(format!("{} {} {}", v, v, v))
                    } else {
                        acc.flatten()
                    }
                }
                _ => Value::S("0 0 0".to_string()),
            },
            other if other.starts_with('i') => {
                let radix: Option<u32> = other[1..].parse().ok();
                match radix {
                    Some(r) if (2..=36).contains(&r) => {
                        let s = acc.as_string();
                        let s = s.trim();
                        match i64::from_str_radix(s, r) {
                            Ok(n) => Value::I(n as i32),
                            Err(_) => Value::S(acc.as_string()),
                        }
                    }
                    _ => Value::S(acc.as_string()),
                }
            }
            _ => Value::S(acc.as_string()),
        },
        Value::I(_) => Value::I(acc.to_int()),
        Value::F(_) => Value::F(acc.to_float()),
        Value::Null => Value::Null,
        Value::Reg(r) => {
            let ty2 = r.borrow_mut().get();
            apply_cst(acc, &ty2)
        }
    }
}
