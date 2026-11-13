use std::fmt::Write;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Number(f64),
    String(String),
    Function(Rc<FunctionProto>),
}

impl Constant {
    pub fn display(&self) -> String {
        match self {
            Constant::Number(value) => format!("{value}"),
            Constant::String(value) => format!("{value:?}"),
            Constant::Function(proto) => format!("<function {}>", proto.display_name()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionProto {
    pub name: String,
    pub param_count: u16,
    pub local_names: Vec<String>,
    pub code: Vec<u8>,
    pub constants: Vec<Constant>,
}

impl FunctionProto {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "<anonymous>"
        } else {
            &self.name
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    PushConst = 0,
    PushUndefined = 1,
    PushNull = 2,
    PushTrue = 3,
    PushFalse = 4,
    Pop = 5,
    Dup = 6,
    Dup2 = 7,
    Swap = 8,
    GetLocal = 9,
    SetLocal = 10,
    GetName = 11,
    SetName = 12,
    GetProp = 13,
    SetProp = 14,
    GetIndex = 15,
    SetIndex = 16,
    GetIndexKeep = 17,
    SetPropKeepOld = 18,
    SetIndexKeepOld = 19,
    MakeFunction = 20,
    Call = 21,
    CallMethod = 22,
    Jump = 23,
    JumpIfFalse = 24,
    JumpIfTrue = 25,
    EnterScope = 26,
    ExitScope = 27,
    DeclareLocal = 28,
    Return = 29,
    LoadThis = 30,
    TypeofName = 31,
    Add = 32,
    Sub = 33,
    Mul = 34,
    Div = 35,
    Rem = 36,
    Pow = 37,
    Eq = 38,
    Ne = 39,
    StrictEq = 40,
    StrictNe = 41,
    Lt = 42,
    Le = 43,
    Gt = 44,
    Ge = 45,
    BitAnd = 46,
    BitOr = 47,
    BitXor = 48,
    Shl = 49,
    Shr = 50,
    UShr = 51,
    Not = 52,
    Neg = 53,
    Pos = 54,
    BitNot = 55,
    Typeof = 56,
    Void = 57,
    Array = 58,
    Object = 59,
}

impl OpCode {
    pub fn from_u8(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => OpCode::PushConst,
            1 => OpCode::PushUndefined,
            2 => OpCode::PushNull,
            3 => OpCode::PushTrue,
            4 => OpCode::PushFalse,
            5 => OpCode::Pop,
            6 => OpCode::Dup,
            7 => OpCode::Dup2,
            8 => OpCode::Swap,
            9 => OpCode::GetLocal,
            10 => OpCode::SetLocal,
            11 => OpCode::GetName,
            12 => OpCode::SetName,
            13 => OpCode::GetProp,
            14 => OpCode::SetProp,
            15 => OpCode::GetIndex,
            16 => OpCode::SetIndex,
            17 => OpCode::GetIndexKeep,
            18 => OpCode::SetPropKeepOld,
            19 => OpCode::SetIndexKeepOld,
            20 => OpCode::MakeFunction,
            21 => OpCode::Call,
            22 => OpCode::CallMethod,
            23 => OpCode::Jump,
            24 => OpCode::JumpIfFalse,
            25 => OpCode::JumpIfTrue,
            26 => OpCode::EnterScope,
            27 => OpCode::ExitScope,
            28 => OpCode::DeclareLocal,
            29 => OpCode::Return,
            30 => OpCode::LoadThis,
            31 => OpCode::TypeofName,
            32 => OpCode::Add,
            33 => OpCode::Sub,
            34 => OpCode::Mul,
            35 => OpCode::Div,
            36 => OpCode::Rem,
            37 => OpCode::Pow,
            38 => OpCode::Eq,
            39 => OpCode::Ne,
            40 => OpCode::StrictEq,
            41 => OpCode::StrictNe,
            42 => OpCode::Lt,
            43 => OpCode::Le,
            44 => OpCode::Gt,
            45 => OpCode::Ge,
            46 => OpCode::BitAnd,
            47 => OpCode::BitOr,
            48 => OpCode::BitXor,
            49 => OpCode::Shl,
            50 => OpCode::Shr,
            51 => OpCode::UShr,
            52 => OpCode::Not,
            53 => OpCode::Neg,
            54 => OpCode::Pos,
            55 => OpCode::BitNot,
            56 => OpCode::Typeof,
            57 => OpCode::Void,
            58 => OpCode::Array,
            59 => OpCode::Object,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            OpCode::PushConst => "PushConst",
            OpCode::PushUndefined => "PushUndefined",
            OpCode::PushNull => "PushNull",
            OpCode::PushTrue => "PushTrue",
            OpCode::PushFalse => "PushFalse",
            OpCode::Pop => "Pop",
            OpCode::Dup => "Dup",
            OpCode::Dup2 => "Dup2",
            OpCode::Swap => "Swap",
            OpCode::GetLocal => "GetLocal",
            OpCode::SetLocal => "SetLocal",
            OpCode::GetName => "GetName",
            OpCode::SetName => "SetName",
            OpCode::GetProp => "GetProp",
            OpCode::SetProp => "SetProp",
            OpCode::GetIndex => "GetIndex",
            OpCode::SetIndex => "SetIndex",
            OpCode::GetIndexKeep => "GetIndexKeep",
            OpCode::SetPropKeepOld => "SetPropKeepOld",
            OpCode::SetIndexKeepOld => "SetIndexKeepOld",
            OpCode::MakeFunction => "MakeFunction",
            OpCode::Call => "Call",
            OpCode::CallMethod => "CallMethod",
            OpCode::Jump => "Jump",
            OpCode::JumpIfFalse => "JumpIfFalse",
            OpCode::JumpIfTrue => "JumpIfTrue",
            OpCode::EnterScope => "EnterScope",
            OpCode::ExitScope => "ExitScope",
            OpCode::DeclareLocal => "DeclareLocal",
            OpCode::Return => "Return",
            OpCode::LoadThis => "LoadThis",
            OpCode::TypeofName => "TypeofName",
            OpCode::Add => "Add",
            OpCode::Sub => "Sub",
            OpCode::Mul => "Mul",
            OpCode::Div => "Div",
            OpCode::Rem => "Rem",
            OpCode::Pow => "Pow",
            OpCode::Eq => "Eq",
            OpCode::Ne => "Ne",
            OpCode::StrictEq => "StrictEq",
            OpCode::StrictNe => "StrictNe",
            OpCode::Lt => "Lt",
            OpCode::Le => "Le",
            OpCode::Gt => "Gt",
            OpCode::Ge => "Ge",
            OpCode::BitAnd => "BitAnd",
            OpCode::BitOr => "BitOr",
            OpCode::BitXor => "BitXor",
            OpCode::Shl => "Shl",
            OpCode::Shr => "Shr",
            OpCode::UShr => "UShr",
            OpCode::Not => "Not",
            OpCode::Neg => "Neg",
            OpCode::Pos => "Pos",
            OpCode::BitNot => "BitNot",
            OpCode::Typeof => "Typeof",
            OpCode::Void => "Void",
            OpCode::Array => "Array",
            OpCode::Object => "Object",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    PushConst(u32),
    PushUndefined,
    PushNull,
    PushTrue,
    PushFalse,
    Pop,
    Dup,
    Dup2,
    Swap,
    GetLocal(u16),
    SetLocal(u16),
    GetName(u32),
    SetName(u32),
    GetProp(u32),
    SetProp(u32),
    GetIndex,
    SetIndex,
    GetIndexKeep,
    SetPropKeepOld(u32),
    SetIndexKeepOld,
    MakeFunction(u32),
    Call(u16),
    CallMethod(u16),
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    EnterScope(u16),
    ExitScope,
    DeclareLocal(u16, u32),
    Return,
    LoadThis,
    TypeofName(u32),
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
    Not,
    Neg,
    Pos,
    BitNot,
    Typeof,
    Void,
    Array(u16),
    Object(u16),
}

impl Instruction {
    pub fn opcode(&self) -> OpCode {
        match self {
            Instruction::PushConst(_) => OpCode::PushConst,
            Instruction::PushUndefined => OpCode::PushUndefined,
            Instruction::PushNull => OpCode::PushNull,
            Instruction::PushTrue => OpCode::PushTrue,
            Instruction::PushFalse => OpCode::PushFalse,
            Instruction::Pop => OpCode::Pop,
            Instruction::Dup => OpCode::Dup,
            Instruction::Dup2 => OpCode::Dup2,
            Instruction::Swap => OpCode::Swap,
            Instruction::GetLocal(_) => OpCode::GetLocal,
            Instruction::SetLocal(_) => OpCode::SetLocal,
            Instruction::GetName(_) => OpCode::GetName,
            Instruction::SetName(_) => OpCode::SetName,
            Instruction::GetProp(_) => OpCode::GetProp,
            Instruction::SetProp(_) => OpCode::SetProp,
            Instruction::GetIndex => OpCode::GetIndex,
            Instruction::SetIndex => OpCode::SetIndex,
            Instruction::GetIndexKeep => OpCode::GetIndexKeep,
            Instruction::SetPropKeepOld(_) => OpCode::SetPropKeepOld,
            Instruction::SetIndexKeepOld => OpCode::SetIndexKeepOld,
            Instruction::MakeFunction(_) => OpCode::MakeFunction,
            Instruction::Call(_) => OpCode::Call,
            Instruction::CallMethod(_) => OpCode::CallMethod,
            Instruction::Jump(_) => OpCode::Jump,
            Instruction::JumpIfFalse(_) => OpCode::JumpIfFalse,
            Instruction::JumpIfTrue(_) => OpCode::JumpIfTrue,
            Instruction::EnterScope(_) => OpCode::EnterScope,
            Instruction::ExitScope => OpCode::ExitScope,
            Instruction::DeclareLocal(_, _) => OpCode::DeclareLocal,
            Instruction::Return => OpCode::Return,
            Instruction::LoadThis => OpCode::LoadThis,
            Instruction::TypeofName(_) => OpCode::TypeofName,
            Instruction::Add => OpCode::Add,
            Instruction::Sub => OpCode::Sub,
            Instruction::Mul => OpCode::Mul,
            Instruction::Div => OpCode::Div,
            Instruction::Rem => OpCode::Rem,
            Instruction::Pow => OpCode::Pow,
            Instruction::Eq => OpCode::Eq,
            Instruction::Ne => OpCode::Ne,
            Instruction::StrictEq => OpCode::StrictEq,
            Instruction::StrictNe => OpCode::StrictNe,
            Instruction::Lt => OpCode::Lt,
            Instruction::Le => OpCode::Le,
            Instruction::Gt => OpCode::Gt,
            Instruction::Ge => OpCode::Ge,
            Instruction::BitAnd => OpCode::BitAnd,
            Instruction::BitOr => OpCode::BitOr,
            Instruction::BitXor => OpCode::BitXor,
            Instruction::Shl => OpCode::Shl,
            Instruction::Shr => OpCode::Shr,
            Instruction::UShr => OpCode::UShr,
            Instruction::Not => OpCode::Not,
            Instruction::Neg => OpCode::Neg,
            Instruction::Pos => OpCode::Pos,
            Instruction::BitNot => OpCode::BitNot,
            Instruction::Typeof => OpCode::Typeof,
            Instruction::Void => OpCode::Void,
            Instruction::Array(_) => OpCode::Array,
            Instruction::Object(_) => OpCode::Object,
        }
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.opcode() as u8);
        match self {
            Instruction::PushConst(value)
            | Instruction::GetName(value)
            | Instruction::SetName(value)
            | Instruction::GetProp(value)
            | Instruction::SetProp(value)
            | Instruction::SetPropKeepOld(value)
            | Instruction::MakeFunction(value)
            | Instruction::TypeofName(value) => {
                out.extend_from_slice(&value.to_le_bytes());
            }
            Instruction::GetLocal(value)
            | Instruction::SetLocal(value)
            | Instruction::Call(value)
            | Instruction::CallMethod(value)
            | Instruction::EnterScope(value)
            | Instruction::Array(value)
            | Instruction::Object(value) => {
                out.extend_from_slice(&value.to_le_bytes());
            }
            Instruction::DeclareLocal(slot, name) => {
                out.extend_from_slice(&slot.to_le_bytes());
                out.extend_from_slice(&name.to_le_bytes());
            }
            Instruction::Jump(target)
            | Instruction::JumpIfFalse(target)
            | Instruction::JumpIfTrue(target) => {
                out.extend_from_slice(&target.to_le_bytes());
            }
            _ => {}
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }
}

pub fn decode_instruction(code: &[u8], ip: usize) -> Option<(Instruction, usize)> {
    let byte = *code.get(ip)?;
    let opcode = OpCode::from_u8(byte)?;
    let mut cursor = ip + 1;
    let read_u16 = |cursor: &mut usize| -> Option<u16> {
        let bytes = code.get(*cursor..*cursor + 2)?;
        *cursor += 2;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]))
    };
    let read_u32 = |cursor: &mut usize| -> Option<u32> {
        let bytes = code.get(*cursor..*cursor + 4)?;
        *cursor += 4;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };

    let instruction = match opcode {
        OpCode::PushConst => Instruction::PushConst(read_u32(&mut cursor)?),
        OpCode::PushUndefined => Instruction::PushUndefined,
        OpCode::PushNull => Instruction::PushNull,
        OpCode::PushTrue => Instruction::PushTrue,
        OpCode::PushFalse => Instruction::PushFalse,
        OpCode::Pop => Instruction::Pop,
        OpCode::Dup => Instruction::Dup,
        OpCode::Dup2 => Instruction::Dup2,
        OpCode::Swap => Instruction::Swap,
        OpCode::GetLocal => Instruction::GetLocal(read_u16(&mut cursor)?),
        OpCode::SetLocal => Instruction::SetLocal(read_u16(&mut cursor)?),
        OpCode::GetName => Instruction::GetName(read_u32(&mut cursor)?),
        OpCode::SetName => Instruction::SetName(read_u32(&mut cursor)?),
        OpCode::GetProp => Instruction::GetProp(read_u32(&mut cursor)?),
        OpCode::SetProp => Instruction::SetProp(read_u32(&mut cursor)?),
        OpCode::GetIndex => Instruction::GetIndex,
        OpCode::SetIndex => Instruction::SetIndex,
        OpCode::GetIndexKeep => Instruction::GetIndexKeep,
        OpCode::SetPropKeepOld => Instruction::SetPropKeepOld(read_u32(&mut cursor)?),
        OpCode::SetIndexKeepOld => Instruction::SetIndexKeepOld,
        OpCode::MakeFunction => Instruction::MakeFunction(read_u32(&mut cursor)?),
        OpCode::Call => Instruction::Call(read_u16(&mut cursor)?),
        OpCode::CallMethod => Instruction::CallMethod(read_u16(&mut cursor)?),
        OpCode::Jump => Instruction::Jump(read_u32(&mut cursor)?),
        OpCode::JumpIfFalse => Instruction::JumpIfFalse(read_u32(&mut cursor)?),
        OpCode::JumpIfTrue => Instruction::JumpIfTrue(read_u32(&mut cursor)?),
        OpCode::EnterScope => Instruction::EnterScope(read_u16(&mut cursor)?),
        OpCode::ExitScope => Instruction::ExitScope,
        OpCode::DeclareLocal => Instruction::DeclareLocal(
            read_u16(&mut cursor)?,
            read_u32(&mut cursor)?,
        ),
        OpCode::Return => Instruction::Return,
        OpCode::LoadThis => Instruction::LoadThis,
        OpCode::TypeofName => Instruction::TypeofName(read_u32(&mut cursor)?),
        OpCode::Add => Instruction::Add,
        OpCode::Sub => Instruction::Sub,
        OpCode::Mul => Instruction::Mul,
        OpCode::Div => Instruction::Div,
        OpCode::Rem => Instruction::Rem,
        OpCode::Pow => Instruction::Pow,
        OpCode::Eq => Instruction::Eq,
        OpCode::Ne => Instruction::Ne,
        OpCode::StrictEq => Instruction::StrictEq,
        OpCode::StrictNe => Instruction::StrictNe,
        OpCode::Lt => Instruction::Lt,
        OpCode::Le => Instruction::Le,
        OpCode::Gt => Instruction::Gt,
        OpCode::Ge => Instruction::Ge,
        OpCode::BitAnd => Instruction::BitAnd,
        OpCode::BitOr => Instruction::BitOr,
        OpCode::BitXor => Instruction::BitXor,
        OpCode::Shl => Instruction::Shl,
        OpCode::Shr => Instruction::Shr,
        OpCode::UShr => Instruction::UShr,
        OpCode::Not => Instruction::Not,
        OpCode::Neg => Instruction::Neg,
        OpCode::Pos => Instruction::Pos,
        OpCode::BitNot => Instruction::BitNot,
        OpCode::Typeof => Instruction::Typeof,
        OpCode::Void => Instruction::Void,
        OpCode::Array => Instruction::Array(read_u16(&mut cursor)?),
        OpCode::Object => Instruction::Object(read_u16(&mut cursor)?),
    };
    Some((instruction, cursor))
}

/// Renders every function prototype reachable from `proto` as a stable listing.
pub fn disassemble(proto: &FunctionProto) -> String {
    let mut out = String::new();
    disassemble_into(proto, 0, &mut out);
    out
}

fn disassemble_into(proto: &FunctionProto, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let _ = writeln!(
        out,
        "{indent}== {} params={} slots={} constants={} ==",
        proto.display_name(),
        proto.param_count,
        proto.local_names.len(),
        proto.constants.len()
    );
    if !proto.local_names.is_empty() {
        let _ = writeln!(out, "{indent}locals: {}", proto.local_names.join(", "));
    }
    let mut ip = 0usize;
    while ip < proto.code.len() {
        let Some((instruction, next)) = decode_instruction(&proto.code, ip) else {
            let _ = writeln!(out, "{indent}{ip:04}  <invalid bytecode>");
            break;
        };
        let operands = describe_operands(&instruction);
        let comment = describe_comment(&instruction, proto);
        if comment.is_empty() {
            let _ = writeln!(out, "{indent}{ip:04}  {operands}");
        } else {
            let _ = writeln!(out, "{indent}{ip:04}  {operands:<24} ; {comment}");
        }
        ip = next;
    }
    for constant in &proto.constants {
        if let Constant::Function(child) = constant {
            disassemble_into(child, depth + 1, out);
        }
    }
}

fn describe_operands(instruction: &Instruction) -> String {
    match instruction {
        Instruction::PushConst(index) => format!("PushConst {index}"),
        Instruction::GetLocal(slot) => format!("GetLocal {slot}"),
        Instruction::SetLocal(slot) => format!("SetLocal {slot}"),
        Instruction::GetName(index) => format!("GetName {index}"),
        Instruction::SetName(index) => format!("SetName {index}"),
        Instruction::GetProp(index) => format!("GetProp {index}"),
        Instruction::SetProp(index) => format!("SetProp {index}"),
        Instruction::SetPropKeepOld(index) => format!("SetPropKeepOld {index}"),
        Instruction::MakeFunction(index) => format!("MakeFunction {index}"),
        Instruction::Call(argc) => format!("Call {argc}"),
        Instruction::CallMethod(argc) => format!("CallMethod {argc}"),
        Instruction::Jump(target) => format!("Jump {target}"),
        Instruction::JumpIfFalse(target) => format!("JumpIfFalse {target}"),
        Instruction::JumpIfTrue(target) => format!("JumpIfTrue {target}"),
        Instruction::EnterScope(slots) => format!("EnterScope {slots}"),
        Instruction::DeclareLocal(slot, name) => format!("DeclareLocal {slot} {name}"),
        Instruction::TypeofName(index) => format!("TypeofName {index}"),
        Instruction::Array(count) => format!("Array {count}"),
        Instruction::Object(count) => format!("Object {count}"),
        other => other.opcode().name().to_string(),
    }
}

fn describe_comment(instruction: &Instruction, proto: &FunctionProto) -> String {
    let constant = |index: u32| -> String {
        proto
            .constants
            .get(index as usize)
            .map(Constant::display)
            .unwrap_or_else(|| format!("<missing constant {index}>"))
    };
    match instruction {
        Instruction::PushConst(index) => constant(*index),
        Instruction::GetName(index)
        | Instruction::SetName(index)
        | Instruction::GetProp(index)
        | Instruction::SetProp(index)
        | Instruction::SetPropKeepOld(index)
        | Instruction::TypeofName(index) => {
            match proto.constants.get(*index as usize) {
                Some(Constant::String(name)) => name.clone(),
                Some(other) => other.display(),
                None => format!("<missing constant {index}>"),
            }
        }
        Instruction::MakeFunction(index) => constant(*index),
        Instruction::DeclareLocal(_, name) => {
            match proto.constants.get(*name as usize) {
                Some(Constant::String(name)) => name.clone(),
                Some(other) => other.display(),
                None => format!("<missing constant {name}>"),
            }
        }
        _ => String::new(),
    }
}


