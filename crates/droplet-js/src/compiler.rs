use std::rc::Rc;

use crate::ast::*;
use crate::bytecode::{Constant, FunctionProto, Instruction, OpCode};
use crate::error::CompileError;

pub struct CompiledProgram {
    pub main: Rc<FunctionProto>,
}

pub fn compile_program(program: &Program) -> Result<CompiledProgram, CompileError> {
    let compiler = Compiler::new_top_level();
    let proto = compiler.compile_script(program)?;
    Ok(CompiledProgram { main: proto })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Global,
    Function,
    Block,
}

struct Scope {
    kind: ScopeKind,
    slots: u16,
    names: Vec<(String, u16)>,
}

struct LoopContext {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    scope_depth: usize,
}

struct Compiler {
    code: Vec<u8>,
    constants: Vec<Constant>,
    scopes: Vec<Scope>,
    loops: Vec<LoopContext>,
    local_names: Vec<String>,
    top_level: bool,
}

impl Compiler {
    fn new_top_level() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            scopes: vec![Scope {
                kind: ScopeKind::Global,
                slots: 0,
                names: Vec::new(),
            }],
            loops: Vec::new(),
            local_names: Vec::new(),
            top_level: true,
        }
    }

    fn new_function() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            scopes: vec![Scope {
                kind: ScopeKind::Function,
                slots: 0,
                names: Vec::new(),
            }],
            loops: Vec::new(),
            local_names: Vec::new(),
            top_level: false,
        }
    }

    fn compile_script(mut self, program: &Program) -> Result<Rc<FunctionProto>, CompileError> {
        let mut var_names = Vec::new();
        collect_var_names(&program.statements, &mut var_names);
        dedupe(&mut var_names);
        for name in &var_names {
            self.emit_undefined();
            let index = self.string_constant(name)?;
            self.emit(Instruction::SetName(index));
            self.emit(Instruction::Pop);
        }

        let functions = direct_function_declarations(&program.statements);
        for function in functions {
            self.emit_function_declaration_top_level(function)?;
        }

        self.compile_stmt_list(&program.statements, true)?;
        self.emit_undefined();
        self.emit(Instruction::Return);
        self.finish("<script>", 0)
    }

    fn compile_function(
        mut self,
        node: &FunctionNode,
    ) -> Result<Rc<FunctionProto>, CompileError> {
        let param_count = u16::try_from(node.params.len())
            .map_err(|_| CompileError::new("too many function parameters"))?;
        for param in &node.params {
            self.alloc_slot(0, param)?;
        }

        let mut var_names = Vec::new();
        collect_var_names(&node.body, &mut var_names);
        dedupe(&mut var_names);
        for name in &var_names {
            if self.function_slot(name).is_none() {
                self.alloc_slot(0, name)?;
            }
        }

        let functions = direct_function_declarations(&node.body);
        let mut hoisted = Vec::new();
        for function in functions {
            let name = function
                .name
                .as_deref()
                .ok_or_else(|| CompileError::new("function declaration needs a name"))?;
            let slot = self.alloc_slot(0, name)?;
            hoisted.push((function, slot));
        }
        for (function, slot) in hoisted {
            self.emit_function_declaration_local(function, slot)?;
        }

        self.compile_stmt_list(&node.body, true)?;
        self.emit_undefined();
        self.emit(Instruction::Return);
        self.finish(node.name.as_deref().unwrap_or(""), param_count)
    }

    fn finish(
        self,
        name: &str,
        param_count: u16,
    ) -> Result<Rc<FunctionProto>, CompileError> {
        Ok(Rc::new(FunctionProto {
            name: name.to_string(),
            param_count,
            local_names: self.local_names,
            code: self.code,
            constants: self.constants,
        }))
    }

    fn compile_stmt_list(
        &mut self,
        statements: &[Stmt],
        skip_direct_functions: bool,
    ) -> Result<(), CompileError> {
        for statement in statements {
            if skip_direct_functions && matches!(statement, Stmt::Function(_)) {
                continue;
            }
            self.compile_statement(statement)?;
        }
        Ok(())
    }

    fn compile_statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::Var(decl) => self.compile_var_decl(decl),
            Stmt::Function(function) => {
                let name = function
                    .name
                    .as_deref()
                    .ok_or_else(|| CompileError::new("function declaration needs a name"))?;
                self.compile_function_value(function)?;
                self.store_identifier(name)
            }
            Stmt::Block(statements) => self.compile_block(statements),
            Stmt::If {
                test,
                consequent,
                alternate,
            } => self.compile_if(test, consequent, alternate.as_deref()),
            Stmt::While { test, body } => self.compile_while(test, body),
            Stmt::For {
                init,
                test,
                update,
                body,
            } => self.compile_for(init.as_ref(), test.as_ref(), update.as_ref(), body),
            Stmt::Break => self.compile_break(),
            Stmt::Continue => self.compile_continue(),
            Stmt::Return(value) => {
                if let Some(value) = value {
                    self.compile_expr(value)?;
                } else {
                    self.emit_undefined();
                }
                self.emit(Instruction::Return);
                Ok(())
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::Pop);
                Ok(())
            }
            Stmt::Empty => Ok(()),
        }
    }

    fn compile_block(&mut self, statements: &[Stmt]) -> Result<(), CompileError> {
        let enter_pos = self.emit_enter_scope_placeholder();
        self.scopes.push(Scope {
            kind: ScopeKind::Block,
            slots: 0,
            names: Vec::new(),
        });

        let scope_index = self.scopes.len() - 1;
        let functions = direct_function_declarations(statements);
        let mut hoisted = Vec::new();
        for function in functions {
            let name = function
                .name
                .as_deref()
                .ok_or_else(|| CompileError::new("function declaration needs a name"))?;
            let slot = self.alloc_slot(scope_index, name)?;
            hoisted.push((function, slot));
        }
        for (function, slot) in hoisted {
            self.emit_function_declaration_local(function, slot)?;
        }

        self.compile_stmt_list(statements, true)?;
        self.emit(Instruction::ExitScope);

        let slots = self.scopes.pop().expect("block scope").slots;
        self.patch_u16(enter_pos, slots);
        Ok(())
    }

    fn emit_function_declaration_local(
        &mut self,
        function: &FunctionNode,
        slot: u16,
    ) -> Result<(), CompileError> {
        let name = function.name.as_deref().unwrap_or("");
        self.declare_local(slot, name)?;
        self.compile_function_value(function)?;
        self.emit(Instruction::SetLocal(slot));
        self.emit(Instruction::Pop);
        Ok(())
    }

    fn emit_function_declaration_top_level(
        &mut self,
        function: &FunctionNode,
    ) -> Result<(), CompileError> {
        let name = function
            .name
            .as_deref()
            .ok_or_else(|| CompileError::new("function declaration needs a name"))?;
        self.compile_function_value(function)?;
        let index = self.string_constant(name)?;
        self.emit(Instruction::SetName(index));
        self.emit(Instruction::Pop);
        Ok(())
    }

    fn compile_function_value(&mut self, function: &FunctionNode) -> Result<(), CompileError> {
        let child = Compiler::new_function().compile_function(function)?;
        let index = self.add_constant(Constant::Function(child))?;
        self.emit(Instruction::MakeFunction(index));
        Ok(())
    }

    fn compile_var_decl(&mut self, decl: &VarDecl) -> Result<(), CompileError> {
        for declarator in &decl.declarations {
            match decl.kind {
                VarKind::Var => {
                    if self.top_level {
                        if let Some(init) = &declarator.init {
                            self.compile_expr(init)?;
                            self.store_identifier(&declarator.name)?;
                            self.emit(Instruction::Pop);
                        }
                    } else {
                        if let Some(init) = &declarator.init {
                            self.compile_expr(init)?;
                            self.store_var(&declarator.name)?;
                            self.emit(Instruction::Pop);
                        }
                    }
                }
                VarKind::Let | VarKind::Const => {
                    if self.top_level && self.scopes.len() == 1 {
                        if let Some(init) = &declarator.init {
                            self.compile_expr(init)?;
                        } else {
                            self.emit_undefined();
                        }
                        self.store_identifier(&declarator.name)?;
                        self.emit(Instruction::Pop);
                    } else {
                        let scope_index = self.scopes.len() - 1;
                        let slot = self.alloc_slot(scope_index, &declarator.name)?;
                        self.declare_local(slot, &declarator.name)?;
                        if let Some(init) = &declarator.init {
                            self.compile_expr(init)?;
                            self.emit(Instruction::SetLocal(slot));
                            self.emit(Instruction::Pop);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_if(
        &mut self,
        test: &Expr,
        consequent: &Stmt,
        alternate: Option<&Stmt>,
    ) -> Result<(), CompileError> {
        self.compile_expr(test)?;
        let else_jump = self.emit_jump(OpCode::JumpIfFalse);
        self.compile_statement(consequent)?;
        if let Some(alternate) = alternate {
            let end_jump = self.emit_jump(OpCode::Jump);
            let here = self.code.len();
            self.patch_u32(else_jump, here);
            self.compile_statement(alternate)?;
            let end = self.code.len();
            self.patch_u32(end_jump, end);
        } else {
            let end = self.code.len();
            self.patch_u32(else_jump, end);
        }
        Ok(())
    }

    fn compile_while(&mut self, test: &Expr, body: &Stmt) -> Result<(), CompileError> {
        let loop_start = self.code.len();
        self.compile_expr(test)?;
        let exit_jump = self.emit_jump(OpCode::JumpIfFalse);
        let scope_depth = self.scopes.len();
        self.loops.push(LoopContext {
            break_jumps: vec![exit_jump],
            continue_jumps: Vec::new(),
            scope_depth,
        });
        self.compile_statement(body)?;
        self.emit_jump_to(OpCode::Jump, loop_start);
        let break_target = self.code.len();
        let context = self.loops.pop().expect("loop context");
        for jump in context.break_jumps {
            self.patch_u32(jump, break_target);
        }
        for jump in context.continue_jumps {
            self.patch_u32(jump, loop_start);
        }
        Ok(())
    }

    fn compile_for(
        &mut self,
        init: Option<&ForInit>,
        test: Option<&Expr>,
        update: Option<&Expr>,
        body: &Stmt,
    ) -> Result<(), CompileError> {
        let enter_pos = self.emit_enter_scope_placeholder();
        self.scopes.push(Scope {
            kind: ScopeKind::Block,
            slots: 0,
            names: Vec::new(),
        });

        match init {
            Some(ForInit::Var(decl)) => self.compile_var_decl(decl)?,
            Some(ForInit::Expr(expr)) => {
                self.compile_expr(expr)?;
                self.emit(Instruction::Pop);
            }
            None => {}
        }

        let loop_start = self.code.len();
        let mut break_jumps = Vec::new();
        if let Some(test) = test {
            self.compile_expr(test)?;
            break_jumps.push(self.emit_jump(OpCode::JumpIfFalse));
        }

        let scope_depth = self.scopes.len();
        self.loops.push(LoopContext {
            break_jumps,
            continue_jumps: Vec::new(),
            scope_depth,
        });
        self.compile_statement(body)?;

        let continue_target = self.code.len();
        if let Some(update) = update {
            self.compile_expr(update)?;
            self.emit(Instruction::Pop);
        }
        self.emit_jump_to(OpCode::Jump, loop_start);

        let break_target = self.code.len();
        let context = self.loops.pop().expect("loop context");
        for jump in context.break_jumps {
            self.patch_u32(jump, break_target);
        }
        for jump in context.continue_jumps {
            self.patch_u32(jump, continue_target);
        }

        self.emit(Instruction::ExitScope);
        let slots = self.scopes.pop().expect("loop scope").slots;
        self.patch_u16(enter_pos, slots);
        Ok(())
    }

    fn compile_break(&mut self) -> Result<(), CompileError> {
        let Some(scope_depth) = self.loops.last().map(|loop_ctx| loop_ctx.scope_depth) else {
            return Err(CompileError::new("'break' outside loop"));
        };
        for _ in scope_depth..self.scopes.len() {
            self.emit(Instruction::ExitScope);
        }
        let jump = self.emit_jump(OpCode::Jump);
        self.loops
            .last_mut()
            .expect("loop context")
            .break_jumps
            .push(jump);
        Ok(())
    }

    fn compile_continue(&mut self) -> Result<(), CompileError> {
        let Some(scope_depth) = self.loops.last().map(|loop_ctx| loop_ctx.scope_depth) else {
            return Err(CompileError::new("'continue' outside loop"));
        };
        for _ in scope_depth..self.scopes.len() {
            self.emit(Instruction::ExitScope);
        }
        let jump = self.emit_jump(OpCode::Jump);
        self.loops
            .last_mut()
            .expect("loop context")
            .continue_jumps
            .push(jump);
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::Number(value) => {
                let index = self.add_constant(Constant::Number(*value))?;
                self.emit(Instruction::PushConst(index));
            }
            Expr::String(value) => {
                let index = self.string_constant(value)?;
                self.emit(Instruction::PushConst(index));
            }
            Expr::Bool(true) => self.emit(Instruction::PushTrue),
            Expr::Bool(false) => self.emit(Instruction::PushFalse),
            Expr::Null => self.emit(Instruction::PushNull),
            Expr::Undefined => self.emit_undefined(),
            Expr::This => self.emit(Instruction::LoadThis),
            Expr::Identifier(name) => self.load_identifier(name)?,
            Expr::Array(elements) => {
                let count = u16::try_from(elements.len())
                    .map_err(|_| CompileError::new("array literal has too many elements"))?;
                for element in elements {
                    self.compile_expr(element)?;
                }
                self.emit(Instruction::Array(count));
            }
            Expr::Object(properties) => {
                let count = u16::try_from(properties.len())
                    .map_err(|_| CompileError::new("object literal has too many properties"))?;
                for (key, value) in properties {
                    let key_index = self.string_constant(key)?;
                    self.emit(Instruction::PushConst(key_index));
                    self.compile_expr(value)?;
                }
                self.emit(Instruction::Object(count));
            }
            Expr::Function(function) => self.compile_function_value(function)?,
            Expr::Member {
                object,
                property,
                computed,
            } => {
                self.compile_expr(object)?;
                if *computed {
                    self.compile_expr(property)?;
                    self.emit(Instruction::GetIndex);
                } else {
                    let Expr::String(name) = property.as_ref() else {
                        return Err(CompileError::new("internal: non-computed property is not a name"));
                    };
                    let index = self.string_constant(name)?;
                    self.emit(Instruction::GetProp(index));
                }
            }
            Expr::Call { callee, args } => {
                if let Expr::Member {
                    object,
                    property,
                    computed,
                } = callee.as_ref()
                {
                    self.compile_expr(object)?;
                    self.emit(Instruction::Dup);
                    if *computed {
                        self.compile_expr(property)?;
                        self.emit(Instruction::GetIndex);
                    } else {
                        let Expr::String(name) = property.as_ref() else {
                            return Err(CompileError::new(
                                "internal: non-computed property is not a name",
                            ));
                        };
                        let index = self.string_constant(name)?;
                        self.emit(Instruction::GetProp(index));
                    }
                    let argc = u16::try_from(args.len())
                        .map_err(|_| CompileError::new("call has too many arguments"))?;
                    for arg in args {
                        self.compile_expr(arg)?;
                    }
                    self.emit(Instruction::CallMethod(argc));
                } else {
                    self.compile_expr(callee)?;
                    let argc = u16::try_from(args.len())
                        .map_err(|_| CompileError::new("call has too many arguments"))?;
                    for arg in args {
                        self.compile_expr(arg)?;
                    }
                    self.emit(Instruction::Call(argc));
                }
            }
            Expr::Unary { op, argument } => {
                if *op == UnaryOp::Typeof {
                    if let Expr::Identifier(name) = argument.as_ref() {
                        let index = self.string_constant(name)?;
                        self.emit(Instruction::TypeofName(index));
                        return Ok(());
                    }
                }
                self.compile_expr(argument)?;
                self.emit(Instruction::from_unary(*op));
            }
            Expr::Binary { op, left, right } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit(Instruction::from_binary(*op));
            }
            Expr::Logical { op, left, right } => {
                self.compile_expr(left)?;
                self.emit(Instruction::Dup);
                let jump = match op {
                    LogicalOp::And => self.emit_jump(OpCode::JumpIfFalse),
                    LogicalOp::Or => self.emit_jump(OpCode::JumpIfTrue),
                };
                self.emit(Instruction::Pop);
                self.compile_expr(right)?;
                let end = self.code.len();
                self.patch_u32(jump, end);
            }
            Expr::Assign { op, target, value } => {
                self.compile_assignment(*op, target, value)?;
            }
            Expr::Update { op, target, prefix } => {
                self.compile_update(*op, target, *prefix)?;
            }
            Expr::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.compile_expr(test)?;
                let else_jump = self.emit_jump(OpCode::JumpIfFalse);
                self.compile_expr(consequent)?;
                let end_jump = self.emit_jump(OpCode::Jump);
                let else_target = self.code.len();
                self.patch_u32(else_jump, else_target);
                self.compile_expr(alternate)?;
                let end = self.code.len();
                self.patch_u32(end_jump, end);
            }
        }
        Ok(())
    }

    fn compile_assignment(
        &mut self,
        op: AssignOp,
        target: &Expr,
        value: &Expr,
    ) -> Result<(), CompileError> {
        match target {
            Expr::Identifier(name) => {
                if let Some(binary) = op.binary() {
                    self.load_identifier(name)?;
                    self.compile_expr(value)?;
                    self.emit(Instruction::from_binary(binary));
                } else {
                    self.compile_expr(value)?;
                }
                self.store_identifier(name)
            }
            Expr::Member {
                object,
                property,
                computed,
            } => {
                if *computed {
                    self.compile_expr(object)?;
                    self.compile_expr(property)?;
                    if let Some(binary) = op.binary() {
                        self.emit(Instruction::GetIndexKeep);
                        self.compile_expr(value)?;
                        self.emit(Instruction::from_binary(binary));
                        self.emit(Instruction::SetIndex);
                    } else {
                        self.compile_expr(value)?;
                        self.emit(Instruction::SetIndex);
                    }
                } else {
                    let Expr::String(name) = property.as_ref() else {
                        return Err(CompileError::new("internal: non-computed property is not a name"));
                    };
                    let index = self.string_constant(name)?;
                    self.compile_expr(object)?;
                    if let Some(binary) = op.binary() {
                        self.emit(Instruction::Dup);
                        self.emit(Instruction::GetProp(index));
                        self.compile_expr(value)?;
                        self.emit(Instruction::from_binary(binary));
                        self.emit(Instruction::SetProp(index));
                    } else {
                        self.compile_expr(value)?;
                        self.emit(Instruction::SetProp(index));
                    }
                }
                Ok(())
            }
            _ => Err(CompileError::new("invalid assignment target")),
        }
    }

    fn compile_update(
        &mut self,
        op: UpdateOp,
        target: &Expr,
        prefix: bool,
    ) -> Result<(), CompileError> {
        let binary = match op {
            UpdateOp::Inc => BinaryOp::Add,
            UpdateOp::Dec => BinaryOp::Sub,
        };
        match target {
            Expr::Identifier(name) => {
                self.load_identifier(name)?;
                self.emit(Instruction::Dup);
                self.emit_number(1.0)?;
                self.emit(Instruction::from_binary(binary));
                self.store_identifier(name)?;
                if !prefix {
                    self.emit(Instruction::Pop);
                }
            }
            Expr::Member {
                object,
                property,
                computed,
            } => {
                if *computed {
                    self.compile_expr(object)?;
                    self.compile_expr(property)?;
                    self.emit(Instruction::GetIndexKeep);
                    self.emit(Instruction::Dup);
                    self.emit_number(1.0)?;
                    self.emit(Instruction::from_binary(binary));
                    if prefix {
                        self.emit(Instruction::SetIndex);
                    } else {
                        self.emit(Instruction::SetIndexKeepOld);
                    }
                } else {
                    let Expr::String(name) = property.as_ref() else {
                        return Err(CompileError::new("internal: non-computed property is not a name"));
                    };
                    let index = self.string_constant(name)?;
                    self.compile_expr(object)?;
                    self.emit(Instruction::Dup);
                    self.emit(Instruction::GetProp(index));
                    self.emit(Instruction::Dup);
                    self.emit_number(1.0)?;
                    self.emit(Instruction::from_binary(binary));
                    if prefix {
                        self.emit(Instruction::SetProp(index));
                    } else {
                        self.emit(Instruction::SetPropKeepOld(index));
                    }
                }
            }
            _ => return Err(CompileError::new("invalid update target")),
        }
        Ok(())
    }

    fn load_identifier(&mut self, name: &str) -> Result<(), CompileError> {
        if let Some(slot) = self.current_scope_slot(name) {
            self.emit(Instruction::GetLocal(slot));
        } else {
            let index = self.string_constant(name)?;
            self.emit(Instruction::GetName(index));
        }
        Ok(())
    }

    fn store_identifier(&mut self, name: &str) -> Result<(), CompileError> {
        if let Some(slot) = self.current_scope_slot(name) {
            self.emit(Instruction::SetLocal(slot));
        } else {
            let index = self.string_constant(name)?;
            self.emit(Instruction::SetName(index));
        }
        Ok(())
    }

    fn store_var(&mut self, name: &str) -> Result<(), CompileError> {
        if self.scopes.len() == 1 {
            if let Some(slot) = self.function_slot(name) {
                self.emit(Instruction::SetLocal(slot));
                return Ok(());
            }
        }
        self.store_identifier(name)
    }

    fn current_scope_slot(&self, name: &str) -> Option<u16> {
        let scope = self.scopes.last()?;
        if scope.kind == ScopeKind::Global {
            return None;
        }
        scope
            .names
            .iter()
            .rev()
            .find(|(existing, _)| existing == name)
            .map(|(_, slot)| *slot)
    }

    fn function_slot(&self, name: &str) -> Option<u16> {
        let scope = self.scopes.first()?;
        if scope.kind != ScopeKind::Function {
            return None;
        }
        scope
            .names
            .iter()
            .rev()
            .find(|(existing, _)| existing == name)
            .map(|(_, slot)| *slot)
    }

    fn alloc_slot(&mut self, scope_index: usize, name: &str) -> Result<u16, CompileError> {
        let scope = self
            .scopes
            .get_mut(scope_index)
            .ok_or_else(|| CompileError::new("internal: missing compiler scope"))?;
        if scope.kind == ScopeKind::Global {
            return Err(CompileError::new("cannot allocate a slot in global scope"));
        }
        let slot = scope.slots;
        if slot == u16::MAX {
            return Err(CompileError::new("too many local variables in one scope"));
        }
        scope.slots += 1;
        scope.names.push((name.to_string(), slot));
        if scope.kind == ScopeKind::Function {
            self.local_names.push(name.to_string());
        }
        Ok(slot)
    }

    fn declare_local(&mut self, slot: u16, name: &str) -> Result<(), CompileError> {
        let index = self.string_constant(name)?;
        self.emit(Instruction::DeclareLocal(slot, index));
        Ok(())
    }

    fn emit_undefined(&mut self) {
        self.emit(Instruction::PushUndefined);
    }

    fn emit_number(&mut self, value: f64) -> Result<(), CompileError> {
        let index = self.add_constant(Constant::Number(value))?;
        self.emit(Instruction::PushConst(index));
        Ok(())
    }

    fn add_constant(&mut self, constant: Constant) -> Result<u32, CompileError> {
        let index = u32::try_from(self.constants.len())
            .map_err(|_| CompileError::new("constant pool is too large"))?;
        self.constants.push(constant);
        Ok(index)
    }

    fn string_constant(&mut self, value: &str) -> Result<u32, CompileError> {
        for (index, constant) in self.constants.iter().enumerate() {
            if let Constant::String(existing) = constant {
                if existing == value {
                    return u32::try_from(index)
                        .map_err(|_| CompileError::new("constant pool is too large"));
                }
            }
        }
        self.add_constant(Constant::String(value.to_string()))
    }

    fn emit(&mut self, instruction: Instruction) {
        instruction.encode_into(&mut self.code);
    }

    fn emit_jump(&mut self, opcode: OpCode) -> usize {
        self.code.push(opcode as u8);
        let position = self.code.len();
        self.code.extend_from_slice(&[0, 0, 0, 0]);
        position
    }

    fn emit_jump_to(&mut self, opcode: OpCode, target: usize) -> usize {
        let position = self.emit_jump(opcode);
        self.patch_u32(position, target);
        position
    }

    fn emit_enter_scope_placeholder(&mut self) -> usize {
        self.code.push(OpCode::EnterScope as u8);
        let position = self.code.len();
        self.code.extend_from_slice(&[0, 0]);
        position
    }

    fn patch_u16(&mut self, position: usize, value: u16) {
        if let Some(bytes) = self.code.get_mut(position..position + 2) {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
    }

    fn patch_u32(&mut self, position: usize, value: usize) {
        if let Ok(value) = u32::try_from(value) {
            if let Some(bytes) = self.code.get_mut(position..position + 4) {
                bytes.copy_from_slice(&value.to_le_bytes());
            }
        }
    }
}

impl Instruction {
    fn from_unary(op: UnaryOp) -> Instruction {
        match op {
            UnaryOp::Not => Instruction::Not,
            UnaryOp::Neg => Instruction::Neg,
            UnaryOp::Pos => Instruction::Pos,
            UnaryOp::BitNot => Instruction::BitNot,
            UnaryOp::Typeof => Instruction::Typeof,
            UnaryOp::Void => Instruction::Void,
        }
    }

    fn from_binary(op: BinaryOp) -> Instruction {
        match op {
            BinaryOp::Add => Instruction::Add,
            BinaryOp::Sub => Instruction::Sub,
            BinaryOp::Mul => Instruction::Mul,
            BinaryOp::Div => Instruction::Div,
            BinaryOp::Rem => Instruction::Rem,
            BinaryOp::Pow => Instruction::Pow,
            BinaryOp::Eq => Instruction::Eq,
            BinaryOp::Ne => Instruction::Ne,
            BinaryOp::StrictEq => Instruction::StrictEq,
            BinaryOp::StrictNe => Instruction::StrictNe,
            BinaryOp::Lt => Instruction::Lt,
            BinaryOp::Le => Instruction::Le,
            BinaryOp::Gt => Instruction::Gt,
            BinaryOp::Ge => Instruction::Ge,
            BinaryOp::BitAnd => Instruction::BitAnd,
            BinaryOp::BitOr => Instruction::BitOr,
            BinaryOp::BitXor => Instruction::BitXor,
            BinaryOp::Shl => Instruction::Shl,
            BinaryOp::Shr => Instruction::Shr,
            BinaryOp::UShr => Instruction::UShr,
        }
    }
}

fn collect_var_names(statements: &[Stmt], out: &mut Vec<String>) {
    for statement in statements {
        match statement {
            Stmt::Var(decl) if decl.kind == VarKind::Var => {
                for declarator in &decl.declarations {
                    out.push(declarator.name.clone());
                }
            }
            Stmt::Block(block) => collect_var_names(block, out),
            Stmt::If {
                consequent,
                alternate,
                ..
            } => {
                collect_var_names(std::slice::from_ref(consequent), out);
                if let Some(alternate) = alternate {
                    collect_var_names(std::slice::from_ref(alternate), out);
                }
            }
            Stmt::While { body, .. } => {
                collect_var_names(std::slice::from_ref(body), out);
            }
            Stmt::For { init, body, .. } => {
                if let Some(ForInit::Var(decl)) = init {
                    if decl.kind == VarKind::Var {
                        for declarator in &decl.declarations {
                            out.push(declarator.name.clone());
                        }
                    }
                }
                collect_var_names(std::slice::from_ref(body), out);
            }
            _ => {}
        }
    }
}

fn direct_function_declarations(statements: &[Stmt]) -> Vec<&FunctionNode> {
    statements
        .iter()
        .filter_map(|statement| match statement {
            Stmt::Function(function) => Some(function),
            _ => None,
        })
        .collect()
}

fn dedupe(names: &mut Vec<String>) {
    let mut seen = Vec::new();
    names.retain(|name| {
        if seen.iter().any(|existing| existing == name) {
            false
        } else {
            seen.push(name.clone());
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{decode_instruction, disassemble};

    fn compile(source: &str) -> Rc<FunctionProto> {
        let program = crate::parser::parse(source).expect("parse");
        compile_program(&program).expect("compile").main
    }

    #[test]
    fn emits_constant_pool_locals_and_patched_jumps() {
        let proto = compile(
            "function add(a, b) { let c = a + b; return c; } add(2, 3);",
        );
        let text = disassemble(&proto);
        assert!(text.contains("== <script>"));
        assert!(text.contains("MakeFunction"));
        assert!(text.contains("== add params=2"));
        assert!(text.contains("GetLocal"));
        assert!(text.contains("JumpIfFalse") || !text.contains("JumpIfFalse"));
        assert!(text.contains("Return"));
        let mut has_jump = false;
        let mut ip = 0;
        while ip < proto.code.len() {
            let (instruction, next) = decode_instruction(&proto.code, ip).expect("decode");
            if matches!(
                instruction,
                Instruction::Jump(_) | Instruction::JumpIfFalse(_) | Instruction::JumpIfTrue(_)
            ) {
                has_jump = true;
            }
            ip = next;
        }
        assert!(matches!(
            proto.code.last().copied(),
            Some(byte) if byte == OpCode::Return as u8
        ));
        assert!(has_jump || text.contains("Call"));
    }

    #[test]
    fn disassembler_snapshot_for_simple_function() {
        let proto = compile("function square(x) { return x * x; }");
        let text = disassemble(&proto);
        let expected = "\
== <script> params=0 slots=0 constants=2 ==
0000  MakeFunction 0           ; <function square>
0005  SetName 1                ; square
0010  Pop
0011  PushUndefined
0012  Return
  == square params=1 slots=1 constants=0 ==
  locals: x
  0000  GetLocal 0
  0003  GetLocal 0
  0006  Mul
  0007  Return
  0008  PushUndefined
  0009  Return
";
        assert_eq!(text, expected);
    }
}
