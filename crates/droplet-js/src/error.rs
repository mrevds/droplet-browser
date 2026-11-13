use std::fmt;

/// Source-positioned syntax or lexing failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl SyntaxError {
    pub fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SyntaxError: {} at line {}, column {}",
            self.message, self.line, self.column
        )
    }
}

impl std::error::Error for SyntaxError {}

/// Bytecode-generation failure (limits exceeded or invalid control flow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}

impl CompileError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CompileError: {}", self.message)
    }
}

impl std::error::Error for CompileError {}

/// Runtime failure with the active function name and instruction pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub kind: &'static str,
    pub message: String,
    pub function: String,
    pub pc: usize,
}

impl RuntimeError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            function: "<script>".to_string(),
            pc: 0,
        }
    }

    pub fn at(mut self, function: impl Into<String>, pc: usize) -> Self {
        self.function = function.into();
        self.pc = pc;
        self
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} (in {} at bytecode {})",
            self.kind, self.message, self.function, self.pc
        )
    }
}

impl std::error::Error for RuntimeError {}

/// Top-level error returned by a script run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsError {
    Syntax(SyntaxError),
    Compile(CompileError),
    Runtime(RuntimeError),
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsError::Syntax(err) => write!(f, "{err}"),
            JsError::Compile(err) => write!(f, "{err}"),
            JsError::Runtime(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for JsError {}

impl From<SyntaxError> for JsError {
    fn from(err: SyntaxError) -> Self {
        JsError::Syntax(err)
    }
}

impl From<CompileError> for JsError {
    fn from(err: CompileError) -> Self {
        JsError::Compile(err)
    }
}

impl From<RuntimeError> for JsError {
    fn from(err: RuntimeError) -> Self {
        JsError::Runtime(err)
    }
}
