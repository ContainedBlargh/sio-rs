#[cfg(feature = "gfx")]
mod gfx;
mod channel;
mod instruction;
mod node;
mod parser;
mod pins;
mod register;
mod value;

use std::thread;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: sio-rs <file.sio> [file.sio ...]");
        std::process::exit(2);
    }

    // Detect gfx usage via comment-aware token scan before spawning threads.
    #[cfg(feature = "gfx")]
    let use_gfx = parser::source_uses_gfx(&args);
    #[cfg(not(feature = "gfx"))]
    let use_gfx = false;

    if use_gfx {
        #[cfg(feature = "gfx")]
        {
            // Initialise global gfx state so parsers running in node threads
            // can call add_graphical_registers.
            gfx::init();
            gfx::GFX_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);

            let handles: Vec<_> = args
                .into_iter()
                .map(|path| {
                    thread::spawn(move || match parser::parse_from_path(&path) {
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
                thread::spawn(move || match parser::parse_from_path(&path) {
                    Ok(node) => node.run(),
                    Err(e) => eprintln!("{}", e),
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    }
}
