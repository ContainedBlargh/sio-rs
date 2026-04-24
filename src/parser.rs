use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::instruction::{Instruction, RegRef, TestKind};
use crate::node::Node;
use crate::pins::get_pin_channel;
use crate::register::{self, Register};
use crate::value::Value;

/// Format a parse error with file location and an optional hint.
/// Error strings may embed a hint after a `\nhint: ` sentinel.
fn format_parse_error(file: &str, line_num: usize, source: &str, msg: &str) -> String {
    let (main, hint) = msg
        .split_once("\nhint: ")
        .map(|(m, h)| (m, Some(h)))
        .unwrap_or((msg, None));
    let mut out = format!(
        "error in {}:{}: {}\n  | {}",
        file,
        line_num,
        main,
        source.trim()
    );
    if let Some(h) = hint {
        out.push_str(&format!("\n  = hint: {}", h));
    }
    out
}

pub fn parse_from_path(path: &str) -> Result<Node, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;
    let lines: Vec<String> = content.lines().map(|l| l.trim_end().to_string()).collect();
    let name = path
        .rsplit(|c: char| c == '/' || c == '\\')
        .next()
        .unwrap_or(path)
        .to_string();

    let mut labels: HashSet<String> = HashSet::new();
    for line in &lines {
        if let Some((lbl, _)) = try_match_label(line) {
            labels.insert(lbl);
        }
    }

    let mut registers = register::new_default_map();
    #[cfg(feature = "gfx")]
    if crate::gfx::GFX_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        register::add_graphical_registers(&mut registers);
    }
    let mut program: Vec<(bool, Instruction)> = Vec::new();
    let mut jmp_table: HashMap<String, usize> = HashMap::new();

    let mut i: usize = 0;
    while i < lines.len() {
        let raw = lines[i].clone();
        i += 1;
        let mut line = raw.trim().to_string();

        // 1) Pin declaration: $x0 / $p0
        if let Some((pin_label, is_xbus, rest)) = try_match_pin(&line) {
            let ch = get_pin_channel(
                pin_label
                    .trim_start_matches(|c: char| c == 'x' || c == 'p')
                    .parse::<i32>()
                    .unwrap_or(0),
                is_xbus,
            );
            registers.insert(
                pin_label.clone(),
                Rc::new(RefCell::new(Register::Pin(ch))),
            );
            if rest.trim().is_empty() {
                continue;
            }
            line = rest;
        }

        // 2) Plain register declaration: $name
        if let Some((reg_name, rest)) = try_match_register(&line) {
            registers.insert(
                reg_name,
                Rc::new(RefCell::new(register::new_plain())),
            );
            if rest.trim().is_empty() {
                continue;
            }
            line = rest;
        }

        // 3) Memory declaration: *name[N], *name, &name
        if let Some((mem_name, size_opt, rest)) = try_match_memory(&line) {
            let offset = Rc::new(RefCell::new(Register::Offset(0)));
            registers.insert(format!("&{}", mem_name), Rc::clone(&offset));
            let mem_reg = match size_opt {
                Some(n) => Register::SizedMemory {
                    mem: vec![Value::Null; n],
                    offset,
                },
                None => Register::UnsizedMemory {
                    mem: Vec::new(),
                    offset,
                },
            };
            registers.insert(
                format!("*{}", mem_name),
                Rc::new(RefCell::new(mem_reg)),
            );
            if rest.trim().is_empty() {
                continue;
            }
            line = rest;
        }

        // 4) Label declaration
        if let Some((label_name, rest)) = try_match_label(&line) {
            if jmp_table.contains_key(&label_name) {
                return Err(format_parse_error(
                    &name,
                    i,
                    &raw,
                    &format!("label '{}' is already defined\nhint: each label name must be unique; rename one of them", label_name),
                ));
            }
            jmp_table.insert(label_name, program.len());
            if rest.trim().is_empty() {
                continue;
            }
            line = rest;
        }

        // 5) Strip comments, check @, tokenize
        let stripped = no_comments(&line);
        if stripped.trim().is_empty() {
            continue;
        }
        let run_once = stripped.trim_start().starts_with('@');
        let content = stripped.replace('@', "");
        let tokens = tokenize(&content);
        if tokens.is_empty() {
            continue;
        }

        let remaining: &[String] = &lines[i..];
        let (instruction, consumed) =
            parse_instruction(&tokens, &registers, &labels, remaining)
                .map_err(|e| format_parse_error(&name, i, &raw, &e))?;
        if let Some(instr) = instruction {
            program.push((run_once, instr));
        }
        i += consumed;
    }

    for (label, target) in &jmp_table {
        if *target >= program.len() {
            return Err(format!(
                "error in {}: label '{}' has no instruction after it\n  = hint: every label must be followed by at least one instruction; add `nop` after it if nothing else fits",
                name, label
            ));
        }
    }

    Ok(Node::new(name, program, registers, jmp_table))
}

