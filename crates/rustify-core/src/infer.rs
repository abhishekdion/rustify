//! Type inference over the Python AST.
//!
//! `infer_expr` is a *pure* function of the environment: it never mutates it,
//! so the code generator can call it freely while emitting Rust.

use crate::ast::*;
use crate::types::*;

/// Unification for the elements of a literal: an unknown element makes the
/// whole literal's element type unknown.
pub fn unify_literal(a: &Ty, b: &Ty) -> Ty {
    if a.is_unknown() || b.is_unknown() {
        return Ty::Unknown;
    }
    unify(a, b)
}

/// Infer the type of an expression.
pub fn infer_expr(env: &Env, e: &Expr) -> Ty {
    match e {
        Expr::Num(NumVal::Int(_)) => Ty::Int,
        Expr::Num(NumVal::Float(_)) => Ty::Float,
        Expr::Str { kind, .. } => match kind {
            StrKind::Str => Ty::Str,
            StrKind::Bytes => Ty::Bytes,
        },
        Expr::FStr { kind, .. } => match kind {
            StrKind::Str => Ty::Str,
            StrKind::Bytes => Ty::Bytes,
        },
        Expr::Name(n) => match n.as_str() {
            "True" | "False" => Ty::Bool,
            "None" => Ty::NoneT,
            _ => env.lookup(n).unwrap_or(Ty::Unknown),
        },
        Expr::Attribute { value, attr } => {
            let base = infer_expr(env, value);
            attribute_type(env, &base, attr)
        }
        Expr::Subscript { value, index } => {
            let base = infer_expr(env, value);
            if index.len() != 1 {
                return Ty::Unknown;
            }
            match &index[0] {
                IndexItem::Single(ix) => {
                    // Tuple indexing with a literal integer.
                    if let (Ty::Tuple(items), Expr::Num(NumVal::Int(i))) = (&base, ix) {
                        if *i >= 0 && (*i as usize) < items.len() {
                            return items[*i as usize].clone();
                        }
                    }
                    subscript_type(&base)
                }
                IndexItem::Slice { .. } => base.clone(),
            }
        }
        Expr::Call { func, args } => call_type(env, func, args),
        Expr::BinOp { op, left, right } => {
            let l = infer_expr(env, left);
            let r = infer_expr(env, right);
            binop_type(op, &l, &r)
        }
        Expr::UnaryOp { op, operand } => match op {
            UnOp::Not => Ty::Bool,
            UnOp::USub | UnOp::UAdd => {
                let t = infer_expr(env, operand);
                match t {
                    Ty::Float => Ty::Float,
                    Ty::Bool => Ty::Int,
                    other => other,
                }
            }
            UnOp::Invert => Ty::Int,
        },
        Expr::BoolOp { .. } => Ty::Bool,
        Expr::Compare { .. } => Ty::Bool,
        Expr::IfExp { body, orelse, .. } => {
            unify(&infer_expr(env, body), &infer_expr(env, orelse))
        }
        Expr::List(items) => {
            let mut acc: Option<Ty> = None;
            for it in items {
                let t = infer_expr(env, it);
                acc = Some(match acc {
                    None => t,
                    Some(a) => unify_literal(&a, &t),
                });
            }
            Ty::List(Box::new(acc.unwrap_or(Ty::Unknown)))
        }
        Expr::Tuple(items) => {
            if items.is_empty() {
                Ty::Tuple(vec![])
            } else {
                Ty::Tuple(items.iter().map(|i| infer_expr(env, i)).collect())
            }
        }
        Expr::Dict(entries) => {
            let mut kt: Option<Ty> = None;
            let mut vt: Option<Ty> = None;
            for (k, v) in entries {
                if let Some(k) = k {
                    let t = infer_expr(env, k);
                    kt = Some(match kt {
                        None => t,
                        Some(a) => unify_literal(&a, &t),
                    });
                }
                let t = infer_expr(env, v);
                vt = Some(match vt {
                    None => t,
                    Some(a) => unify_literal(&a, &t),
                });
            }
            Ty::Dict(
                Box::new(kt.unwrap_or(Ty::Unknown)),
                Box::new(vt.unwrap_or(Ty::Unknown)),
            )
        }
        Expr::Set(items) => {
            let mut acc: Option<Ty> = None;
            for it in items {
                let t = infer_expr(env, it);
                acc = Some(match acc {
                    None => t,
                    Some(a) => unify_literal(&a, &t),
                });
            }
            Ty::Set(Box::new(acc.unwrap_or(Ty::Unknown)))
        }
        Expr::ListComp { elt, .. } => Ty::List(Box::new(infer_expr(env, elt))),
        Expr::SetComp { elt, .. } => Ty::Set(Box::new(infer_expr(env, elt))),
        Expr::DictComp { key, value, .. } => Ty::Dict(
            Box::new(infer_expr(env, key)),
            Box::new(infer_expr(env, value)),
        ),
        Expr::GeneratorExp { elt, .. } => infer_expr(env, elt),
        Expr::Lambda { params, body } => {
            // Infer the body with the parameters in scope. Python lambdas
            // cannot have annotated parameters, so parameter types come from
            // the binding annotation (`Callable[[..], ..]`) at codegen time.
            let mut scopes = env.scopes.clone();
            scopes.push(
                params
                    .iter()
                    .map(|p| (p.name.clone(), Ty::Unknown))
                    .collect(),
            );
            let scoped = Env {
                scopes,
                funcs: env.funcs.clone(),
                classes: env.classes.clone(),
                diagnostics: Vec::new(),
                module_vars: env.module_vars.clone(),
                func_bodies: env.func_bodies.clone(),
                module_scopes: env.module_scopes.clone(),
            };
            let ret = infer_expr(&scoped, body);
            Ty::Lambda {
                params: params.iter().map(|_| Ty::Unknown).collect(),
                ret: Box::new(ret),
                raises: false,
            }
        }
        Expr::Starred(inner) => infer_expr(env, inner),
    }
}

