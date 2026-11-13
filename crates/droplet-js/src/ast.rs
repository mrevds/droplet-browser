#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

impl VarKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VarKind::Var => "var",
            VarKind::Let => "let",
            VarKind::Const => "const",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDeclarator {
    pub name: String,
    pub init: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub kind: VarKind,
    pub declarations: Vec<VarDeclarator>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionNode {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ForInit {
    Var(VarDecl),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Var(VarDecl),
    Function(FunctionNode),
    Block(Vec<Stmt>),
    If {
        test: Expr,
        consequent: Box<Stmt>,
        alternate: Option<Box<Stmt>>,
    },
    While {
        test: Expr,
        body: Box<Stmt>,
    },
    For {
        init: Option<ForInit>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    Break,
    Continue,
    Return(Option<Expr>),
    Expr(Expr),
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
    Pos,
    BitNot,
    Typeof,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
}

impl AssignOp {
    pub fn binary(self) -> Option<BinaryOp> {
        Some(match self {
            AssignOp::Assign => return None,
            AssignOp::Add => BinaryOp::Add,
            AssignOp::Sub => BinaryOp::Sub,
            AssignOp::Mul => BinaryOp::Mul,
            AssignOp::Div => BinaryOp::Div,
            AssignOp::Rem => BinaryOp::Rem,
            AssignOp::Pow => BinaryOp::Pow,
            AssignOp::Shl => BinaryOp::Shl,
            AssignOp::Shr => BinaryOp::Shr,
            AssignOp::UShr => BinaryOp::UShr,
            AssignOp::BitAnd => BinaryOp::BitAnd,
            AssignOp::BitOr => BinaryOp::BitOr,
            AssignOp::BitXor => BinaryOp::BitXor,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Undefined,
    This,
    Identifier(String),
    Array(Vec<Expr>),
    Object(Vec<(String, Expr)>),
    Function(FunctionNode),
    Member {
        object: Box<Expr>,
        property: Box<Expr>,
        computed: bool,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Unary {
        op: UnaryOp,
        argument: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Logical {
        op: LogicalOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Assign {
        op: AssignOp,
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Update {
        op: UpdateOp,
        target: Box<Expr>,
        prefix: bool,
    },
    Conditional {
        test: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },
}