fn parse_instruction(
    tokens: &[String],
    registers: &HashMap<String, RegRef>,
    labels: &HashSet<String>,
    remaining_lines: &[String],
) -> Result<(Option<Instruction>, usize), String> {
    if tokens.is_empty() {
        return Ok((None, 0));
    }
    let op = tokens[0].to_lowercase();
    let args = &tokens[1..];

    let mon = |i: usize| args.get(i).cloned();
    let parse_val = |s: &str| -> Result<Value, String> { parse_value(s, registers) };

    let instr = match op.as_str() {
        "end" => Instruction::End,
        "nop" => Instruction::Nop,
        "mov" => {
            let l = mon(0).ok_or("mov requires 2 operands\nhint: usage is `mov <src> <dst>` — e.g. `mov 1 acc` or `mov acc $myvar`")?;
            let r = mon(1).ok_or("mov requires 2 operands\nhint: usage is `mov <src> <dst>` — e.g. `mov 1 acc` or `mov acc $myvar`")?;
            let src = parse_val(&l)?;
            let dst = parse_val(&r)?;
            match dst {
                Value::Reg(rc) => Instruction::Mov(src, rc),
                _ => return Err(format!(
                    "mov destination '{}' is not a register\nhint: the second argument must be a writable register (e.g. `acc`, `$myvar`), not a literal value",
                    r
                )),
            }
        }
        "swp" => {
            let l = mon(0).ok_or("swp requires 2 registers\nhint: usage is `swp <reg1> <reg2>` — e.g. `swp acc $tmp`")?;
            let r = mon(1).ok_or("swp requires 2 registers\nhint: usage is `swp <reg1> <reg2>` — e.g. `swp acc $tmp`")?;
            let a = must_register(&parse_val(&l)?, &l)?;
            let b = must_register(&parse_val(&r)?, &r)?;
            Instruction::Swp(a, b)
        }
        "jmp" => {
            let lbl = mon(0).ok_or("jmp requires a label\nhint: usage is `jmp <label>` — add `mylabel:` somewhere in the program")?;
            if !labels.contains(&lbl) {
                return Err(format!(
                    "jmp to unknown label '{}'\nhint: add `{}:` on its own line at the target location",
                    lbl, lbl
                ));
            }
            Instruction::Jmp(lbl)
        }
        "slp" => {
            let v = parse_val(&mon(0).ok_or("slp requires 1 operand\nhint: usage is `slp <duration>` — duration is in milliseconds (relative to clock speed)")?)?;
            Instruction::Slp(v)
        }
        "slx" => {
            let s = mon(0).ok_or("slx requires 1 register\nhint: usage is `slx <xbus-pin>` — e.g. `slx x0`; the register must be declared as `$x0`")?;
            let v = parse_val(&s)?;
            let rc = must_register(&v, &s)?;
            {
                let borrow = rc.borrow();
                match borrow.as_pin_channel() {
                    Some(ch) if ch.is_xbus() => {}
                    _ => return Err(format!(
                        "slx requires an XBus register, got '{}'\nhint: XBus registers are declared as `$xN` (e.g. `$x0`); power pins (`$pN`) cannot be used with slx",
                        s
                    )),
                }
            }
            Instruction::Slx(rc)
        }
        "gen" => {
            let p = mon(0).ok_or("gen requires 3 operands\nhint: usage is `gen <pin> <on_ms> <off_ms>` — e.g. `gen p0 10 10`")?;
            let on = mon(1).ok_or("gen requires 3 operands\nhint: usage is `gen <pin> <on_ms> <off_ms>` — e.g. `gen p0 10 10`")?;
            let off = mon(2).ok_or("gen requires 3 operands\nhint: usage is `gen <pin> <on_ms> <off_ms>` — e.g. `gen p0 10 10`")?;
            let rv = parse_val(&p)?;
            let rc = must_register(&rv, &p)?;
            {
                let borrow = rc.borrow();
                if borrow.as_pin_channel().is_none() {
                    return Err(format!(
                        "gen requires a pin register, got '{}'\nhint: declare a power or XBus pin with `$p0` or `$x0` and use that as the first argument",
                        p
                    ));
                }
            }
            Instruction::Gen(rc, parse_val(&on)?, parse_val(&off)?)
        }
        "add" => Instruction::Add(parse_val(&mon(0).ok_or("add requires 1 operand\nhint: usage is `add <value>` — adds value to acc")?)?),
        "sub" => Instruction::Sub(parse_val(&mon(0).ok_or("sub requires 1 operand\nhint: usage is `sub <value>` — subtracts value from acc")?)?),
        "mul" => Instruction::Mul(parse_val(&mon(0).ok_or("mul requires 1 operand\nhint: usage is `mul <value>` — multiplies acc by value")?)?),
        "div" => Instruction::Div(parse_val(&mon(0).ok_or("div requires 1 operand\nhint: usage is `div <value>` — divides acc by value")?)?),
        "not" => Instruction::Not,
        "cst" => Instruction::Cst(parse_val(&mon(0).ok_or("cst requires 1 operand\nhint: usage is `cst <type>` — type is one of \"i\", \"f\", \"s\", \"c\", \"rgb\", or \"iN\" for base-N integer parsing")?)?),
        "inc" => {
            let s = mon(0).ok_or("inc requires 1 register\nhint: usage is `inc <reg>` — increments the register by 1")?;
            let v = parse_val(&s)?;
            Instruction::Inc(must_register(&v, &s)?)
        }
        "dec" => {
            let s = mon(0).ok_or("dec requires 1 register\nhint: usage is `dec <reg>` — decrements the register by 1")?;
            let v = parse_val(&s)?;
            Instruction::Dec(must_register(&v, &s)?)
        }
        "dgt" => Instruction::Dgt(parse_val(&mon(0).ok_or("dgt requires 1 operand\nhint: usage is `dgt <index>` — extracts the character/digit at that position from acc")?)?),
        "dst" => {
            let l = mon(0).ok_or("dst requires 2 operands\nhint: usage is `dst <index> <value>` — sets the character/digit at index in acc to value")?;
            let r = mon(1).ok_or("dst requires 2 operands\nhint: usage is `dst <index> <value>` — sets the character/digit at index in acc to value")?;
            Instruction::Dst(parse_val(&l)?, parse_val(&r)?)
        }
        "teq" | "tgt" | "tlt" | "tcp" => {
            let kind = match op.as_str() {
                "teq" => TestKind::Teq,
                "tgt" => TestKind::Tgt,
                "tlt" => TestKind::Tlt,
                _ => TestKind::Tcp,
            };
            let l = mon(0).ok_or("test instruction requires 2 operands\nhint: usage is e.g. `teq acc 0` — follow it with `+ <instr>` and/or `- <instr>` branch lines")?;
            let r = mon(1).ok_or("test instruction requires 2 operands\nhint: usage is e.g. `teq acc 0` — follow it with `+ <instr>` and/or `- <instr>` branch lines")?;
            let (pos, neg, consumed) = parse_test_branches(registers, labels, remaining_lines)?;
            return Ok((
                Some(Instruction::Test {
                    kind,
                    left: parse_val(&l)?,
                    right: parse_val(&r)?,
                    pos,
                    neg,
                }),
                consumed,
            ));
        }
        other => {
            let known = "end nop mov swp jmp slp slx gen add sub mul div not cst inc dec dgt dst teq tgt tlt tcp";
            return Err(format!(
                "unknown instruction '{}'\nhint: known instructions are: {}",
                other, known
            ));
        }
    };

    Ok((Some(instr), 0))
}