fn subscript_type(base: &Ty) -> Ty {
    match base {
        Ty::List(t) => (**t).clone(),
        Ty::Set(t) => (**t).clone(),
        Ty::Dict(_, v) => (**v).clone(),
        Ty::Tuple(items) => {
            if items.len() == 1 {
                items[0].clone()
            } else {
                Ty::Unknown
            }
        }
        Ty::Bytes => Ty::Int,
        Ty::Str => Ty::Str,
        Ty::Optional(t) => t.inner(),
        _ => Ty::Unknown,
    }
}

fn attribute_type(env: &Env, base: &Ty, attr: &str) -> Ty {
    match base {
        Ty::Named(cls) => {
            if let Some(info) = env.classes.get(cls) {
                if let Some((_, t)) = info.fields.iter().find(|(n, _)| n == attr) {
                    return t.clone();
                }
                if let Some(_sig) = info.methods.get(attr) {
                    return Ty::Named(format!("__method__::{}::{}", cls, attr));
                }
            }
            Ty::Unknown
        }
        Ty::Str => match attr {
            "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "join" | "format"
            | "title" | "capitalize" | "swapcase" | "zfill" | "removeprefix" | "removesuffix" => {
                Ty::Named("__method__::str".into())
            }
            "split" | "rsplit" | "splitlines" => Ty::Named("__method__::str".into()),
            "encode" => Ty::Named("__method__::str".into()),
            "startswith" | "endswith" | "isdigit" | "isalpha" | "isspace" | "isupper"
            | "islower" | "isnumeric" | "isdecimal" => Ty::Named("__method__::str".into()),
            "find" | "rfind" | "index" | "count" | "len" => Ty::Int,
            _ => Ty::Unknown,
        },
        Ty::Bytes => match attr {
            "hex" | "decode" => Ty::Named("__method__::bytes".into()),
            _ => Ty::Unknown,
        },
        Ty::List(_) => match attr {
            "append" | "extend" | "insert" | "clear" | "sort" | "reverse" => {
                Ty::Named("__method__::list".into())
            }
            "pop" | "index" | "count" => Ty::Unknown,
            "copy" => base.clone(),
            _ => Ty::Unknown,
        },
        Ty::Dict(_, v) => match attr {
            "get" | "pop" | "setdefault" => (**v).clone(),
            "keys" => Ty::Named("__method__::dict_keys".into()),
            "values" => Ty::Named("__method__::dict_values".into()),
            "items" => Ty::Named("__method__::dict_items".into()),
            "update" | "clear" => Ty::Named("__method__::dict".into()),
            _ => Ty::Unknown,
        },
        _ => Ty::Unknown,
    }
}

pub fn binop_type(op: &BinOp, l: &Ty, r: &Ty) -> Ty {
    use BinOp::*;
    match op {
        Add => match (l, r) {
            (Ty::Str, Ty::Str) => Ty::Str,
            (Ty::Bytes, Ty::Bytes) => Ty::Bytes,
            (Ty::List(a), Ty::List(b)) => Ty::List(Box::new(unify(a, b))),
            (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Int) | (Ty::Int, Ty::Bool) | (Ty::Bool, Ty::Int) => Ty::Int,
            (Ty::Bool, Ty::Bool) => Ty::Int,
            _ => Ty::Unknown,
        },
        Sub => match (l, r) {
            (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Int) | (Ty::Int, Ty::Bool) | (Ty::Bool, Ty::Int) => Ty::Int,
            _ => Ty::Unknown,
        },
        Mul => match (l, r) {
            (Ty::Str, Ty::Int) | (Ty::Int, Ty::Str) => Ty::Str,
            (Ty::Bytes, Ty::Int) | (Ty::Int, Ty::Bytes) => Ty::Bytes,
            (Ty::List(a), Ty::Int) => Ty::List(a.clone()),
            (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Int) => Ty::Int,
            _ => Ty::Unknown,
        },
        Div => Ty::Float,
        FloorDiv => match (l, r) {
            (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Int) => Ty::Int,
            _ => Ty::Unknown,
        },
        Mod => match (l, r) {
            (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Int) => Ty::Int,
            _ => Ty::Unknown,
        },
        Pow => match (l, r) {
            (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Int) => Ty::Int,
            _ => Ty::Unknown,
        },
        LShift | RShift | BitOr | BitAnd | BitXor => Ty::Int,
    }
}

