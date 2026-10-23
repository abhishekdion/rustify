//! rustify — a Python-to-Rust transpiler for statically-typed hot-path code.
//!
//! Usage:
//!   rustify build --root <project-dir> --entry <file.py> --out <out-dir>
//!   rustify check <file.py>
//!   rustify scan  --root <project-dir>

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rustify_core::ast::*;
use rustify_core::codegen::Codegen;
use rustify_core::infer;
use rustify_core::parser;
use rustify_core::prelude;
use rustify_core::types::*;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(2);
    }
    let cmd = args[1].clone();
    let result = match cmd.as_str() {
        "build" => cmd_build(&args[2..]),
        "check" => cmd_check(&args[2..]),
        "scan" => cmd_scan(&args[2..]),
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        other => Err(format!("unknown command `{}`", other)),
    };
    if let Err(e) = result {
        eprintln!("rustify: {}", e);
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!(
        "rustify — Python to Rust transpiler

USAGE:
    rustify build --root <dir> --entry <file.py> --out <dir> [--strict]
    rustify check <file.py>
    rustify scan --root <dir>

The transpiler converts the Python subset it can type confidently and reports
everything it could not convert instead of mistranslating it."
    );
}

fn opt(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        if let Some(rest) = args[i].strip_prefix(&format!("{}=", name)) {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Project discovery
// ---------------------------------------------------------------------------

/// A Python source file together with its dotted module name.
struct SourceFile {
    path: PathBuf,
    module: String,
    package: String,
}

fn module_name_for(root: &Path, path: &Path) -> (String, String) {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if comps.is_empty() {
        return (String::new(), String::new());
    }
    let file_name = &comps[comps.len() - 1];
    let is_init = file_name == "__init__.py";
    let dir_parts = &comps[..comps.len() - 1];
    let mut parts: Vec<String> = dir_parts.to_vec();
    if !is_init {
        parts.push(file_name.trim_end_matches(".py").to_string());
    }
    let module = parts.join(".");
    let package = if is_init {
        module.clone()
    } else {
        dir_parts.join(".")
    };
    (module, package)
}

fn collect_sources(root: &Path) -> Vec<SourceFile> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if name == "__pycache__"
                    || name == ".git"
                    || name == "node_modules"
                    || name == ".venv"
                    || name == "venv"
                    || name == ".mypy_cache"
                    || name == ".pytest_cache"
                    || name == "dist"
                    || name == "build"
                {
                    continue;
                }
                stack.push(p);
            } else if name.ends_with(".py") {
                let (module, package) = module_name_for(root, &p);
                out.push(SourceFile {
                    path: p,
                    module,
                    package,
                });
            }
        }
    }
    out.sort_by(|a, b| a.module.cmp(&b.module));
    out
}

