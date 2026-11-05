//! Throwaway helper: parse a `rustc --emit=mir` file and print a summary.
use std::io::Read;

fn main() {
    let path = std::env::args().nth(1).expect("usage: mirdump FILE");
    let mut text = String::new();
    std::fs::File::open(&path).unwrap().read_to_string(&mut text).unwrap();
    match rmc::mir::parse(&text) {
        Ok(p) => {
            println!("functions: {}", p.funcs.len());
            for f in &p.funcs {
                println!(
                    "  {}: {} locals, {} blocks, args={}",
                    f.name,
                    f.locals.len(),
                    f.blocks.len(),
                    f.locals.iter().filter(|l| l.is_arg).count()
                );
                for (i, b) in f.blocks.iter().enumerate() {
                    if !matches!(b.term, rmc::mir::Terminator::Unreachable) || !b.stmts.is_empty() {
                        println!("    bb{i}: {} stmts, term={}", b.stmts.len(), short_term(&b.term));
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("PARSE ERROR: {e}");
            std::process::exit(1);
        }
    }
}

fn short_term(t: &rmc::mir::Terminator) -> String {
    match t {
        rmc::mir::Terminator::Goto(n) => format!("goto {n}"),
        rmc::mir::Terminator::Return => "return".into(),
        rmc::mir::Terminator::Unreachable => "unreachable".into(),
        rmc::mir::Terminator::Abort => "abort".into(),
        rmc::mir::Terminator::SwitchInt { targets, otherwise, .. } => {
            format!("switch {} targets -> else {otherwise}", targets.len())
        }
        rmc::mir::Terminator::Call { func, .. } => format!("call {func:?}"),
        rmc::mir::Terminator::Assert { negate, .. } => format!("assert negate={negate}"),
    }
}
