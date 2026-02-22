//! `runec` — Rune compiler and runner CLI (Phase 3 Week 10 stub).
//!
//! Usage:
//!   runec compile <input.c> -o <output.rune>
//!   runec run <module.rune> <func> [args...]
//!   runec inspect <module.rune>

use rune::{Module, Runtime};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: runec <command> [args...]");
        eprintln!("Commands: run, inspect");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "run" => cmd_run(&args[2..]),
        "inspect" => cmd_inspect(&args[2..]),
        other => {
            eprintln!("Unknown command: {other}");
            std::process::exit(1);
        }
    }
}

fn cmd_run(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: runec run <module.rune> <func> [i32 args...]");
        std::process::exit(1);
    }
    let path = &args[0];
    let func = &args[1];

    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("Cannot read {path}: {e}");
        std::process::exit(1);
    });

    let module = Module::from_bytes(&bytes).unwrap_or_else(|e| {
        eprintln!("Invalid module: {e}");
        std::process::exit(1);
    });

    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).unwrap_or_else(|e| {
        eprintln!("Instantiation failed: {e}");
        std::process::exit(1);
    });

    let val_args: Vec<rune::Val> = args[2..]
        .iter()
        .map(|s| {
            rune::Val::I32(s.parse::<i32>().unwrap_or_else(|_| {
                eprintln!("Cannot parse arg {s:?} as i32");
                std::process::exit(1);
            }))
        })
        .collect();

    match inst.call(func, &val_args) {
        Ok(Some(v)) => println!("{v:?}"),
        Ok(None) => println!("(no return value)"),
        Err(e) => {
            eprintln!("Trap: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_inspect(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: runec inspect <module.rune>");
        std::process::exit(1);
    }
    let path = &args[0];
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("Cannot read {path}: {e}");
        std::process::exit(1);
    });
    let module = Module::from_bytes(&bytes).unwrap_or_else(|e| {
        eprintln!("Invalid module: {e}");
        std::process::exit(1);
    });

    println!("=== Rune Module: {path} ===");
    println!(
        "Memory: {} initial pages, max: {:?}",
        module.initial_memory_pages, module.max_memory_pages
    );
    println!("Functions: {}", module.functions.len());
    for (i, f) in module.functions.iter().enumerate() {
        println!("  [{i}] {} ({} ops)", f.name, f.body.len());
    }
    println!("Exports:");
    for (name, idx) in &module.exports {
        println!("  {name} -> func[{idx}]");
    }
    println!("Data segments: {}", module.data_segments.len());
}