/// Resolve a relative `from ..x import y` to an absolute dotted module name.
fn resolve_relative(package: &str, level: usize, name: &str) -> String {
    let mut parts: Vec<&str> = if package.is_empty() {
        Vec::new()
    } else {
        package.split('.').collect()
    };
    // level 1 == the current package, level 2 == its parent, ...
    for _ in 1..level {
        if !parts.is_empty() {
            parts.pop();
        }
    }
    if !name.is_empty() {
        parts.push(name);
    }
    parts.join(".")
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

fn cmd_build(args: &[String]) -> Result<(), String> {
    let root = opt(args, "--root").ok_or("--root <dir> is required")?;
    let entry = opt(args, "--entry").ok_or("--entry <file.py> is required")?;
    let out = opt(args, "--out").ok_or("--out <dir> is required")?;
    let root = PathBuf::from(root);
    let entry_path = root.join(&entry);
    if !entry_path.exists() {
        return Err(format!("entry file not found: {}", entry_path.display()));
    }
    let out_dir = PathBuf::from(out);

    let sources = collect_sources(&root);
    let by_module: HashMap<String, usize> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.module.clone(), i))
        .collect();

    // Resolve the entry module.
    let entry_src = match sources.iter().find(|s| s.path == entry_path) {
        Some(s) => s,
        None => return Err(format!("entry file is not under the project root: {}", entry)),
    };
    let entry_module = entry_src.module.clone();

    // Walk the import graph from the entry point.
    let mut selected: Vec<usize> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue = vec![entry_module.clone()];
    while let Some(m) = queue.pop() {
        if !seen.insert(m.clone()) {
            continue;
        }
        let idx = match by_module.get(&m) {
            Some(i) => *i,
            None => continue,
        };
        selected.push(idx);
        let src = &sources[idx];
        let parsed = match parse_file(&src.path) {
            Ok((module_ast, diags)) => {
                report_diags(&src.path, &diags);
                module_ast
            }
            Err(e) => {
                eprintln!("rustify: cannot parse {}: {}", src.path.display(), e);
                continue;
            }
        };
        for imp in collect_imports(&parsed) {
            let resolved = if imp.level > 0 {
                resolve_relative(&src.package, imp.level, &imp.module)
            } else {
                imp.module.clone()
            };
            if by_module.contains_key(&resolved) && !seen.contains(&resolved) {
                queue.push(resolved);
            }
        }
    }
    selected.sort_unstable();
    selected.dedup();

    // Transpile each selected module.
    let out_src = out_dir.join("src");
    fs::create_dir_all(&out_src).map_err(|e| e.to_string())?;

    let mut module_names: HashMap<String, String> = HashMap::new();
    for &i in &selected {
        let m = &sources[i].module;
        let rust_mod = m.replace('.', "__");
        module_names.insert(m.clone(), format!("crate::{}", rust_mod));
    }

    // Parse every selected module and build a shared signature table, so that
    // cross-module references keep their inferred types.
    let mut shared = Env::new();
    let mut parsed: Vec<(usize, Module)> = Vec::new();
    for &i in &selected {
        let src = &sources[i];
        let (module_ast, diags) = match parse_file(&src.path) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("rustify: cannot parse {}: {}", src.path.display(), e);
                continue;
            }
        };
        report_diags(&src.path, &diags);
        parsed.push((i, module_ast));
    }

    // Order the modules so that dependencies are processed first: a subclass
    // needs its base class to be known before its fields can be flattened.
    let order = topo_order(&selected, &sources, &parsed);

    for &i in &order {
        let src = &sources[i];
        let module_ast = match parsed.iter().find(|(idx, _)| *idx == i) {
            Some((_, m)) => m,
            None => continue,
        };
        shared.scopes = vec![std::collections::HashMap::new()];
        infer::collect_signatures(&mut shared, module_ast);
        shared
            .module_scopes
            .insert(src.module.clone(), shared.scopes[0].clone());
    }
    infer::compute_raises(&mut shared);

    let mut generated: Vec<(String, String)> = Vec::new();
    let mut all_diags = 0usize;

    for &i in &order {
        let src = &sources[i];
        let rust_mod = src.module.replace('.', "__");
        let module_ast = match parsed.iter().find(|(idx, _)| *idx == i) {
            Some((_, m)) => m,
            None => continue,
        };
        let (code, diags) = transpile_module(src, module_ast, &shared, &module_names);
        all_diags += diags.len();
        report_diags(&src.path, &diags);
        generated.push((rust_mod, code));
    }

    // Write the files.
    for (rust_mod, code) in &generated {
        let p = out_src.join(format!("{}.rs", rust_mod));
        fs::write(&p, code).map_err(|e| e.to_string())?;
    }
    fs::write(out_src.join("py2rs_prelude.rs"), prelude::PRELUDE).map_err(|e| e.to_string())?;

    let mut lib = String::new();
    lib.push_str("//! Generated by rustify.\n\n");
    lib.push_str("pub mod py2rs_prelude;\n");
    for (rust_mod, _) in &generated {
        lib.push_str(&format!("pub mod {};\n", rust_mod));
    }
    fs::write(out_src.join("lib.rs"), lib).map_err(|e| e.to_string())?;

    fs::write(
        out_dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"dipulse_protocol\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n\n[[bin]]\nname = \"dipulse_protocol\"\npath = \"src/main.rs\"\n\n[profile.release]\nopt-level = 3\nlto = true\ncodegen-units = 1\n"
        ),
    )
    .map_err(|e| e.to_string())?;

    // Only write a placeholder main.rs if the project does not have one yet,
    // so a hand-written benchmark harness is never overwritten.
    let main_path = out_src.join("main.rs");
    if !main_path.exists() {
        let main_rs = "fn main() {\n    println!(\"rustify: build succeeded — see src/lib.rs\");\n}\n";
        fs::write(&main_path, main_rs).map_err(|e| e.to_string())?;
    }

    println!(
        "rustify: transpiled {} module(s) into {}",
        generated.len(),
        out_dir.display()
    );
    if all_diags > 0 {
        println!("rustify: {} diagnostic(s) reported above", all_diags);
    }
    Ok(())
}

