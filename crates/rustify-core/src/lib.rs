//! Rustify core — a Python-to-Rust transpiler for statically-typed hot-path code.
//!
//! Pipeline: lexer → parser → AST → type inference → Rust codegen, with a
//! runtime prelude emitted next to the generated code.

pub mod ast;
pub mod codegen;
pub mod infer;
pub mod lexer;
pub mod parser;
pub mod prelude;
pub mod types;