fn is_test_line(line: &str) -> bool {
    let stripped = no_comments(line);
    let inner = stripped.trim_start_matches('@').trim_start();
    let op = inner.split_whitespace().next().unwrap_or("").to_lowercase();
    matches!(op.as_str(), "teq" | "tgt" | "tlt" | "tcp")
}

fn parse_test_branches(
    registers: &HashMap<String, RegRef>,
    labels: &HashSet<String>,
    remaining: &[String],
) -> Result<(Vec<Instruction>, Vec<Instruction>, usize), String> {
    let trimmed: Vec<String> = remaining.iter().map(|l| l.trim().to_string()).collect();
    if trimmed.is_empty() {
        return Ok((Vec::new(), Vec::new(), 0));
    }
    let first = &trimmed[0];
    let pos_first = first.starts_with('+');
    let neg_first = first.starts_with('-');

    // Lookahead: if no immediate branches but consecutive test instructions follow,
    // share their branches with this test (OR semantics for stacked tests).
    // consumed=0 so the intervening tests are still parsed normally by the main loop.
    if !pos_first && !neg_first {
        let skip = trimmed.iter().take_while(|l| is_test_line(l)).count();
        if skip > 0 && skip < trimmed.len() {
            let after = &trimmed[skip..];
            if after[0].starts_with('+') || after[0].starts_with('-') {
                let (pos, neg, _) = collect_branches(after, registers, labels)?;
                return Ok((pos, neg, 0));
            }
        }
        return Ok((Vec::new(), Vec::new(), 0));
    }

    collect_branches(&trimmed, registers, labels)
}