fn report_diags(path: &Path, diags: &[Diagnostic]) {
    for d in diags {
        if d.line > 0 {
            eprintln!("{}:{}: {}", path.display(), d.line, d.message);
        } else {
            eprintln!("{}: {}", path.display(), d.message);
        }
    }
}

// ---------------------------------------------------------------------------
// Per-file transpilation
// ---------------------------------------------------------------------------

/// Order modules so that a module's dependencies come first.
fn topo_order(
    selected: &[usize],
    sources: &[SourceFile],
    parsed: &[(usize, Module)],
) -> Vec<usize> {
    let mut deps: HashMap<usize, Vec<usize>> = HashMap::new();
    let by_module: HashMap<&str, usize> = sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.module.as_str(), i))
        .collect();
    for (i, m) in parsed {
        let src = &sources[*i];
        let mut list = Vec::new();
        for stmt in &m.body {
            if let Stmt::ImportFrom {
                module, level, ..
            } = stmt
            {
                let resolved = if *level > 0 {
                    resolve_relative(&src.package, *level, module)
                } else {
                    module.clone()
                };
                if let Some(&j) = by_module.get(resolved.as_str()) {
                    if j != *i && !list.contains(&j) {
                        list.push(j);
                    }
                }
            }
        }
        deps.insert(*i, list);
    }

    let mut order = Vec::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut stack: HashSet<usize> = HashSet::new();
    fn visit(
        i: usize,
        deps: &HashMap<usize, Vec<usize>>,
        seen: &mut HashSet<usize>,
        stack: &mut HashSet<usize>,
        order: &mut Vec<usize>,
    ) {
        if seen.contains(&i) || stack.contains(&i) {
            return;
        }
        stack.insert(i);
        if let Some(list) = deps.get(&i) {
            for d in list.clone() {
                visit(d, deps, seen, stack, order);
            }
        }
        stack.remove(&i);
        seen.insert(i);
        order.push(i);
    }
    for &i in selected {
        visit(i, &deps, &mut seen, &mut stack, &mut order);
    }
    order
}

/// Transpile one module using the shared signature table.
fn transpile_module(
    src: &SourceFile,
    module_ast: &Module,
    shared: &Env,
    module_names: &HashMap<String, String>,
) -> (String, Vec<Diagnostic>) {
    let mut cg = Codegen::new(shared.fork());
    cg.project_modules = module_names.clone();
    // Record where imported symbols come from so cross-module references are
    // qualified with their defining module and keep their inferred type.
    for stmt in &module_ast.body {
        if let Stmt::ImportFrom {
            module, names, level, ..
        } = stmt
        {
            let resolved = if *level > 0 {
                resolve_relative(&src.package, *level, module)
            } else {
                module.clone()
            };
            let rust_mod = match module_names.get(&resolved) {
                Some(m) => m.clone(),
                None => {
                    if std::env::var("RUSTIFY_DEBUG").is_ok() {
                        eprintln!(
                            "[rustify] module `{}` imports `{}` -> unresolved `{}`",
                            src.module, module, resolved
                        );
                    }
                    continue;
                }
            };
            let scope = shared.module_scopes.get(&resolved);
            for (n, asn) in names {
                // `from x import *` cannot be expressed as a named import.
                if n == "*" {
                    continue;
                }
                let local = asn.clone().unwrap_or_else(|| n.clone());
                let ty = scope
                    .and_then(|s| s.get(n).cloned())
                    .or_else(|| shared.funcs.get(n).map(|f| f.ret.clone()))
                    .unwrap_or(Ty::Unknown);
                cg.import_origins.insert(local, (rust_mod.clone(), ty));
            }
        }
    }
    cg.generate(module_ast)
}

