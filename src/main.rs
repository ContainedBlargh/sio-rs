#[cfg(feature = "gfx")]
mod gfx;
// The debugger module must be declared when the dbg feature is active so that
// shared modules (node.rs) can reference crate::debugger via cfg(feature="dbg").
#[cfg(feature = "dbg")]
#[allow(dead_code, unused_imports)]
mod debugger;
mod channel;
mod instruction;
mod node;
mod parser;
mod pins;
mod register;
mod value;

use colored::Colorize;
use std::thread;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: sio-rs <file.sio> [file.sio ...]");
        std::process::exit(2);
    }

    // Anything that isn't a .sio file is a program argument visible via *args.
    let program_args: Vec<String> = args.iter()
        .filter(|s| !s.ends_with(".sio"))
        .cloned()
        .collect();

    // Warn about XBus pins that are only declared in one file — those will
    // block forever when the program tries to send or receive on them.
    let mut xbus_pins = parser::scan_xbus_declarations(&args)
        .into_iter()
        .filter(|(_, files)| files.len() == 1)
        .collect::<Vec<_>>();
    xbus_pins.sort_by_key(|(id, _)| *id);
    for (pin_id, files) in &xbus_pins {
        eprintln!(
            "{}: XBus pin {} is declared in only one node ({}); \
             any mov or slx on this pin will block forever",
            "warning".yellow().bold(),
            format!("x{}", pin_id).bold(),
            files[0].white()
        );
    }

    // Detect gfx usage via comment-aware token scan before spawning threads.
    #[cfg(feature = "gfx")]
    let use_gfx = parser::source_uses_gfx(&args);
    #[cfg(not(feature = "gfx"))]
    let use_gfx = false;

    if use_gfx {
        #[cfg(feature = "gfx")]
        {
            gfx::GFX_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);

            let handles: Vec<_> = args
                .into_iter()
                .map(|path| {
                    let pa = program_args.clone();
                    thread::spawn(move || match parser::parse_from_path(&path, &pa) {
                        Ok(node) => node.run(),
                        Err(e) => eprintln!("{}", e),
                    })
                })
                .collect();

            // Run the minifb event loop on the main thread (required on macOS/Windows).
            // Nodes terminate via process::exit; window close also calls process::exit.
            gfx::run_main_loop();

            // Unreachable in normal operation but keeps the type system happy.
            for h in handles {
                let _ = h.join();
            }
        }
    } else {
        let handles: Vec<_> = args
            .into_iter()
            .map(|path| {
                let pa = program_args.clone();
                thread::spawn(move || match parser::parse_from_path(&path, &pa) {
                    Ok(node) => node.run(),
                    // Ignore non-node strings, these are passed as commandline arguments.
                    Err(_e) => (),
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}