fn collect_branches(
    trimmed: &[String],
    registers: &HashMap<String, RegRef>,
    labels: &HashSet<String>,
) -> Result<(Vec<Instruction>, Vec<Instruction>, usize), String> {
    if trimmed.is_empty() {
        return Ok((Vec::new(), Vec::new(), 0));
    }
    let first = &trimmed[0];
    let pos_first = first.starts_with('+');
    let neg_first = first.starts_with('-');

    let (pos_raw, neg_raw) = if pos_first {
        let pos: Vec<String> = trimmed
            .iter()
            .take_while(|l| l.starts_with('+'))
            .cloned()
            .collect();
        let neg: Vec<String> = trimmed
            .iter()
            .skip(pos.len())
            .take_while(|l| l.starts_with('-'))
            .cloned()
            .collect();
        (pos, neg)
    } else if neg_first {
        let neg: Vec<String> = trimmed
            .iter()
            .take_while(|l| l.starts_with('-'))
            .cloned()
            .collect();
        let pos: Vec<String> = trimmed
            .iter()
            .skip(neg.len())
            .take_while(|l| l.starts_with('+'))
            .cloned()
            .collect();
        (pos, neg)
    } else {
        (Vec::new(), Vec::new())
    };

    let pos = parse_branch_lines(&pos_raw, '+', registers, labels, trimmed)?;
    let neg = parse_branch_lines(&neg_raw, '-', registers, labels, trimmed)?;
    let consumed = pos_raw.len() + neg_raw.len();
    Ok((pos, neg, consumed))
}

fn parse_branch_lines(
    branch_lines: &[String],
    marker: char,
    registers: &HashMap<String, RegRef>,
    labels: &HashSet<String>,
    full_remaining: &[String],
) -> Result<Vec<Instruction>, String> {
    let mut out = Vec::new();
    for (idx, raw) in branch_lines.iter().enumerate() {
        let stripped = raw.trim_start_matches(marker).trim_start();
        let no_comm = no_comments(stripped);
        let no_at = no_comm.replace('@', "");
        let tokens = tokenize(&no_at);
        if tokens.is_empty() {
            continue;
        }
        let remaining = if idx + 1 < full_remaining.len() {
            &full_remaining[idx + 1..]
        } else {
            &full_remaining[full_remaining.len()..]
        };
        let (instr, _consumed) = parse_instruction(&tokens, registers, labels, remaining)?;
        if let Some(i) = instr {
            out.push(i);
        }
    }
    Ok(out)
}

fn must_register(v: &Value, token: &str) -> Result<RegRef, String> {
    match v {
        Value::Reg(r) => Ok(Rc::clone(r)),
        _ => Err(format!(
            "'{}' is a literal value, but a register is required here\nhint: use a register like `acc` or a declared `${}` instead of a literal",
            token, token
        )),
    }
}

pub fn parse_value(token: &str, registers: &HashMap<String, RegRef>) -> Result<Value, String> {
    if let Ok(i) = token.parse::<i32>() {
        return Ok(Value::I(i));
    }
    if let Ok(f) = token.parse::<f32>() {
        return Ok(Value::F(f));
    }
    if (token.starts_with('"') && token.ends_with('"') && token.len() >= 2)
        || (token.starts_with('\'') && token.ends_with('\'') && token.len() >= 2)
    {
        let inner = &token[1..token.len() - 1];
        let unescaped = inner
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\r", "\r");
        return Ok(Value::S(unescaped));
    }
    if let Some(r) = registers.get(token) {
        return Ok(Value::Reg(Rc::clone(r)));
    }
    let hint = if token.chars().next().map_or(false, |c| c.is_alphabetic()) {
        format!(
            "\nhint: '{}' is not a known register or literal. If it's a variable, declare it at the top of the file with `${}`",
            token, token
        )
    } else {
        String::new()
    };
    Err(format!("unknown token '{}'{}", token, hint))
}

fn no_comments(line: &str) -> String {
    let mut in_str = false;
    let mut quote: char = '"';
    let mut cut: Option<usize> = None;
    for (i, c) in line.char_indices() {
        if in_str {
            if c == quote {
                in_str = false;
            }
        } else if c == '"' || c == '\'' {
            in_str = true;
            quote = c;
        } else if c == '#' || c == ';' {
            cut = Some(i);
            break;
        }
    }
    let out = match cut {
        Some(i) => &line[..i],
        None => line,
    };
    out.trim().to_string()
}