fn parse_file(path: &Path) -> Result<(Module, Vec<Diagnostic>), String> {
    let src = fs::read_to_string(path).map_err(|e| e.to_string())?;
    match parser::Parser::parse_module(&src) {
        Ok(m) => Ok((m, Vec::new())),
        Err(e) => Err(format!("parse error: {}", e)),
    }
}

struct ImportRef {
    module: String,
    level: usize,
}

fn collect_imports(m: &Module) -> Vec<ImportRef> {
    let mut out = Vec::new();
    for s in &m.body {
        if let Stmt::ImportFrom {
            module, level, ..
        } = s
        {
            out.push(ImportRef {
                module: module.clone(),
                level: *level,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Check / scan
// ---------------------------------------------------------------------------

fn cmd_check(args: &[String]) -> Result<(), String> {
    let file = args.first().ok_or("check requires a file path")?;
    let path = PathBuf::from(file);
    match parse_file(&path) {
        Ok((m, _)) => {
            println!("{}: parsed OK ({} top-level statements)", path.display(), m.body.len());
            Ok(())
        }
        Err(e) => Err(format!("{}: {}", path.display(), e)),
    }
}

fn cmd_scan(args: &[String]) -> Result<(), String> {
    let root = opt(args, "--root").ok_or("--root <dir> is required")?;
    let root = PathBuf::from(root);
    let sources = collect_sources(&root);
    let mut ok = 0usize;
    let mut failed = 0usize;
    for s in &sources {
        match parse_file(&s.path) {
            Ok(_) => ok += 1,
            Err(e) => {
                failed += 1;
                eprintln!("{}: {}", s.path.display(), e);
            }
        }
    }
    println!("scan: {} parsed, {} failed", ok, failed);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Transpile a Python snippet and return the generated Rust.
    fn transpile(src: &str) -> (String, Vec<Diagnostic>) {
        let module = parser::Parser::parse_module(src).expect("parse failed");
        let mut env = Env::new();
        infer::collect_signatures(&mut env, &module);
        infer::compute_raises(&mut env);
        let cg = Codegen::new(env);
        cg.generate(&module)
    }

    #[test]
    fn parses_and_types_a_function() {
        let (code, diags) = transpile(
            "def add(a: int, b: int) -> int:\n    return a + b\n",
        );
        assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
        assert!(code.contains("pub fn add(a: i64, b: i64) -> i64"), "{}", code);
        assert!(code.contains("return ((a) + (b));"), "{}", code);
    }

    #[test]
    fn default_arguments_are_filled_at_the_call_site() {
        let (code, _) = transpile(
            "def f(a: int, b: int = 7) -> int:\n    return a\n\nx = f(1)\n",
        );
        assert!(code.contains("f(1, 7)"), "{}", code);
    }

    #[test]
    fn bytes_become_slices_and_sums() {
        let (code, diags) = transpile(
            "def total(data: bytes) -> int:\n    return sum(data[:-4]) & 0xFFFFFFFF\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("data: &[u8]"), "{}", code);
        assert!(code.contains("py_sum_bytes"), "{}", code);
    }

    #[test]
    fn raising_functions_become_results() {
        let (code, diags) = transpile(
            "def f(x: int) -> int:\n    if x < 0:\n        raise ValueError(\"negative\")\n    return x\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("-> Result<i64, PyErr>"), "{}", code);
        assert!(code.contains("Err(PyErr::new(\"ValueError\""), "{}", code);
    }

    #[test]
    fn dataclasses_become_structs() {
        let (code, diags) = transpile(
            "from dataclasses import dataclass, field\n\n@dataclass\nclass P:\n    name: str = \"\"\n    cells: list[int] = field(default_factory=list)\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("pub struct P {"), "{}", code);
        assert!(code.contains("pub name: String,"), "{}", code);
        assert!(code.contains("pub cells: Vec<i64>,"), "{}", code);
    }

    #[test]
    fn locals_are_hoisted_to_function_scope() {
        let (code, _) = transpile(
            "def f(a: bool) -> int:\n    if a:\n        x = 1\n    else:\n        x = 2\n    return x\n",
        );
        assert!(code.contains("let mut x: i64 = 0;"), "{}", code);
    }

    #[test]
    fn comprehensions_become_loops() {
        let (code, diags) = transpile(
            "def f(n: int) -> list[int]:\n    return [i * 2 for i in range(n)]\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("for i in (0..n)"), "{}", code);
    }

    #[test]
    fn fstrings_become_string_builders() {
        let (code, diags) = transpile(
            "def f(x: int) -> str:\n    return f\"value {x:04X} end\"\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("push_str"), "{}", code);
        assert!(code.contains("py_format"), "{}", code);
    }

    #[test]
    fn unsupported_constructs_are_reported_not_mistranslated() {
        let (_, diags) = transpile(
            "def f(x: int) -> int:\n    y = lambda a: a\n    return x\n",
        );
        assert!(
            diags.iter().any(|d| d.message.contains("unsupported")),
            "expected a diagnostic, got {:?}",
            diags
        );
    }

    #[test]
    fn relative_imports_resolve_to_absolute_modules() {
        assert_eq!(
            resolve_relative("a.b.c", 1, "frames"),
            "a.b.c.frames"
        );
        assert_eq!(resolve_relative("a.b.c", 2, "models"), "a.b.models");
        assert_eq!(resolve_relative("a.b.c", 3, "base"), "a.base");
    }

    #[test]
    fn match_statement_becomes_a_rust_match() {
        let (code, diags) = transpile(
            "def f(x: int) -> str:\n    match x:\n        case 1:\n            return \"one\"\n        case 2 | 3:\n            return \"few\"\n        case _:\n            return \"many\"\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("match "), "{}", code);
        assert!(code.contains("1 => {"), "{}", code);
        assert!(code.contains("2 | 3 => {"), "{}", code);
        assert!(code.contains("_ => {"), "{}", code);
    }

    #[test]
    fn non_exhaustive_match_gets_a_fallback_arm() {
        let (code, diags) = transpile(
            "def f(x: int) -> str:\n    match x:\n        case 1:\n            return \"one\"\n    return \"done\"\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("1 => {"), "{}", code);
        assert!(code.contains("_ => {}"), "{}", code);
        assert!(code.contains("\"done\""), "{}", code);
    }

    #[test]
    fn match_guards_use_captured_values() {
        let (code, diags) = transpile(
            "def f(x: int) -> str:\n    match x:\n        case n if n > 10:\n            return \"big\"\n        case _:\n            return \"small\"\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("if ("), "{}", code);
        assert!(code.contains("\"big\""), "{}", code);
    }

    #[test]
    fn match_tuple_patterns_unpack() {
        let (code, diags) = transpile(
            "def f(p: tuple[int, int]) -> int:\n    match p:\n        case (0, y):\n            return y\n        case _:\n            return 0\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("(0, y)"), "{}", code);
    }

    #[test]
    fn match_dotted_constants_are_pattern_usable() {
        let (code, diags) = transpile(
            "class Kind:\n    A = 1\n    B = 2\n\ndef f(x: int) -> str:\n    match x:\n        case Kind.A:\n            return \"a\"\n        case _:\n            return \"b\"\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("Kind::A => {"), "{}", code);
    }

    #[test]
    fn match_as_a_variable_name_is_still_parsed() {
        let (code, diags) = transpile(
            "def f(x: int) -> int:\n    match = x + 1\n    return match\n",
        );
        assert!(diags.is_empty(), "{:?}", diags);
        assert!(code.contains("match_"), "{}", code);
        assert!(code.contains("(x) + (1)"), "{}", code);
    }

    #[test]
    fn unsupported_patterns_are_reported_not_mistranslated() {
        let (_, diags) = transpile(
            "def f(x: int) -> int:\n    match x:\n        case [1, 2]:\n            return 1\n        case _:\n            return 0\n",
        );
    assert!(diags.iter().any(|d| d.message.contains("unsupported")),
            "expected a diagnostic, got {:?}", diags);
    }
}