/// Return type of a call expression.
pub fn call_type(env: &Env, func: &Expr, args: &[Arg]) -> Ty {
    match func {
        Expr::Name(name) => {
            if let Some(sig) = env.funcs.get(name) {
                return sig.ret.clone();
            }
            if let Some(info) = env.classes.get(name) {
                return Ty::Named(info.name.clone());
            }
            builtin_call(env, name, args).unwrap_or(Ty::Unknown)
        }
        Expr::Attribute { value, attr } => {
            // Module-qualified calls we understand.
            if let Expr::Name(mod_name) = &**value {
                match (mod_name.as_str(), attr.as_str()) {
                    ("struct", "unpack") => return struct_unpack_type(args),
                    ("struct", "pack") => return Ty::Bytes,
                    ("datetime", "now") => return Ty::Str,
                    ("datetime", "utcnow") => return Ty::Str,
                    ("math", "sqrt") | ("math", "floor") | ("math", "ceil") => return Ty::Float,
                    ("logging", "getLogger") => return Ty::Named("__logger__".into()),
                    ("os", "getpid") => return Ty::Int,
                    _ => {}
                }
            }
            let recv = infer_expr(env, value);
            method_call(env, &recv, attr, args)
        }
        _ => Ty::Unknown,
    }
}

fn struct_unpack_type(args: &[Arg]) -> Ty {
    // struct.unpack(">H", buf) etc.
    let fmt = args.iter().find_map(|a| match a {
        Arg::Pos(Expr::Str { value, .. }) => Some(value.clone()),
        _ => None,
    });
    let fmt = match fmt {
        Some(f) => f,
        None => return Ty::Unknown,
    };
    let mut types = Vec::new();
    for ch in fmt.chars().skip(1) {
        types.push(match ch {
            'f' | 'd' => Ty::Float,
            _ => Ty::Int,
        });
    }
    match types.len() {
        0 => Ty::Unknown,
        1 => types[0].clone(),
        _ => Ty::Tuple(types),
    }
}

fn builtin_call(env: &Env, name: &str, args: &[Arg]) -> Option<Ty> {
    let first_ty = || args.iter().find_map(|a| match a {
        Arg::Pos(e) => Some(infer_expr(env, e)),
        _ => None,
    });
    Some(match name {
        "len" => Ty::Int,
        "int" => Ty::Int,
        "float" => Ty::Float,
        "str" => Ty::Str,
        "bool" => Ty::Bool,
        "bytes" | "bytearray" => Ty::Bytes,
        "list" => Ty::List(Box::new(first_ty().unwrap_or(Ty::Unknown))),
        "dict" => Ty::Dict(Box::new(Ty::Unknown), Box::new(Ty::Unknown)),
        "tuple" => Ty::Tuple(vec![]),
        "set" => Ty::Set(Box::new(Ty::Unknown)),
        "sum" => match first_ty() {
            Some(Ty::List(t)) => (*t).clone(),
            Some(Ty::Bytes) => Ty::Int,
            _ => Ty::Int,
        },
        "min" | "max" => match first_ty() {
            Some(Ty::List(t)) => (*t).clone(),
            Some(Ty::Bytes) => Ty::Int,
            Some(Ty::Str) => Ty::Str,
            other => other.unwrap_or(Ty::Unknown),
        },
        "abs" => match first_ty() {
            Some(Ty::Float) => Ty::Float,
            Some(Ty::Int) => Ty::Int,
            other => other.unwrap_or(Ty::Unknown),
        },
        "round" => Ty::Float,
        "ord" => Ty::Int,
        "chr" => Ty::Str,
        "hex" | "bin" | "oct" | "repr" | "format" | "ascii" => Ty::Str,
        "range" => Ty::List(Box::new(Ty::Int)),
        "enumerate" => Ty::List(Box::new(Ty::Tuple(vec![Ty::Int, Ty::Unknown]))),
        "zip" => Ty::List(Box::new(Ty::Tuple(vec![]))),
        "reversed" => Ty::List(Box::new(first_ty().unwrap_or(Ty::Unknown))),
        "sorted" => Ty::List(Box::new(match first_ty() {
            Some(Ty::List(t)) => (*t).clone(),
            Some(Ty::Str) => Ty::Str,
            other => other.unwrap_or(Ty::Unknown),
        })),
        "any" | "all" => Ty::Bool,
        "isinstance" | "issubclass" | "callable" => Ty::Bool,
        "hasattr" | "getattr" => Ty::Unknown,
        "input" => Ty::Str,
        "id" => Ty::Int,
        "hash" => Ty::Int,
        "divmod" => Ty::Tuple(vec![Ty::Int, Ty::Int]),
        "pow" => Ty::Int,
        "chr_" => Ty::Str,
        _ => return None,
    })
}

