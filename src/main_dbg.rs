#![allow(dead_code)]
mod debugger;
mod channel;
mod instruction;
mod node;
mod parser;
mod pins;
mod register;
mod value;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (history_size, stdin_path, sio_args, program_args) = parse_dbg_flags(&args);
    if sio_args.is_empty() {
        eprintln!(
            "usage: sio-dbg [--dbg-history N] [--stdin <file|->] \
             <file.sio> [file.sio ...] [-- program-args]"
        );
        std::process::exit(2);
    }
    debugger::run_debug_session(sio_args, history_size, stdin_path, program_args);
}

fn parse_dbg_flags(args: &[String]) -> (usize, Option<String>, Vec<String>, Vec<String>) {
    let mut history_size: usize = 512;
    let mut stdin_path: Option<String> = None;
    let mut sio_files = Vec::new();
    let mut program_args = Vec::new();
    let mut i = 0;
    let mut past_separator = false;
    while i < args.len() {
        if past_separator {
            program_args.push(args[i].clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--" => {
                past_separator = true;
                i += 1;
            }
            "--dbg-history" => {
                if let Some(n) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    history_size = n;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--stdin" => {
                if let Some(path) = args.get(i + 1) {
                    stdin_path = Some(path.clone());
                    i += 2;
                } else {
                    eprintln!("sio-dbg: --stdin requires a path argument (use - for stdin)");
                    i += 1;
                }
            }
            _ => {
                sio_files.push(args[i].clone());
                i += 1;
            }
        }
    }
    (history_size, stdin_path, sio_files, program_args)
}
