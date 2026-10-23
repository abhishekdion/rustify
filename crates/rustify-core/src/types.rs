//! Type lattice and inference for the Python subset.
//!
//! The compiler is *type-directed*: it infers a Rust type for every
//! expression and emits native Rust for the parts it understands. Anything it
//! cannot type confidently is reported as a diagnostic rather than silently
//! mistranslated.

use std::collections::{HashMap, HashSet};

use crate::ast::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    NoneT,
    List(Box<Ty>),
    Dict(Box<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    Set(Box<Ty>),
    Optional(Box<Ty>),
    /// A user class / dataclass instance.
    Named(String),
    /// Callable of unknown shape.
    Callable,
    /// A lambda / closure with a known shape (bound through a
    /// `Callable[[P1, ..], R]` annotation).
    Lambda {
        params: Vec<Ty>,
        ret: Box<Ty>,
        raises: bool,
    },
    /// Not statically known.
    Unknown,
}

impl Ty {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Ty::Unknown | Ty::Callable)
    }

    /// Optional-wrapped version of this type.
    pub fn optional(self) -> Ty {
        match self {
            Ty::NoneT => Ty::Optional(Box::new(Ty::NoneT)),
            Ty::Optional(_) => self,
            other => Ty::Optional(Box::new(other)),
        }
    }

    /// Strip an Optional wrapper.
    pub fn inner(&self) -> Ty {
        match self {
            Ty::Optional(t) => (**t).clone(),
            other => other.clone(),
        }
    }

    /// The Rust type used to represent this Python type.
    pub fn rust(&self) -> String {
        match self {
            Ty::Int => "i64".into(),
            Ty::Float => "f64".into(),
            Ty::Bool => "bool".into(),
            Ty::Str => "String".into(),
            Ty::Bytes => "Vec<u8>".into(),
            Ty::NoneT => "()".into(),
            Ty::List(t) => format!("Vec<{}>", t.rust()),
            Ty::Dict(k, v) => format!("HashMap<{}, {}>", k.rust(), v.rust()),
            Ty::Tuple(ts) => {
                if ts.is_empty() {
                    "()".into()
                } else {
                    format!(
                        "({})",
                        ts.iter()
                            .map(|t| t.rust())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            }
            Ty::Set(t) => format!("HashSet<{}>", t.rust()),
            Ty::Optional(t) => format!("Option<{}>", t.rust()),
            Ty::Named(n) => n.clone(),
            Ty::Lambda { .. } | Ty::Callable | Ty::Unknown => "PyValue".into(),
        }
    }

    /// Element type of an iterable (list, tuple, set, dict, str, bytes).
    pub fn iter_elem(&self) -> Ty {
        match self {
            Ty::List(t) | Ty::Set(t) => (**t).clone(),
            Ty::Tuple(ts) => {
                if ts.len() == 1 {
                    ts[0].clone()
                } else {
                    Ty::Unknown
                }
            }
            Ty::Dict(k, _) => (**k).clone(),
            Ty::Str => Ty::Str.into(),
            Ty::Bytes => Ty::Int.into(),
            _ => Ty::Unknown,
        }
    }

    /// The zero value used when a variable is only declared.
    pub fn zero(&self) -> String {
        match self {
            Ty::Int => "0".into(),
            Ty::Float => "0.0".into(),
            Ty::Bool => "false".into(),
            Ty::Str => "String::new()".into(),
            Ty::Bytes => "Vec::new()".into(),
            Ty::NoneT => "()".into(),
            Ty::List(_) => "Vec::new()".into(),
            Ty::Dict(_, _) => "HashMap::new()".into(),
            Ty::Set(_) => "HashSet::new()".into(),
            Ty::Tuple(ts) => format!(
                "({})",
                ts.iter()
                    .map(|t| t.zero())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Ty::Optional(t) => format!("None::<{}>", t.rust()),
            Ty::Named(_) => "Default::default()".into(),
            Ty::Lambda { .. } | Ty::Callable | Ty::Unknown => "PyValue::None".into(),
        }
    }
}

/// Least upper bound of two types.
pub fn unify(a: &Ty, b: &Ty) -> Ty {
    if a == b {
        return a.clone();
    }
    match (a, b) {
        (Ty::Unknown, o) | (o, Ty::Unknown) => o.clone(),
        (Ty::Callable, o) | (o, Ty::Callable) => o.clone(),
        (Ty::Bool, Ty::Int) | (Ty::Int, Ty::Bool) => Ty::Int,
        (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
        (Ty::Bool, Ty::Float) | (Ty::Float, Ty::Bool) => Ty::Float,
        (Ty::NoneT, o) | (o, Ty::NoneT) => o.clone().optional(),
        (Ty::Optional(x), Ty::Optional(y)) => Ty::Optional(Box::new(unify(x, y))),
        (Ty::Optional(x), y) | (y, Ty::Optional(x)) => Ty::Optional(Box::new(unify(x, y))),
        (Ty::List(x), Ty::List(y)) => Ty::List(Box::new(unify(x, y))),
        (Ty::Set(x), Ty::Set(y)) => Ty::Set(Box::new(unify(x, y))),
        (Ty::Dict(k1, v1), Ty::Dict(k2, v2)) => {
            Ty::Dict(Box::new(unify(k1, k2)), Box::new(unify(v1, v2)))
        }
        (Ty::Tuple(x), Ty::Tuple(y)) => {
            if x.len() == y.len() {
                Ty::Tuple(
                    x.iter()
                        .zip(y.iter())
                        .map(|(p, q)| unify(p, q))
                        .collect(),
                )
            } else {
                Ty::Unknown
            }
        }
        (Ty::Named(x), Ty::Named(y)) if x == y => Ty::Named(x.clone()),
        _ => Ty::Unknown,
    }
}

/// Convert a Python type annotation (as an AST expression) to a `Ty`.
pub fn ty_from_annotation(e: &Expr) -> Ty {
    match e {
        Expr::Name(n) => match n.as_str() {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "str" => Ty::Str,
            "bytes" => Ty::Bytes,
            "bytearray" => Ty::Bytes,
            "None" | "NoneType" => Ty::NoneT,
            "Any" => Ty::Unknown,
            "datetime" => Ty::Str,
            "list" | "List" => Ty::List(Box::new(Ty::Unknown)),
            "dict" | "Dict" => Ty::Dict(Box::new(Ty::Str), Box::new(Ty::Unknown)),
            "tuple" | "Tuple" => Ty::Tuple(vec![]),
            "set" | "Set" => Ty::Set(Box::new(Ty::Unknown)),
            "object" => Ty::Unknown,
            other => Ty::Named(other.to_string()),
        },
        Expr::Subscript { value, index } => {
            // Callable[[P1, P2], R] gives a lambda its full shape.
            if let Expr::Name(n) = &**value {
                if n == "Callable" {
                    return callable_shape_from_annotation(index);
                }
            }
            let base = ty_from_annotation(value);
            let args: Vec<Ty> = index
                .iter()
                .filter_map(|i| match i {
                    IndexItem::Single(e) => Some(ty_from_annotation(e)),
                    _ => None,
                })
                .collect();
            match base {
                Ty::Named(n) if n == "Optional" => match args.first() {
                    Some(t) => t.clone().optional(),
                    None => Ty::Unknown,
                },
                Ty::Named(n) if n == "Union" => {
                    let mut acc = Ty::NoneT;
                    for a in &args {
                        acc = unify(&acc, a);
                    }
                    acc
                }
                Ty::Named(n) if n == "Callable" => Ty::Callable,
                Ty::List(_) => Ty::List(Box::new(args.first().cloned().unwrap_or(Ty::Unknown))),
                Ty::Dict(_, _) => Ty::Dict(
                    Box::new(args.first().cloned().unwrap_or(Ty::Unknown)),
                    Box::new(args.get(1).cloned().unwrap_or(Ty::Unknown)),
                ),
                Ty::Set(_) => Ty::Set(Box::new(args.first().cloned().unwrap_or(Ty::Unknown))),
                Ty::Tuple(_) => Ty::Tuple(args),
                other => other,
            }
        }
        Expr::BinOp {
            op: BinOp::BitOr,
            left,
            right,
        } => {
            let l = ty_from_annotation(left);
            let r = ty_from_annotation(right);
            if matches!(l, Ty::NoneT) {
                r.optional()
            } else if matches!(r, Ty::NoneT) {
                l.optional()
            } else {
                Ty::Unknown
            }
        }
        Expr::Str { value, .. } => match value.as_str() {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "str" => Ty::Str,
            "bytes" => Ty::Bytes,
            _ => Ty::Unknown,
        },
        _ => Ty::Unknown,
    }
}

/// `Callable[[P1, P2], R]` (also with a parenthesised `(P1, P2)`) becomes a
/// fully typed lambda signature; anything else stays an unshaped callable.
fn callable_shape_from_annotation(index: &[IndexItem]) -> Ty {
    let mut params: Option<Vec<Ty>> = None;
    let mut ret = Ty::Unknown;
    let mut seen_params = false;
    for it in index {
        if let IndexItem::Single(e) = it {
            if !seen_params {
                match e {
                    Expr::List(ps) => {
                        params = Some(ps.iter().map(ty_from_annotation).collect());
                        seen_params = true;
                    }
                    Expr::Tuple(ts) => {
                        params = Some(ts.iter().map(ty_from_annotation).collect());
                        seen_params = true;
                    }
                    _ => {}
                }
            } else {
                ret = ty_from_annotation(e);
            }
        }
    }
    match params {
        Some(ps) => Ty::Lambda {
            params: ps,
            ret: Box::new(ret),
            raises: false,
        },
        None => Ty::Callable,
    }
}

// ---------------------------------------------------------------------------
// Function / class signatures
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct FuncSig {
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    /// The function can raise, so it is lowered to `Result<_, PyErr>`.
    pub raises: bool,
    /// Positional-only count (parameters before a `/` marker).
    pub positional_only: usize,
    /// The function is a method bound to `self`.
    pub is_method: bool,
    /// Rust source for each parameter's default value (if any).
    pub defaults: Vec<Option<String>>,
    /// Parameters that the body assigns through (passed as `&mut`).
    pub mut_params: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ClassInfo {
    pub name: String,
    pub fields: Vec<(String, Ty)>,
    pub field_defaults: HashMap<String, String>,
    pub methods: HashMap<String, FuncSig>,
    pub properties: HashSet<String>,
    pub bases: Vec<String>,
    pub is_dataclass: bool,
    /// Bodies of the class methods, used for the `raises` analysis.
    pub method_bodies: HashMap<String, Vec<Stmt>>,
    /// Parameters each method mutates (assigned through), per method name.
    pub method_mut_params: HashMap<String, Vec<String>>,
    /// The Python parameter list of each method, per method name.
    pub method_params: HashMap<String, Vec<Param>>,
}

#[derive(Default)]
pub struct Env {
    pub scopes: Vec<HashMap<String, Ty>>,
    pub funcs: HashMap<String, FuncSig>,
    pub classes: HashMap<String, ClassInfo>,
    pub diagnostics: Vec<Diagnostic>,
    /// Module-level variables that are not compile-time constants.
    pub module_vars: HashSet<String>,
    /// Bodies of module-level functions, used for the `raises` analysis.
    pub func_bodies: HashMap<String, Vec<Stmt>>,
    /// Top-level bindings of each module, keyed by dotted module name.
    pub module_scopes: HashMap<String, HashMap<String, Ty>>,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub line: usize,
    pub message: String,
}

impl Env {
    pub fn new() -> Self {
        Env {
            scopes: vec![HashMap::new()],
            ..Default::default()
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn lookup(&self, name: &str) -> Option<Ty> {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t.clone());
            }
        }
        None
    }

    pub fn set(&mut self, name: &str, t: Ty) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), t);
        }
    }

    pub fn diag(&mut self, line: usize, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            line,
            message: message.into(),
        });
    }

    /// A copy of this environment with a fresh local scope, used when
    /// generating a module that shares the global signature table.
    pub fn fork(&self) -> Env {
        Env {
            scopes: vec![HashMap::new()],
            funcs: self.funcs.clone(),
            classes: self.classes.clone(),
            diagnostics: Vec::new(),
            module_vars: self.module_vars.clone(),
            func_bodies: self.func_bodies.clone(),
            module_scopes: self.module_scopes.clone(),
        }
    }
}
