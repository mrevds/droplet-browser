//! Minimal, dependency-free JavaScript subset for Droplet.
//!
//! The pipeline is deliberately explicit: source -> tokens -> AST -> bytecode
//! function prototypes -> stack VM. No AST is executed directly.

pub mod ast;
pub mod bytecode;
pub mod compiler;
pub mod dom;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod vm;

pub use compiler::{compile_program, CompiledProgram};
pub use dom::{DomEventInput, DomEventOutcome, DomHost, DomNodeId, MAX_DOM_LISTENERS};
pub use error::{CompileError, JsError, RuntimeError, SyntaxError};
pub use parser::parse;
pub use vm::JsRuntime;
