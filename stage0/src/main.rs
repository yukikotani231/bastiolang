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
//   --dump-tokens  print the tokens of one file (see `lexer::dump`) and exit;
//               a lexical error prints `error line:col` and exits 1
//   --dump-ast  print the parse of one file (see `dump.rs`) and exit; an
//               error prints `error line:col` and exits 1
//   --trusted-base  list and count the hand-written trusted base, then exit
//   --target    `hosted` (default): a normal program on the C library
//               `kernel`: freestanding C for a bare-metal x86-64 kernel (implies --emit-c)
//               `user`: freestanding C for a user program under BastiOS (implies --emit-c)
// Several input files form one program; constants must be defined before use,
// in command-line order. A file can also pull others in with `import "path";`
// lines at its top (see `Loader`).

mod ast;
mod check;
mod codegen;
mod diag;
mod dump;
mod lexer;
mod mono;
mod parser;
mod smt;
mod verify;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

/// Reads source files into one program. A file may start with
/// `import "path";` lines (paths relative to the file); each imported file
/// is loaded, with its own imports, before the rest of the importing file
/// is parsed, so constants it defines can be used there. A file is loaded
/// once however often it is imported; a cycle of imports is an error.
struct Loader {
    files: Vec<(String, String)>,
    /// Canonical path -> finished loading (false while its imports load).
    state: HashMap<PathBuf, bool>,
    prog: ast::Program,
    consts: HashMap<String, u128>,
}

impl Loader {
    /// Loads `path`. `from` is the import that names it (None for a file
    /// on the command line). Errors are diagnostics, or None when the
    /// message has already been printed.
    fn load(&mut self, path: &Path, from: Option<diag::Pos>) -> Result<(), Option<diag::Diag>> {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                return Err(match from {
                    Some(pos) => Some(diag::Diag::new("E-IMPORT", pos, format!("cannot read `{}`: {}", path.display(), e))),
                    None => {
                        eprintln!("bastioc0: cannot read {}: {}", path.display(), e);
                        None
                    }
                })
            }
        };
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        match self.state.get(&canon) {
            Some(true) => return Ok(()),
            Some(false) => {
                let pos = from.expect("a file on the command line is not loading yet");
                return Err(Some(diag::Diag::new("E-IMPORT", pos, format!("`{}` imports itself (through this chain of imports)", path.display()))));
            }
            None => {}
        }
        self.state.insert(canon.clone(), false);
        let k = self.files.len();
        self.files.push((path.display().to_string(), src));
        let toks = lexer::lex(&self.files[k].1, k).map_err(Some)?;
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        // Leading `import "path";` lines.
        let mut i = 0;
        while toks[i].tok == lexer::Tok::Import {
            let pos = toks[i].pos;
            let lexer::Tok::Str(name) = &toks[i + 1].tok else {
                return Err(Some(diag::Diag::new("E-PARSE", toks[i + 1].pos, "expected a file name in quotes after `import`")));
            };
            if toks[i + 2].tok != lexer::Tok::Semi {
                return Err(Some(diag::Diag::new("E-PARSE", toks[i + 2].pos, "expected `;` after the imported file name")));
            }
            let name = String::from_utf8_lossy(name).to_string();
            self.load(&dir.join(name), Some(pos))?;
            i += 3;
        }
        let consts = std::mem::take(&mut self.consts);
        let mut parser = parser::Parser::new(toks[i..].to_vec(), dir, consts);
        let part = parser.program().map_err(Some)?;
        self.consts = parser.consts;
        self.prog.fns.extend(part.fns);
        self.prog.specs.extend(part.specs);
        self.prog.structs.extend(part.structs);
        self.prog.enums.extend(part.enums);
        self.state.insert(canon, true);
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut check_only = false;
    let mut emit_c = false;
    let mut dump_smt = false;
    let mut dump_tokens = false;
    let mut dump_ast = false;
    let mut target = codegen::Target::Hosted;
    let mut out: Option<String> = None;
    let mut inputs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check_only = true,
            "--emit-c" => emit_c = true,
            "--dump-smt" => dump_smt = true,
            "--dump-tokens" => dump_tokens = true,
            "--dump-ast" => dump_ast = true,
            "--trusted-base" => {
                print!("{}", codegen::trusted_base());
                return;
            }
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
    if dump_tokens {
        let src = std::fs::read_to_string(&inputs[0]).unwrap_or_else(|e| {
            eprintln!("bastioc0: cannot read {}: {}", inputs[0], e);
            exit(2)
        });
        match lexer::lex(&src, 0) {
            Ok(toks) => toks.iter().for_each(|t| println!("{}", lexer::dump(t))),
            Err(d) => {
                println!("error {}:{}", d.pos.line, d.pos.col);
                exit(1);
            }
        }
        return;
    }
    if dump_ast {
        let path = Path::new(&inputs[0]);
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("bastioc0: cannot read {}: {}", inputs[0], e);
            exit(2)
        });
        let err = |d: diag::Diag| -> ! {
            println!("error {}:{}", d.pos.line, d.pos.col);
            exit(1)
        };
        let toks = lexer::lex(&src, 0).unwrap_or_else(|d| err(d));
        let mut i = 0;
        while toks[i].tok == lexer::Tok::Import {
            let lexer::Tok::Str(name) = &toks[i + 1].tok else { err(diag::Diag::new("E-PARSE", toks[i + 1].pos, "")) };
            if toks[i + 2].tok != lexer::Tok::Semi {
                err(diag::Diag::new("E-PARSE", toks[i + 2].pos, ""));
            }
            println!("(import {}:{} {})", toks[i].pos.line, toks[i].pos.col, name.iter().map(|x| format!("{:02x}", x)).collect::<String>());
            i += 3;
        }
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let mut parser = parser::Parser::new(toks[i..].to_vec(), dir, HashMap::new());
        let prog = parser.program().unwrap_or_else(|d| err(d));
        print!("{}", dump::program(&prog));
        return;
    }
    let input = inputs.join(" ");
    let mut ld = Loader {
        files: Vec::new(),
        state: HashMap::new(),
        prog: ast::Program { fns: Vec::new(), specs: Vec::new(), structs: Vec::new(), enums: Vec::new() },
        consts: HashMap::new(),
    };
    for input in &inputs {
        if let Err(d) = ld.load(Path::new(input), None) {
            match d {
                Some(d) => eprint!("{}", d.render(&ld.files)),
                None => {}
            }
            exit(if ld.files.is_empty() { 2 } else { 1 });
        }
    }
    let Loader { files, mut prog, .. } = ld;

    let fail = |d: diag::Diag| -> ! {
        eprint!("{}", d.render(&files));
        exit(1);
    };
    mono::monomorphize(&mut prog).unwrap_or_else(|d| fail(d));
    ast::resolve_enum_types(&mut prog);
    check::check_structs(&prog).unwrap_or_else(|d| fail(d));
    ast::register_structs(prog.structs.clone());
    ast::register_enums(prog.enums.clone());
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
