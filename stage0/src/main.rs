// bastioc0 -- the stage0 (bootstrap) compiler for Bastio.
//
// Pipeline: lex -> parse -> check (types + linearity) -> verify (SMT) -> C.
// The C output is compiled with the system C compiler.
//
// usage: bastioc0 [--check] [--emit-c] [--dump-smt] [--target T] [-o out] <file.bastio>...
//   (default)   verify, then build a native executable `out` (default: a.out)
//   --check     stop after verification
//   --emit-c    write the generated C to `out` instead of building
//   --dump-smt  print every query sent to the solver
//   --target    `hosted` (default): a normal program on the C library
//               `kernel`: freestanding C for a bare-metal x86-64 kernel (implies --emit-c)
//               `user`: freestanding C for a user program under BastiOS (implies --emit-c)
// Several input files form one program; constants must be defined before use,
// in command-line order.

mod ast;
mod check;
mod codegen;
mod diag;
mod lexer;
mod parser;
mod smt;
mod verify;

use std::process::{exit, Command};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut check_only = false;
    let mut emit_c = false;
    let mut dump_smt = false;
    let mut target = codegen::Target::Hosted;
    let mut out: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check_only = true,
            "--emit-c" => emit_c = true,
            "--dump-smt" => dump_smt = true,
            "--target" => {
                i += 1;
                target = match args.get(i).map(|s| s.as_str()) {
                    Some("hosted") => codegen::Target::Hosted,
                    Some("kernel") => {
                        emit_c = true;
                        codegen::Target::Kernel
                    }
                    Some("user") => {
                        emit_c = true;
                        codegen::Target::User
                    }
                    other => {
                        eprintln!("bastioc0: unknown target {:?} (expected `hosted`, `kernel` or `user`)", other);
                        exit(2);
                    }
                };
            }
            "-o" => {
                i += 1;
                out = args.get(i).cloned();
            }
            a if a.starts_with('-') => {
                eprintln!("bastioc0: unknown option `{}`", a);
                exit(2);
            }
            a => inputs.push(a.to_string()),
        }
        i += 1;
    }
    if inputs.is_empty() {
        eprintln!("usage: bastioc0 [--check] [--emit-c] [--dump-smt] [--target hosted|kernel|user] [-o out] <file.bastio>...");
        exit(2);
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for input in &inputs {
        match std::fs::read_to_string(input) {
            Ok(s) => files.push((input.clone(), s)),
            Err(e) => {
                eprintln!("bastioc0: cannot read {}: {}", input, e);
                exit(2);
            }
        }
    }
    let input = inputs.join(" ");

    let fail = |d: diag::Diag| -> ! {
        eprint!("{}", d.render(&files));
        exit(1);
    };

    let mut prog = ast::Program { fns: Vec::new(), specs: Vec::new(), structs: Vec::new() };
    let mut consts = std::collections::HashMap::new();
    for (k, (path, src)) in files.iter().enumerate() {
        let toks = lexer::lex(src, k).unwrap_or_else(|d| fail(d));
        let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let mut parser = parser::Parser::new(toks, dir, consts);
        let part = parser.program().unwrap_or_else(|d| fail(d));
        consts = parser.consts;
        prog.fns.extend(part.fns);
        prog.specs.extend(part.specs);
        prog.structs.extend(part.structs);
    }
    check::check_structs(&prog).unwrap_or_else(|d| fail(d));
    ast::register_structs(prog.structs.clone());
    check::check_program(&mut prog, target).unwrap_or_else(|d| fail(d));
    let (n, log) = verify::verify_program(&prog).unwrap_or_else(|d| fail(d));
    if dump_smt {
        for l in &log {
            println!("{}", l);
        }
    }
    eprintln!("bastioc0: {}: {} proof obligation(s) discharged", input, n);
    if check_only {
        return;
    }

    let c = codegen::emit(&prog, target);
    if emit_c {
        let path = out.unwrap_or_else(|| "out.c".into());
        std::fs::write(&path, c).unwrap_or_else(|e| {
            eprintln!("bastioc0: cannot write {}: {}", path, e);
            exit(2)
        });
        return;
    }
    let exe = out.unwrap_or_else(|| "a.out".into());
    let cfile = format!("{}.c", exe);
    std::fs::write(&cfile, c).unwrap_or_else(|e| {
        eprintln!("bastioc0: cannot write {}: {}", cfile, e);
        exit(2)
    });
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let status = Command::new(&cc).args(["-O2", "-std=c11", "-w", "-o", &exe, &cfile]).status();
    let _ = std::fs::remove_file(&cfile);
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!("bastioc0: the C compiler failed (this is a compiler bug)");
            exit(3);
        }
        Err(e) => {
            eprintln!("bastioc0: cannot run `{}`: {}", cc, e);
            exit(3);
        }
    }
}