fn method_call(env: &Env, recv: &Ty, method: &str, args: &[Arg]) -> Ty {
    let _arg_ty = |i: usize| -> Ty {
        args.iter()
            .filter_map(|a| match a {
                Arg::Pos(e) => Some(e),
                _ => None,
            })
            .nth(i)
            .map(|e| infer_expr(env, e))
            .unwrap_or(Ty::Unknown)
    };
    match recv {
        Ty::Named(cls) => {
            if let Some(sig) = env.classes.get(cls).and_then(|c| c.methods.get(method)) {
                return sig.ret.clone();
            }
            Ty::Unknown
        }
        Ty::Str => match method {
            "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace" | "title"
            | "capitalize" | "swapcase" | "zfill" | "removeprefix" | "removesuffix" => Ty::Str,
            "join" => Ty::Str,
            "split" | "rsplit" | "splitlines" => Ty::List(Box::new(Ty::Str)),
            "format" => Ty::Str,
            "encode" => Ty::Bytes,
            "startswith" | "endswith" | "isdigit" | "isalpha" | "isspace" | "isnumeric" => Ty::Bool,
            "find" | "rfind" | "index" | "count" => Ty::Int,
            _ => Ty::Unknown,
        },
        Ty::Bytes => match method {
            "hex" => Ty::Str,
            "decode" => Ty::Str,
            _ => Ty::Unknown,
        },
        Ty::List(t) => match method {
            "append" | "extend" | "insert" | "clear" | "sort" | "reverse" => Ty::NoneT,
            "pop" => (**t).clone(),
            "index" | "count" => Ty::Int,
            "copy" => recv.clone(),
            _ => Ty::Unknown,
        },
        Ty::Dict(_, v) => match method {
            "get" | "pop" | "setdefault" => (**v).clone(),
            "keys" => Ty::List(Box::new(Ty::Unknown)),
            "values" => Ty::List(Box::new((**v).clone())),
            "items" => Ty::List(Box::new(Ty::Tuple(vec![Ty::Unknown, (**v).clone()]))),
            "update" | "clear" => Ty::NoneT,
            _ => Ty::Unknown,
        },
        _ => Ty::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Signature collection (pass 1)
// ---------------------------------------------------------------------------

/// Collect top-level function and class signatures from a module.
pub fn collect_signatures(env: &mut Env, module: &Module) {
    for stmt in &module.body {
        match stmt {
            Stmt::FuncDef {
                name,
                params,
                returns,
                body,
                ..
            } => {
                let mut sig_params = Vec::new();
                let mut positional_only = 0usize;
                for p in params {
                    if p.name == "*" || p.name == "/" {
                        if p.name == "/" {
                            positional_only = sig_params.len();
                        }
                        continue;
                    }
                    let base = p.name.trim_start_matches('*').to_string();
                    let t = match &p.annotation {
                        Some(a) => ty_from_annotation(a),
                        None => Ty::Unknown,
                    };
                    sig_params.push((base, t));
                }
                let ret = match returns {
                    Some(r) => ty_from_annotation(r),
                    None => Ty::Unknown,
                };
                let defaults = params
                    .iter()
                    .map(|p| p.default.as_ref().and_then(default_rust_src))
                    .collect();
                let mut_params = collect_mutated_params(body, &sig_params);
                env.funcs.insert(
                    name.clone(),
                    FuncSig {
                        name: name.clone(),
                        params: sig_params,
                        ret,
                        raises: false,
                        positional_only,
                        is_method: false,
                        defaults,
                        mut_params,
                    },
                );
                env.func_bodies.insert(name.clone(), body.clone());
            }
            Stmt::ClassDef {
                name,
                bases,
                body,
                decorators,
            } => {
                let mut info = ClassInfo {
                    name: name.clone(),
                    is_dataclass: decorators.iter().any(|d| decorator_is(d, "dataclass")),
                    ..Default::default()
                };
                for b in bases {
                    if let Expr::Name(n) = b {
                        info.bases.push(n.clone());
                    }
                }
                // Fields: annotated assignments at class level.
                for s in body {
                    if let Stmt::AnnAssign {
                        target,
                        annotation,
                        value,
                    } = s
                    {
                        if let Expr::Name(n) = target {
                            info.fields.push((n.clone(), ty_from_annotation(annotation)));
                            if let Some(v) = value {
                                info.field_defaults
                                    .insert(n.clone(), default_expr_source(v));
                            }
                        }
                    }
                }
                // Methods.
                for s in body {
                    if let Stmt::FuncDef {
                        name, params, returns, body: mbody, ..
                    } = s
                    {
                        let mut sig_params = Vec::new();
                        let mut positional_only = 0usize;
                        for p in params {
                            if p.name == "*" || p.name == "/" {
                                if p.name == "/" {
                                    positional_only = sig_params.len();
                                }
                                continue;
                            }
                            let base = p.name.trim_start_matches('*').to_string();
                            let t = match &p.annotation {
                                Some(a) => ty_from_annotation(a),
                                None => Ty::Unknown,
                            };
                            sig_params.push((base, t));
                        }
                        let ret = match returns {
                            Some(r) => ty_from_annotation(r),
                            None => Ty::Unknown,
                        };
                        let defaults = params
                            .iter()
                            .map(|p| p.default.as_ref().and_then(default_rust_src))
                            .collect();
                        let mut_params = collect_mutated_params(mbody, &sig_params);
                        info.methods.insert(
                            name.clone(),
                            FuncSig {
                                name: name.clone(),
                                params: sig_params,
                                ret,
                                raises: false,
                                positional_only,
                                is_method: true,
                                defaults,
                                mut_params: mut_params.clone(),
                            },
                        );
                        info.method_bodies.insert(name.clone(), mbody.clone());
                        info.method_params.insert(name.clone(), params.clone());
                        info.method_mut_params.insert(name.clone(), mut_params);
                        if has_property_decorator(s) {
                            info.properties.insert(name.clone());
                        }
                    }
                }
                env.classes.insert(name.clone(), info);
            }
            _ => {}
        }
    }
    // Resolve inheritance: copy base class fields into the subclass.
    let class_names: Vec<String> = env.classes.keys().cloned().collect();
    for name in class_names {
        let bases = env.classes.get(&name).map(|c| c.bases.clone()).unwrap_or_default();
        for base in bases {
            let base_info = match env.classes.get(&base) {
                Some(b) => b.clone(),
                None => continue,
            };
            let child = env.classes.get_mut(&name).unwrap();
            let mut merged = base_info.fields.clone();
            merged.extend(child.fields.clone());
            child.fields = merged;
            for (m, sig) in base_info.methods.clone() {
                if child.methods.contains_key(&m) {
                    // The subclass overrides the base method: keep the base
                    // implementation available under `base_<name>` so that
                    // `super().m(...)` can call it.
                    let shadow = format!("base_{}", m);
                    if !child.methods.contains_key(&shadow) {
                        child.methods.insert(shadow.clone(), sig.clone());
                        if let Some(b) = base_info.method_bodies.get(&m) {
                            child.method_bodies.insert(shadow.clone(), b.clone());
                        }
                        if let Some(p) = base_info.method_params.get(&m) {
                            child.method_params.insert(shadow.clone(), p.clone());
                        }
                        if let Some(p) = base_info.method_mut_params.get(&m) {
                            child.method_mut_params.insert(shadow.clone(), p.clone());
                        }
                    }
                } else {
                    child.methods.entry(m.clone()).or_insert(sig.clone());
                    if let Some(b) = base_info.method_bodies.get(&m) {
                        child.method_bodies.entry(m.clone()).or_insert(b.clone());
                    }
                    if let Some(p) = base_info.method_params.get(&m) {
                        child.method_params.entry(m.clone()).or_insert(p.clone());
                    }
                    if let Some(p) = base_info.method_mut_params.get(&m) {
                        child.method_mut_params.entry(m.clone()).or_insert(p.clone());
                    }
                }
            }
        }
    }

    // Module-level bindings, resolved after the signature table is complete.
    for stmt in &module.body {
        match stmt {
            Stmt::Assign { targets, value } => {
                if let Some(Expr::Name(n)) = targets.first() {
                    let t = infer_expr(env, value);
                    env.set(n, t);
                }
            }
            Stmt::AnnAssign {
                target, annotation, ..
            } => {
                if let Expr::Name(n) = target {
                    env.set(n, ty_from_annotation(annotation));
                }
            }
            _ => {}
        }
    }
}

/// Reserved for decorator-driven inference passes.
#[allow(dead_code)]
fn has_decorator(body: &[Stmt], name: &str) -> bool {
    body.iter().any(|s| match s {
        Stmt::FuncDef { decorators, .. } | Stmt::ClassDef { decorators, .. } => decorators
            .iter()
            .any(|d| decorator_is(d, name)),
        _ => false,
    })
}

/// True when a decorator expression refers to `name`.
fn decorator_is(d: &Expr, name: &str) -> bool {
    match d {
        Expr::Name(n) => n == name,
        Expr::Attribute { attr, .. } => attr == name,
        Expr::Call { func, .. } => match &**func {
            Expr::Name(n) => n == name,
            Expr::Attribute { attr, .. } => attr == name,
            _ => false,
        },
        _ => false,
    }
}

/// True if the function is decorated with `@property`.
fn has_property_decorator(s: &Stmt) -> bool {
    match s {
        Stmt::FuncDef { decorators, .. } => decorators.iter().any(|d| match d {
            Expr::Name(n) => n == "property",
            Expr::Attribute { attr, .. } => attr == "property",
            Expr::Call { func, .. } => match &**func {
                Expr::Name(n) => n == "property",
                Expr::Attribute { attr, .. } => attr == "property",
                _ => false,
            },
            _ => false,
        }),
        _ => false,
    }
}

/// Render a default argument value as Rust source.
pub fn default_rust_src(e: &Expr) -> Option<String> {
    Some(match e {
        Expr::Num(NumVal::Int(v)) => format!("{}", v),
        Expr::Num(NumVal::Float(v)) => format!("{:?}", v),
        Expr::Str { value, kind } => match kind {
            StrKind::Str => format!("{:?}.to_string()", value),
            StrKind::Bytes => format!("b{:?}.to_vec()", value),
        },
        Expr::Name(n) => match n.as_str() {
            "True" => "true".into(),
            "False" => "false".into(),
            "None" => "None".into(),
            other => other.to_string(),
        },
        Expr::UnaryOp {
            op: UnOp::USub,
            operand,
        } => format!("-{}", default_rust_src(operand)?),
        Expr::UnaryOp {
            op: UnOp::UAdd,
            operand,
        } => default_rust_src(operand)?,
        Expr::List(items) => {
            let mut parts = Vec::new();
            for i in items {
                parts.push(default_rust_src(i)?);
            }
            format!("vec![{}]", parts.join(", "))
        }
        Expr::Tuple(items) => {
            let mut parts = Vec::new();
            for i in items {
                parts.push(default_rust_src(i)?);
            }
            format!("({},)", parts.join(", "))
        }
        Expr::Dict(entries) => {
            let mut parts = Vec::new();
            for (k, v) in entries {
                let k = k.as_ref()?;
                parts.push(format!(
                    "({}.into(), {}.into())",
                    default_rust_src(k)?,
                    default_rust_src(v)?
                ));
            }
            format!("HashMap::from([{}])", parts.join(", "))
        }
        Expr::Call { func, args } => {
            let fname = match &**func {
                Expr::Name(n) => n.clone(),
                _ => return None,
            };
            if fname == "bytes" || fname == "bytearray" {
                if let Some(Arg::Pos(Expr::List(items))) = args.first() {
                    let mut parts = Vec::new();
                    for i in items {
                        parts.push(format!("({}) as u8", default_rust_src(i)?));
                    }
                    return Some(format!("vec![{}]", parts.join(", ")));
                }
            }
            return None;
        }
        _ => return None,
    })
}

/// Render a module-level constant as Rust source, if it is compile-time
/// evaluable.
pub fn const_rust_src(e: &Expr) -> Option<String> {
    Some(match e {
        Expr::Num(NumVal::Int(v)) => format!("{}", v),
        Expr::Num(NumVal::Float(v)) => format!("{:?}", v),
        Expr::Str { value, kind } => match kind {
            StrKind::Str => format!("{:?}", value),
            StrKind::Bytes => format!("b{:?}", value),
        },
        Expr::Name(n) => match n.as_str() {
            "True" => "true".into(),
            "False" => "false".into(),
            "None" => "None".into(),
            other => other.to_string(),
        },
        Expr::UnaryOp {
            op: UnOp::USub,
            operand,
        } => format!("-{}", const_rust_src(operand)?),
        Expr::UnaryOp {
            op: UnOp::UAdd,
            operand,
        } => const_rust_src(operand)?,
        Expr::BinOp { op, left, right } => format!(
            "({} {} {})",
            const_rust_src(left)?,
            op.symbol(),
            const_rust_src(right)?
        ),
        _ => return None,
    })
}

/// Names of the parameters that a method body assigns through.
pub fn collect_mutated_params(body: &[Stmt], params: &[(String, Ty)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in body {
        collect_mutated_in_stmt(s, params, &mut out);
    }
    out
}

fn collect_mutated_in_stmt(s: &Stmt, params: &[(String, Ty)], out: &mut Vec<String>) {
    let mut note = |target: &Expr| {
        let base = match target {
            Expr::Attribute { value, .. } => &**value,
            Expr::Subscript { value, .. } => &**value,
            _ => return,
        };
        if let Expr::Name(n) = base {
            // `self` is not a parameter of the signature we track.
            if n != "self" && params.iter().any(|(p, _)| p == n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
    };
    match s {
        Stmt::Assign { targets, .. } => {
            for t in targets {
                note(t);
            }
        }
        Stmt::AugAssign { target, .. } => note(target),
        Stmt::AnnAssign { target, .. } => note(target),
        Stmt::If { body, orelse, .. } => {
            for b in body.iter().chain(orelse.iter()) {
                collect_mutated_in_stmt(b, params, out);
            }
        }
        Stmt::While { body, orelse, .. } => {
            for b in body.iter().chain(orelse.iter()) {
                collect_mutated_in_stmt(b, params, out);
            }
        }
        Stmt::For { body, orelse, .. } => {
            for b in body.iter().chain(orelse.iter()) {
                collect_mutated_in_stmt(b, params, out);
            }
        }
        Stmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for b in body.iter() {
                collect_mutated_in_stmt(b, params, out);
            }
            for h in handlers {
                for b in h.body.iter() {
                    collect_mutated_in_stmt(b, params, out);
                }
            }
            for b in orelse.iter().chain(finalbody.iter()) {
                collect_mutated_in_stmt(b, params, out);
            }
        }
        Stmt::With { body, .. } => {
            for b in body.iter() {
                collect_mutated_in_stmt(b, params, out);
            }
        }
        _ => {}
    }
}

/// Compute the `raises` flag for every function and method.
///
/// A function raises if it contains an explicit `raise`, or calls a raising
/// function outside a `try` block (errors raised inside a `try` are handled
/// locally by the handler).
pub fn compute_raises(env: &mut Env) {
    // Module-level functions.
    let names: Vec<String> = env.funcs.keys().cloned().collect();
    for _ in 0..10 {
        let mut changed = false;
        for name in &names {
            let body = match env.func_bodies.get(name) {
                Some(b) => b.clone(),
                None => continue,
            };
            let mut raises = contains_raise(&body);
            if !raises {
                let mut calls = Vec::new();
                collect_calls(&body, false, &mut calls);
                for c in calls {
                    if let Some(sig) = env.funcs.get(&c) {
                        if sig.raises {
                            raises = true;
                            break;
                        }
                    }
                }
            }
            if let Some(sig) = env.funcs.get_mut(name) {
                if sig.raises != raises {
                    sig.raises = raises;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Methods.
    let class_names: Vec<String> = env.classes.keys().cloned().collect();
    for _ in 0..10 {
        let mut changed = false;
        for cls in &class_names {
            let info = match env.classes.get(cls) {
                Some(i) => i.clone(),
                None => continue,
            };
            let mnames: Vec<String> = info.methods.keys().cloned().collect();
            for m in &mnames {
                let body = match info.method_bodies.get(m) {
                    Some(b) => b.clone(),
                    None => continue,
                };
                let mut raises = contains_raise(&body);
                if !raises {
                    let mut calls = Vec::new();
                    collect_calls(&body, false, &mut calls);
                    for c in calls {
                        if let Some(sig) = env.funcs.get(&c) {
                            if sig.raises {
                                raises = true;
                                break;
                            }
                        }
                        // A method of the same class may also raise.
                        if let Some(sig) = info.methods.get(&c) {
                            if sig.raises {
                                raises = true;
                                break;
                            }
                        }
                    }
                }
                if let Some(info) = env.classes.get_mut(cls) {
                    if let Some(sig) = info.methods.get_mut(m) {
                        if sig.raises != raises {
                            sig.raises = raises;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}
fn default_expr_source(e: &Expr) -> String {
    match e {
        Expr::Num(NumVal::Int(v)) => v.to_string(),
        Expr::Num(NumVal::Float(v)) => format!("{:?}", v),
        Expr::Str { value, kind } => match kind {
            StrKind::Str => format!("{:?}.to_string()", value),
            StrKind::Bytes => format!("{:?}.as_bytes().to_vec()", value),
        },
        Expr::Name(n) => match n.as_str() {
            "True" => "true".into(),
            "False" => "false".into(),
            "None" => "None".into(),
            _ => "Default::default()".into(),
        },
        Expr::List(_) => "Vec::new()".into(),
        Expr::Dict(_) => "HashMap::new()".into(),
        Expr::Set(_) => "HashSet::new()".into(),
        Expr::Call { func, args } => {
            let fname = match &**func {
                Expr::Name(n) => n.clone(),
                Expr::Attribute { value, attr } => match &**value {
                    Expr::Name(m) => format!("{}.{}", m, attr),
                    _ => attr.clone(),
                },
                _ => "unknown".into(),
            };
            if fname == "field" {
                // field(default_factory=list) etc.
                let df = args.iter().find_map(|a| match a {
                    Arg::Kw(k, e) if k == "default_factory" => Some(e),
                    _ => None,
                });
                match df {
                    Some(Expr::Name(n)) => match n.as_str() {
                        "list" => "Vec::new()".into(),
                        "dict" => "HashMap::new()".into(),
                        "set" => "HashSet::new()".into(),
                        _ => "Default::default()".into(),
                    },
                    Some(Expr::Lambda { body, .. }) => match &**body {
                        Expr::Call { func, .. } => match &**func {
                            Expr::Attribute { value, attr } => {
                                if attr == "now" {
                                    "py_now_iso()".into()
                                } else {
                                    match &**value {
                                        Expr::Name(m) => format!("{}.{}()", m, attr),
                                        _ => "Default::default()".into(),
                                    }
                                }
                            }
                            _ => "Default::default()".into(),
                        },
                        _ => "Default::default()".into(),
                    },
                    _ => "Default::default()".into(),
                }
            } else {
                "Default::default()".into()
            }
        }
        _ => "Default::default()".into(),
    }
}

// ---------------------------------------------------------------------------
// "Raises" analysis
// ---------------------------------------------------------------------------

/// Does this statement contain an explicit `raise`?
pub fn contains_raise(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_contains_raise)
}

fn stmt_contains_raise(s: &Stmt) -> bool {
    match s {
        Stmt::Raise { .. } => true,
        Stmt::If { body, orelse, .. } => contains_raise(body) || contains_raise(orelse),
        Stmt::While { body, orelse, .. } => contains_raise(body) || contains_raise(orelse),
        Stmt::For { body, orelse, .. } => contains_raise(body) || contains_raise(orelse),
        Stmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            contains_raise(body)
                || handlers.iter().any(|h| contains_raise(&h.body))
                || contains_raise(orelse)
                || contains_raise(finalbody)
        }
        Stmt::With { body, .. } => contains_raise(body),
        Stmt::FuncDef { body, .. } => contains_raise(body),
        Stmt::ClassDef { body, .. } => contains_raise(body),
        Stmt::ExprStmt(e) => expr_contains_raise(e),
        _ => false,
    }
}

fn expr_contains_raise(e: &Expr) -> bool {
    match e {
        Expr::Call { args, .. } => args.iter().any(|a| match a {
            Arg::Pos(x) | Arg::Star(x) | Arg::DoubleStar(x) => expr_contains_raise(x),
            Arg::Kw(_, x) => expr_contains_raise(x),
        }),
        Expr::BinOp { left, right, .. } => expr_contains_raise(left) || expr_contains_raise(right),
        Expr::UnaryOp { operand, .. } => expr_contains_raise(operand),
        _ => false,
    }
}

/// Names of functions called by these statements, excluding calls that occur
/// inside a `try` block (those errors are handled locally).
pub fn collect_calls(stmts: &[Stmt], inside_try: bool, out: &mut Vec<String>) {
    for s in stmts {
        stmt_collect_calls(s, inside_try, out);
    }
}

fn stmt_collect_calls(s: &Stmt, inside_try: bool, out: &mut Vec<String>) {
    match s {
        Stmt::ExprStmt(e) => expr_collect_calls(e, inside_try, out),
        Stmt::Assign { targets, value } => {
            expr_collect_calls(value, inside_try, out);
            for t in targets {
                expr_collect_calls(t, inside_try, out);
            }
        }
        Stmt::AugAssign { target, value, .. } => {
            expr_collect_calls(target, inside_try, out);
            expr_collect_calls(value, inside_try, out);
        }
        Stmt::AnnAssign { target, value, .. } => {
            expr_collect_calls(target, inside_try, out);
            if let Some(v) = value {
                expr_collect_calls(v, inside_try, out);
            }
        }
        Stmt::Return(Some(e)) => expr_collect_calls(e, inside_try, out),
        Stmt::Raise { exc, .. } => {
            if let Some(e) = exc {
                expr_collect_calls(e, inside_try, out);
            }
        }
        Stmt::If { body, orelse, .. } => {
            collect_calls(body, inside_try, out);
            collect_calls(orelse, inside_try, out);
        }
        Stmt::While { body, orelse, .. } => {
            collect_calls(body, inside_try, out);
            collect_calls(orelse, inside_try, out);
        }
        Stmt::For { body, orelse, .. } => {
            collect_calls(body, inside_try, out);
            collect_calls(orelse, inside_try, out);
        }
        Stmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            collect_calls(body, true, out);
            for h in handlers {
                collect_calls(&h.body, inside_try, out);
            }
            collect_calls(orelse, inside_try, out);
            collect_calls(finalbody, inside_try, out);
        }
        Stmt::With { body, .. } => collect_calls(body, inside_try, out),
        Stmt::FuncDef { body, .. } => collect_calls(body, inside_try, out),
        Stmt::ClassDef { body, .. } => collect_calls(body, inside_try, out),
        Stmt::Assert { test, msg } => {
            expr_collect_calls(test, inside_try, out);
            if let Some(m) = msg {
                expr_collect_calls(m, inside_try, out);
            }
        }
        _ => {}
    }
}

fn expr_collect_calls(e: &Expr, inside_try: bool, out: &mut Vec<String>) {
    match e {
        Expr::Call { func, args } => {
            if let Expr::Name(n) = &**func {
                out.push(n.clone());
            }
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Star(x) | Arg::DoubleStar(x) => {
                        expr_collect_calls(x, inside_try, out)
                    }
                    Arg::Kw(_, x) => expr_collect_calls(x, inside_try, out),
                }
            }
        }
        Expr::BinOp { left, right, .. } => {
            expr_collect_calls(left, inside_try, out);
            expr_collect_calls(right, inside_try, out);
        }
        Expr::UnaryOp { operand, .. } => expr_collect_calls(operand, inside_try, out),
        Expr::BoolOp { values, .. } => {
            for v in values {
                expr_collect_calls(v, inside_try, out);
            }
        }
        Expr::Compare { left, comparators, .. } => {
            expr_collect_calls(left, inside_try, out);
            for c in comparators {
                expr_collect_calls(c, inside_try, out);
            }
        }
        Expr::IfExp { test, body, orelse } => {
            expr_collect_calls(test, inside_try, out);
            expr_collect_calls(body, inside_try, out);
            expr_collect_calls(orelse, inside_try, out);
        }
        Expr::List(items) | Expr::Tuple(items) | Expr::Set(items) => {
            for i in items {
                expr_collect_calls(i, inside_try, out);
            }
        }
        Expr::Dict(entries) => {
            for (k, v) in entries {
                if let Some(k) = k {
                    expr_collect_calls(k, inside_try, out);
                }
                expr_collect_calls(v, inside_try, out);
            }
        }
        Expr::ListComp { elt, generators } => {
            expr_collect_calls(elt, inside_try, out);
            for g in generators {
                expr_collect_calls(&g.iter, inside_try, out);
                for i in &g.ifs {
                    expr_collect_calls(i, inside_try, out);
                }
            }
        }
        Expr::Attribute { value, .. } => expr_collect_calls(value, inside_try, out),
        Expr::Subscript { value, index } => {
            expr_collect_calls(value, inside_try, out);
            for i in index {
                match i {
                    IndexItem::Single(e) => expr_collect_calls(e, inside_try, out),
                    IndexItem::Slice { lower, upper, step } => {
                        for x in [lower, upper, step].into_iter().flatten() {
                            expr_collect_calls(x, inside_try, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}
