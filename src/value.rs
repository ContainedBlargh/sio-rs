use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use crate::register::Register;

#[derive(Clone)]
pub enum Value {
    Null,
    I(i32),
    F(f32),
    S(String),
    Reg(Rc<RefCell<Register>>),
}

// Send-safe flattened form for cross-thread channel transport.
#[derive(Clone)]
pub enum FlatValue {
    Null,
    I(i32),
    F(f32),
    S(String),
}

impl FlatValue {
    pub fn from_value(v: Value) -> Self {
        match v.flatten() {
            Value::Null => FlatValue::Null,
            Value::I(i) => FlatValue::I(i),
            Value::F(f) => FlatValue::F(f),
            Value::S(s) => FlatValue::S(s),
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn into_value(self) -> Value {
        match self {
            FlatValue::Null => Value::Null,
            FlatValue::I(i) => Value::I(i),
            FlatValue::F(f) => Value::F(f),
            FlatValue::S(s) => Value::S(s),
        }
    }
}

fn deref(r: &Rc<RefCell<Register>>) -> Value {
    r.borrow_mut().get()
}

impl Value {
    pub fn as_string(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::I(i) => i.to_string(),
            Value::F(f) => format!("{:.8}", f),
            Value::S(s) => s.clone(),
            Value::Reg(r) => deref(r).as_string(),
        }
    }

    pub fn to_int(&self) -> i32 {
        match self {
            Value::Null => 0,
            Value::I(i) => *i,
            Value::F(f) => *f as i32,
            Value::S(s) => s
                .trim()
                .parse::<i32>()
                .ok()
                .or_else(|| {
                    let mut chars = s.chars();
                    let first = chars.next()?;
                    if chars.next().is_none() {
                        Some(first as i32)
                    } else {
                        None
                    }
                })
                .unwrap_or(0),
            Value::Reg(r) => deref(r).to_int(),
        }
    }

    pub fn to_float(&self) -> f32 {
        match self {
            Value::Null => f32::NAN,
            Value::I(i) => *i as f32,
            Value::F(f) => *f,
            Value::S(s) => s.parse::<f32>().unwrap_or(0.0),
            Value::Reg(r) => deref(r).to_float(),
        }
    }

    pub fn flatten(&self) -> Value {
        match self {
            Value::Reg(r) => deref(r).flatten(),
            v => v.clone(),
        }
    }

    pub fn add(&self, other: &Value) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(i) => Value::I(i.wrapping_add(other.to_int())),
            Value::F(f) => Value::F(f + other.to_float()),
            Value::S(s) => match other.flatten() {
                Value::S(rs) => Value::S(format!("{}{}", s, rs)),
                Value::F(f) => Value::S(format!("{}{}", s, float_kotlin_str(f))),
                Value::I(i) => Value::S(format!("{}{}", s, i)),
                Value::Null => Value::S(s.clone()),
                Value::Reg(_) => unreachable!(),
            },
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn sub(&self, other: &Value) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(i) => Value::I(i.wrapping_sub(other.to_int())),
            Value::F(f) => Value::F(f - other.to_float()),
            Value::S(s) => match other.flatten() {
                Value::S(rs) => Value::S(s.replace(&rs, "")),
                Value::I(i) => {
                    let chars: Vec<char> = s.chars().collect();
                    let end = (i.max(0) as usize).min(chars.len());
                    Value::S(chars[..end].iter().collect())
                }
                Value::F(f) => s_sub_f(s, f),
                Value::Null => Value::S(s.clone()),
                Value::Reg(_) => unreachable!(),
            },
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn mul(&self, other: &Value) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(i) => Value::I(i.wrapping_mul(other.to_int())),
            Value::F(f) => Value::F(f * other.to_float()),
            Value::S(s) => match other.flatten() {
                Value::S(rs) => {
                    let mut out = String::new();
                    for c1 in s.chars() {
                        for c2 in rs.chars() {
                            out.push(c1);
                            out.push(c2);
                        }
                    }
                    Value::S(out)
                }
                Value::F(f) => Value::S(scale(s, f)),
                Value::I(i) => {
                    if i <= 0 {
                        Value::S(String::new())
                    } else {
                        Value::S(s.repeat(i as usize))
                    }
                }
                Value::Null => Value::S(String::new()),
                Value::Reg(_) => unreachable!(),
            },
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn div(&self, other: &Value) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(i) => {
                let o = other.to_int();
                if o == 0 {
                    Value::I(0)
                } else {
                    Value::I(i.wrapping_div(o))
                }
            }
            Value::F(f) => Value::F(f / other.to_float()),
            Value::S(s) => match other.flatten() {
                Value::F(f) => Value::S(scale(s, f.powi(-1))),
                Value::I(i) => {
                    if i == 0 {
                        Value::S(String::new())
                    } else {
                        Value::S(scale(s, (i as f32).powi(-1)))
                    }
                }
                Value::Null => Value::S("NaN".to_string()),
                Value::S(rs) => {
                    if rs.is_empty() {
                        Value::S(s.clone())
                    } else {
                        let chunk = rs.chars().count();
                        let chars: Vec<char> = s.chars().collect();
                        let mut out = String::new();
                        for c in chars.chunks(chunk) {
                            if let Some(first) = c.first() {
                                out.push(*first);
                            }
                        }
                        Value::S(out)
                    }
                }
                Value::Reg(_) => unreachable!(),
            },
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn not(&self) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(i) => Value::I(if *i == 0 { 100 } else { 0 }),
            Value::F(f) => Value::I(if (*f as i32) != 0 { 0 } else { 100 }),
            Value::S(s) => {
                let out: String = s.bytes().map(|b| (!b) as char).collect();
                Value::S(out)
            }
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn dgt(&self, i: i32) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(n) => Value::I(digit_at(&n.to_string(), i)),
            Value::F(f) => Value::I(digit_at(&(*f as i32).to_string(), i)),
            Value::S(s) => Value::S(
                s.chars()
                    .nth(i.max(0) as usize)
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
            ),
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn dst(&self, i: i32, v: &Value) -> Value {
        let a = self.flatten();
        match &a {
            Value::Null => Value::Null,
            Value::I(n) => Value::I(set_digit(*n, i, v.to_int())),
            Value::F(f) => Value::I(set_digit(*f as i32, i, v.to_int())),
            Value::S(s) => Value::S(replace_at_kotlin(s, i, &v.as_string())),
            Value::Reg(_) => unreachable!(),
        }
    }

    pub fn compare(&self, other: &Value) -> Ordering {
        let a = self.flatten();
        let b = other.flatten();
        match &a {
            Value::Null => match &b {
                Value::Null => Ordering::Equal,
                Value::S(s) if s.trim().is_empty() || s == "\u{0000}" => Ordering::Equal,
                _ => Ordering::Less,
            },
            Value::I(i) => i.cmp(&b.to_int()),
            Value::F(f) => match &b {
                Value::Null => Ordering::Less,
                _ => f.partial_cmp(&b.to_float()).unwrap_or(Ordering::Equal),
            },
            Value::S(s) => match &b {
                Value::Null => {
                    if s.trim().is_empty() || s == "\u{0000}" {
                        Ordering::Equal
                    } else {
                        Ordering::Less
                    }
                }
                _ => s.as_str().cmp(b.as_string().as_str()),
            },
            Value::Reg(_) => unreachable!(),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

fn float_kotlin_str(f: f32) -> String {
    if f.is_nan() {
        "NaN".to_string()
    } else if f == f.trunc() && f.abs() < 1e16 {
        format!("{}.0", f as i64)
    } else {
        format!("{}", f)
    }
}

fn scale(s: &str, f: f32) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len == 0 || !f.is_finite() {
        return String::new();
    }
    let mut p = (f * len as f32) as i32;
    let mut out = String::new();
    while p > 0 {
        let r = (p as usize).min(len);
        for i in 0..r {
            out.push(chars[i]);
        }
        p -= r as i32;
    }
    out
}

fn digit_at(s: &str, i: i32) -> i32 {
    if i < 0 {
        return 0;
    }
    s.chars()
        .nth(i as usize)
        .and_then(|c| c.to_digit(10))
        .map(|d| d as i32)
        .unwrap_or(0)
}

fn set_digit(number: i32, i: i32, digit: i32) -> i32 {
    let s = number.to_string();
    let most_sig = digit
        .to_string()
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "0".to_string());
    if i < 0 {
        return number;
    }
    let idx = i as usize;
    let chars: Vec<char> = s.chars().collect();
    if idx >= chars.len() {
        return number;
    }
    let left: String = chars[..idx].iter().collect();
    let right: String = chars[idx + 1..].iter().collect();
    format!("{}{}{}", left, most_sig, right)
        .parse::<i32>()
        .unwrap_or(number)
}

fn s_sub_f(s: &str, f: f32) -> Value {
    let formatted = format!("{:.6}.1", f);
    let mut parts: Vec<String> = formatted.split('.').map(|p| p.to_string()).collect();
    parts.sort();
    let first = parts
        .first()
        .and_then(|p| p.parse::<i32>().ok())
        .unwrap_or(0);
    let last = parts
        .last()
        .and_then(|p| p.parse::<i32>().ok())
        .unwrap_or(0);
    let chars: Vec<char> = s.chars().collect();
    let slen = chars.len() as i32;
    let l = first.max(slen - 1).max(0) as usize;
    let r = last.max(slen).max(0) as usize;
    let l = l.min(chars.len());
    let r = r.min(chars.len());
    if l <= r {
        Value::S(chars[l..r].iter().collect())
    } else {
        Value::S(String::new())
    }
}

fn replace_at_kotlin(_this: &str, i: i32, replacement: &str) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    let len = chars.len() as i32;
    let left = if i > 1 {
        let end = ((i - 1) as usize).min(chars.len());
        chars[..end].iter().collect::<String>()
    } else {
        String::new()
    };
    let right = if i < len - 1 {
        let start = ((i + 1) as usize).min(chars.len());
        chars[start..].iter().collect::<String>()
    } else {
        String::new()
    };
    format!("{}{}{}", left, replacement, right)
}