fn tokenize(s: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            let end = (i + 1).min(chars.len());
            tokens.push(chars[start..end].iter().collect());
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '"' && chars[i] != '\''
        {
            i += 1;
        }
        tokens.push(chars[start..i].iter().collect());
    }
    tokens
}

fn try_match_pin(line: &str) -> Option<(String, bool, String)> {
    // ^\$[px](\d+)\s?(.*)$
    let bytes = line.as_bytes();
    if bytes.len() < 3 || bytes[0] != b'$' {
        return None;
    }
    let kind = bytes[1];
    if kind != b'x' && kind != b'p' {
        return None;
    }
    let mut i = 2;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 2 {
        return None;
    }
    let digits = &line[2..i];
    let number: i32 = digits.parse().ok()?;
    let rest_start = if i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i + 1
    } else {
        i
    };
    let rest = if rest_start <= line.len() {
        line[rest_start..].to_string()
    } else {
        String::new()
    };
    let pin_label = format!("{}{}", kind as char, number);
    Some((pin_label, kind == b'x', rest))
}

fn try_match_register(line: &str) -> Option<(String, String)> {
    // ^\$([a-zA-Z0-9]+)\s?(.*)$
    let bytes = line.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'$' {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 1 {
        return None;
    }
    let name = line[1..i].to_string();
    let rest_start = if i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i + 1
    } else {
        i
    };
    let rest = if rest_start <= line.len() {
        line[rest_start..].to_string()
    } else {
        String::new()
    };
    Some((name, rest))
}

fn try_match_memory(line: &str) -> Option<(String, Option<usize>, String)> {
    // ^[*&]([A-Za-z0-9]+)(?:\[(\d+)\])?\s?(.*)$
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if bytes[0] != b'*' && bytes[0] != b'&' {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 1 {
        return None;
    }
    let name = line[1..i].to_string();
    let mut size: Option<usize> = None;
    if i < bytes.len() && bytes[i] == b'[' {
        let mut j = i + 1;
        let digit_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j == digit_start || j >= bytes.len() || bytes[j] != b']' {
            return None;
        }
        size = line[digit_start..j].parse::<usize>().ok();
        i = j + 1;
    }
    let rest_start = if i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i + 1
    } else {
        i
    };
    let rest = if rest_start <= line.len() {
        line[rest_start..].to_string()
    } else {
        String::new()
    };
    Some((name, size, rest))
}

fn try_match_label(line: &str) -> Option<(String, String)> {
    // ^\s*([A-Za-z_\-][A-Za-z0-9_\-]*):\s*(.*)$
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i >= bytes.len() {
        return None;
    }
    let first = bytes[i];
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'-') {
        return None;
    }
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
    {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    let name = line[start..i].to_string();
    let mut j = i + 1;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    let rest = if j <= line.len() {
        line[j..].to_string()
    } else {
        String::new()
    };
    Some((name, rest))
}

/// Scan source files for XBus pin declarations (`$xN`) and return a map from
/// pin ID to the list of file names that declare it.  Used to warn about
/// one-sided connections before any threads start running.
pub fn scan_xbus_declarations(paths: &[String]) -> std::collections::HashMap<i32, Vec<String>> {
    let mut map: std::collections::HashMap<i32, Vec<String>> = std::collections::HashMap::new();
    for path in paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let name = path
            .rsplit(|c: char| c == '/' || c == '\\')
            .next()
            .unwrap_or(path)
            .to_string();
        let mut seen = std::collections::HashSet::new();
        for line in content.lines() {
            if let Some((pin_label, true, _)) = try_match_pin(line.trim()) {
                let id: i32 = pin_label.trim_start_matches('x').parse().unwrap_or(-1);
                if seen.insert(id) {
                    map.entry(id).or_default().push(name.clone());
                }
            }
        }
    }
    map
}

/// Returns true if any source file references a graphical register as a token
/// outside comments. Reuses the same `no_comments` + `tokenize` logic as the
/// full parser so that `# mov 1 gfx` does not trigger a false positive.
#[cfg(feature = "gfx")]
pub fn source_uses_gfx(paths: &[String]) -> bool {
    const GFX_TOKENS: &[&str] = &["gfx", "*pxl", "&pxl", "kb0", "wsz", "hsz", "xsz", "ysz"];
    paths.iter().any(|path| {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        content.lines().any(|line| {
            let stripped = no_comments(line);
            tokenize(&stripped)
                .iter()
                .any(|t| GFX_TOKENS.contains(&t.as_str()))
        })
    })
}
