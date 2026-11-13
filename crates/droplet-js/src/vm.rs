use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::bytecode::{decode_instruction, Constant, FunctionProto, Instruction};
use crate::compiler::compile_program;
use crate::dom::{
    host_get_property, host_set_property, make_host_object, same_listener, DomEventInput,
    DomEventOutcome, DomHost, DomListenerEntry, DomNodeId, EventState, HostBinding,
    MAX_BUBBLE_DEPTH, MAX_DOM_LISTENERS,
};
use crate::error::{JsError, RuntimeError};
use crate::parser::parse;

pub const DEFAULT_MAX_CALL_DEPTH: usize = 256;
pub const DEFAULT_INSTRUCTION_LIMIT: u64 = 1_000_000;
pub const MAX_PENDING_TIMERS: usize = 1024;
pub const MAX_TIMERS_PER_TICK: usize = 1000;
pub const MAX_VALUE_STACK: usize = 16_384;

pub type EnvRef = Rc<RefCell<Environment>>;
pub type NativeFn = fn(&mut JsRuntime, &Value, &[Value]) -> Result<Value, RuntimeError>;

#[derive(Clone)]
struct Binding {
    name: Option<String>,
    value: Value,
}

#[derive(Clone)]
pub struct Environment {
    bindings: Vec<Binding>,
    parent: Option<EnvRef>,
}

impl Environment {
    pub fn new(parent: Option<EnvRef>, slots: usize, names: Vec<String>) -> EnvRef {
        let mut bindings = Vec::with_capacity(slots.max(names.len()));
        for index in 0..slots.max(names.len()) {
            bindings.push(Binding {
                name: names.get(index).cloned(),
                value: Value::Undefined,
            });
        }
        Rc::new(RefCell::new(Self { bindings, parent }))
    }

    pub fn empty(parent: Option<EnvRef>) -> EnvRef {
        Rc::new(RefCell::new(Self {
            bindings: Vec::new(),
            parent,
        }))
    }

    fn get_local(&self, slot: u16) -> Value {
        self.bindings
            .get(slot as usize)
            .map(|binding| binding.value.clone())
            .unwrap_or(Value::Undefined)
    }

    fn set_local(&mut self, slot: u16, value: Value) {
        if let Some(binding) = self.bindings.get_mut(slot as usize) {
            binding.value = value;
        }
    }

    fn declare_local(&mut self, slot: u16, name: &str) {
        let index = slot as usize;
        while self.bindings.len() <= index {
            self.bindings.push(Binding {
                name: None,
                value: Value::Undefined,
            });
        }
        self.bindings[index].name = Some(name.to_string());
    }

    fn add_binding(&mut self, name: &str, value: Value) {
        if let Some(binding) = self
            .bindings
            .iter_mut()
            .rev()
            .find(|binding| binding.name.as_deref() == Some(name))
        {
            binding.value = value;
        } else {
            self.bindings.push(Binding {
                name: Some(name.to_string()),
                value,
            });
        }
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        self.bindings
            .iter()
            .rev()
            .find(|binding| binding.name.as_deref() == Some(name))
            .map(|binding| binding.value.clone())
    }
}

fn lookup_name(env: &EnvRef, name: &str) -> Option<Value> {
    let mut current = Some(env.clone());
    while let Some(env) = current {
        let borrowed = env.borrow();
        if let Some(value) = borrowed.lookup(name) {
            return Some(value);
        }
        current = borrowed.parent.clone();
    }
    None
}

fn set_name(env: &EnvRef, name: &str, value: Value) {
    let mut current = Some(env.clone());
    while let Some(candidate) = current {
        let mut borrowed = candidate.borrow_mut();
        if let Some(binding) = borrowed
            .bindings
            .iter_mut()
            .rev()
            .find(|binding| binding.name.as_deref() == Some(name))
        {
            binding.value = value;
            return;
        }
        current = borrowed.parent.clone();
    }
    env.borrow_mut().add_binding(name, value);
}

fn define_global(env: &EnvRef, name: &str, value: Value) {
    env.borrow_mut().add_binding(name, value);
}

#[derive(Debug, Clone, Default)]
pub struct Object {
    pub properties: Vec<(String, Value)>,
    pub(crate) host: Option<HostBinding>,
}

impl Object {
    pub fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            properties: Vec::new(),
            host: None,
        }))
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    pub fn set(&mut self, name: &str, value: Value) {
        if let Some((_, existing)) = self
            .properties
            .iter_mut()
            .find(|(key, _)| key == name)
        {
            *existing = value;
        } else {
            self.properties.push((name.to_string(), value));
        }
    }
}

#[derive(Clone)]
pub struct NativeFunction {
    pub name: String,
    pub function: NativeFn,
}

#[derive(Clone)]
pub struct Function {
    pub proto: Rc<FunctionProto>,
    pub env: EnvRef,
}

#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Rc<str>),
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<Object>>),
    Function(Rc<Function>),
    NativeFunction(Rc<NativeFunction>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) | Value::Object(_) => "object",
            Value::Function(_) | Value::NativeFunction(_) => "function",
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(value) => *value,
            Value::Number(value) => *value != 0.0 && !value.is_nan(),
            Value::String(value) => !value.is_empty(),
            _ => true,
        }
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn to_js_string(&self) -> String {
        to_string(self)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", to_string(self))
    }
}

fn to_primitive_string(value: &Value) -> String {
    match value {
        Value::Undefined => "undefined".to_string(),
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => number_to_string(*value),
        Value::String(value) => value.to_string(),
        Value::Array(values) => {
            let values = values.borrow();
            let mut parts = Vec::with_capacity(values.len());
            for value in values.iter() {
                parts.push(match value {
                    Value::Undefined | Value::Null => String::new(),
                    other => to_string(other),
                });
            }
            parts.join(",")
        }
        Value::Object(_) => "[object Object]".to_string(),
        Value::Function(function) => {
            format!("function {}() {{ [bytecode] }}", function.proto.display_name())
        }
        Value::NativeFunction(function) => {
            format!("function {}() {{ [native code] }}", function.name)
        }
    }
}

pub fn to_string(value: &Value) -> String {
    to_primitive_string(value)
}

fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else if value == 0.0 {
        "0".to_string()
    } else if value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

fn to_number(value: &Value) -> f64 {
    match value {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(value) => *value,
        Value::String(value) => string_to_number(value),
        Value::Array(values) => string_to_number(&to_primitive_string(&Value::Array(values.clone()))),
        Value::Object(_) => f64::NAN,
        Value::Function(_) | Value::NativeFunction(_) => f64::NAN,
    }
}

fn string_to_number(value: &str) -> f64 {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    if trimmed == "Infinity" || trimmed == "+Infinity" {
        return f64::INFINITY;
    }
    if trimmed == "-Infinity" {
        return f64::NEG_INFINITY;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16)
            .map(|value| value as f64)
            .unwrap_or(f64::NAN);
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

pub(crate) fn strict_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Undefined, Value::Undefined) => true,
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
        (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
        (Value::NativeFunction(a), Value::NativeFunction(b)) => Rc::ptr_eq(a, b),
        _ => false,
    }
}

fn loose_equal(left: &Value, right: &Value) -> bool {
    if strict_equal(left, right) {
        return true;
    }
    match (left, right) {
        (Value::Null | Value::Undefined, Value::Null | Value::Undefined) => true,
        (Value::Number(a), Value::String(b)) => *a == string_to_number(b),
        (Value::String(a), Value::Number(b)) => string_to_number(a) == *b,
        (Value::Bool(_), _) => loose_equal(&Value::Number(to_number(left)), right),
        (_, Value::Bool(_)) => loose_equal(left, &Value::Number(to_number(right))),
        (Value::Array(_) | Value::Object(_), Value::Number(_) | Value::String(_)) => {
            loose_equal(&Value::String(Rc::from(to_primitive_string(left))), right)
        }
        (Value::Number(_) | Value::String(_), Value::Array(_) | Value::Object(_)) => {
            loose_equal(left, &Value::String(Rc::from(to_primitive_string(right))))
        }
        _ => false,
    }
}

fn to_int32(value: f64) -> i32 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let truncated = value.trunc();
    let modulo = truncated.rem_euclid(4_294_967_296.0);
    if modulo >= 2_147_483_648.0 {
        (modulo - 4_294_967_296.0) as i32
    } else {
        modulo as i32
    }
}

fn to_uint32(value: f64) -> u32 {
    to_int32(value) as u32
}

fn property_key(value: &Value) -> String {
    match value {
        Value::String(value) => value.to_string(),
        Value::Number(value) => number_to_string(*value),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_string(),
        Value::Undefined => "undefined".to_string(),
        other => to_string(other),
    }
}

fn parse_array_index(name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    if name.len() > 1 && name.starts_with('0') {
        return None;
    }
    name.parse::<usize>().ok()
}

pub(crate) fn native(name: &str, function: NativeFn) -> Value {
    Value::NativeFunction(Rc::new(NativeFunction {
        name: name.to_string(),
        function,
    }))
}

pub(crate) fn array_value(values: Vec<Value>) -> Value {
    Value::Array(Rc::new(RefCell::new(values)))
}

fn object_value(properties: Vec<(String, Value)>) -> Value {
    Value::Object(Rc::new(RefCell::new(Object {
        properties,
        host: None,
    })))
}

#[derive(Clone)]
struct Timer {
    id: u64,
    due_ms: f64,
    interval_ms: Option<f64>,
    callback: Value,
    args: Vec<Value>,
    cancelled: bool,
}

pub struct TimerRun {
    pub id: u64,
    pub result: Result<Value, JsError>,
}

pub struct JsRuntime {
    global: EnvRef,
    console: Vec<String>,
    timers: Vec<Timer>,
    next_timer_id: u64,
    clock_ms: f64,
    max_call_depth: usize,
    max_instructions: u64,
    max_timers: usize,
    active_host: Option<*mut (dyn DomHost + 'static)>,
    dom_installed: bool,
    dom_elements: HashMap<DomNodeId, Rc<RefCell<Object>>>,
    dom_class_lists: HashMap<DomNodeId, Rc<RefCell<Object>>>,
    dom_styles: HashMap<DomNodeId, Rc<RefCell<Object>>>,
    dom_listeners: Vec<DomListenerEntry>,
    active_events: Vec<EventState>,
    next_event_id: u64,
}

struct HostGuard<'a> {
    runtime: &'a mut JsRuntime,
    prev: Option<*mut (dyn DomHost + 'static)>,
}

impl<'a> HostGuard<'a> {
    fn enter(runtime: &'a mut JsRuntime, host: &mut dyn DomHost) -> Self {
        // SAFETY: `host` is exclusively borrowed for the entire lifetime `'a` of `HostGuard`,
        // and `Drop` unconditionally restores `prev` before `'a` ends so the erased pointer
        // can never outlive the referent or be aliased across unwinding.
        let erased: *mut (dyn DomHost + 'static) =
            unsafe { std::mem::transmute::<*mut dyn DomHost, *mut (dyn DomHost + 'static)>(host) };
        let prev = runtime.active_host.replace(erased);
        Self { runtime, prev }
    }
}

impl Drop for HostGuard<'_> {
    fn drop(&mut self) {
        self.runtime.active_host = self.prev;
    }
}

impl Default for JsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for JsRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsRuntime")
            .field("clock_ms", &self.clock_ms)
            .field("pending_timers", &self.pending_timer_count())
            .field("console_len", &self.console.len())
            .field("dom_installed", &self.dom_installed)
            .field("dom_listeners", &self.dom_listeners.len())
            .finish()
    }
}

impl Drop for JsRuntime {
    fn drop(&mut self) {
        // Global bindings, element wrappers, and listeners can hold closures whose
        // captured environment points back at the global object. Clear them to break cycles.
        self.global.borrow_mut().bindings.clear();
        self.dom_elements.clear();
        self.dom_class_lists.clear();
        self.dom_styles.clear();
        self.dom_listeners.clear();
        self.active_events.clear();
        self.active_host = None;
    }
}

impl JsRuntime {
    pub fn new() -> Self {
        let global = Environment::empty(None);
        let mut runtime = Self {
            global,
            console: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
            clock_ms: 0.0,
            max_call_depth: DEFAULT_MAX_CALL_DEPTH,
            max_instructions: DEFAULT_INSTRUCTION_LIMIT,
            max_timers: MAX_PENDING_TIMERS,
            active_host: None,
            dom_installed: false,
            dom_elements: HashMap::new(),
            dom_class_lists: HashMap::new(),
            dom_styles: HashMap::new(),
            dom_listeners: Vec::new(),
            active_events: Vec::new(),
            next_event_id: 1,
        };
        runtime.install_globals();
        runtime
    }

    pub fn install_dom_bindings(&mut self) {
        if self.dom_installed {
            return;
        }
        let document = make_host_object(HostBinding::Document);
        let window = make_host_object(HostBinding::Window);
        let doc_val = Value::Object(document);
        window.borrow_mut().set("document", doc_val.clone());
        self.define_global("document", doc_val);
        self.define_global("window", Value::Object(window));
        self.dom_installed = true;
    }

    pub fn eval_with_host(
        &mut self,
        source: &str,
        host: &mut dyn DomHost,
    ) -> Result<Value, JsError> {
        self.install_dom_bindings();
        let guard = HostGuard::enter(self, host);
        guard.runtime.run_script(source)
    }

    pub fn dispatch_event_with_host(
        &mut self,
        event: &DomEventInput,
        host: &mut dyn DomHost,
    ) -> Result<DomEventOutcome, JsError> {
        self.install_dom_bindings();
        let guard = HostGuard::enter(self, host);
        let runtime = &mut *guard.runtime;

        let mut path = Vec::new();
        let mut curr = Some(event.target);
        while let Some(node_id) = curr {
            path.push(node_id);
            if path.len() >= MAX_BUBBLE_DEPTH {
                break;
            }
            curr = runtime
                .with_active_host(|h| h.parent_node(node_id))
                .map_err(JsError::Runtime)?;
        }

        let event_id = runtime.next_event_id;
        runtime.next_event_id = runtime.next_event_id.wrapping_add(1).max(1);
        runtime.active_events.push(EventState {
            id: event_id,
            event_type: event.event_type.clone(),
            target: event.target,
            current_target: event.target,
            key: event.key.clone(),
            value: event.value.clone(),
            checked: event.checked,
            default_prevented: false,
            propagation_stopped: false,
        });
        let event_val = Value::Object(make_host_object(HostBinding::Event(event_id)));

        let mut outcome = DomEventOutcome::default();
        let mut remaining_budget = runtime.max_instructions;

        let dispatch_res: Result<(), JsError> = (|| {
            for node_id in path {
                let listeners: Vec<Value> = runtime
                    .dom_listeners
                    .iter()
                    .filter(|entry| {
                        entry.node_id == node_id && entry.event_type == event.event_type
                    })
                    .map(|entry| entry.listener.clone())
                    .collect();
                if listeners.is_empty() {
                    continue;
                }
                if let Some(state) = runtime.event_state_mut(event_id) {
                    state.current_target = node_id;
                }
                let current_target_val = if node_id == 0 {
                    runtime
                        .get_global("document")
                        .unwrap_or_else(|| runtime.dom_element_value(0))
                } else {
                    runtime.dom_element_value(node_id)
                };
                for listener in listeners {
                    let used = runtime.invoke_with_budget(
                        listener,
                        current_target_val.clone(),
                        vec![event_val.clone()],
                        remaining_budget,
                    )?;
                    remaining_budget = remaining_budget.saturating_sub(used);
                    outcome.invoked_listeners += 1;
                }
                if runtime
                    .event_state(event_id)
                    .is_some_and(|state| state.propagation_stopped)
                {
                    break;
                }
            }
            Ok(())
        })();

        if let Some(pos) = runtime
            .active_events
            .iter()
            .rposition(|state| state.id == event_id)
        {
            let state = runtime.active_events.remove(pos);
            outcome.default_prevented = state.default_prevented;
            outcome.propagation_stopped = state.propagation_stopped;
        }

        dispatch_res?;
        Ok(outcome)
    }

    pub fn tick_timers_with_host(
        &mut self,
        now_ms: u64,
        host: &mut dyn DomHost,
    ) -> Result<usize, JsError> {
        self.install_dom_bindings();
        let guard = HostGuard::enter(self, host);
        let runtime = &mut *guard.runtime;
        let runs = runtime.tick_with_budget(now_ms as f64, runtime.max_instructions);
        let count = runs.len();
        for run in runs {
            run.result?;
        }
        Ok(count)
    }

    pub(crate) fn with_active_host<R>(
        &mut self,
        f: impl FnOnce(&mut dyn DomHost) -> R,
    ) -> Result<R, RuntimeError> {
        let Some(ptr) = self.active_host else {
            return Err(RuntimeError::new("TypeError", "DOM host is not active"));
        };
        // SAFETY: `HostGuard` guarantees `ptr` points to a valid `DomHost` exclusively
        // borrowed for the current execution scope, and `f` never re-enters `JsRuntime`.
        let host = unsafe { &mut *ptr };
        Ok(f(host))
    }

    fn notify_host_console(&mut self, line: &str) {
        if let Some(ptr) = self.active_host {
            // SAFETY: scoped `HostGuard` guarantees `ptr` is valid and non-aliased here.
            let host = unsafe { &mut *ptr };
            host.console_log(line);
        }
    }

    pub(crate) fn dom_element_value(&mut self, node_id: DomNodeId) -> Value {
        let obj = self
            .dom_elements
            .entry(node_id)
            .or_insert_with(|| make_host_object(HostBinding::Element(node_id)))
            .clone();
        Value::Object(obj)
    }

    pub(crate) fn dom_class_list_value(&mut self, node_id: DomNodeId) -> Value {
        let obj = self
            .dom_class_lists
            .entry(node_id)
            .or_insert_with(|| make_host_object(HostBinding::ClassList(node_id)))
            .clone();
        Value::Object(obj)
    }

    pub(crate) fn dom_style_value(&mut self, node_id: DomNodeId) -> Value {
        let obj = self
            .dom_styles
            .entry(node_id)
            .or_insert_with(|| make_host_object(HostBinding::Style(node_id)))
            .clone();
        Value::Object(obj)
    }

    pub(crate) fn add_dom_listener(
        &mut self,
        node_id: DomNodeId,
        event_type: String,
        listener: Value,
    ) -> Result<(), RuntimeError> {
        if self.dom_listeners.iter().any(|entry| {
            entry.node_id == node_id
                && entry.event_type == event_type
                && same_listener(&entry.listener, &listener)
        }) {
            return Ok(());
        }
        if self.dom_listeners.len() >= MAX_DOM_LISTENERS {
            return Err(RuntimeError::new(
                "RangeError",
                format!("too many DOM event listeners (limit {MAX_DOM_LISTENERS})"),
            ));
        }
        self.dom_listeners.push(DomListenerEntry {
            node_id,
            event_type,
            listener,
        });
        Ok(())
    }

    pub(crate) fn remove_dom_listener(
        &mut self,
        node_id: DomNodeId,
        event_type: &str,
        listener: &Value,
    ) {
        self.dom_listeners.retain(|entry| {
            !(entry.node_id == node_id
                && entry.event_type == event_type
                && same_listener(&entry.listener, listener))
        });
    }

    pub(crate) fn event_state(&self, id: u64) -> Option<&EventState> {
        self.active_events.iter().find(|state| state.id == id)
    }

    pub(crate) fn event_state_mut(&mut self, id: u64) -> Option<&mut EventState> {
        self.active_events.iter_mut().find(|state| state.id == id)
    }

    pub fn run_script(&mut self, source: &str) -> Result<Value, JsError> {
        let program = parse(source)?;
        let compiled = compile_program(&program)?;
        self.run_proto(compiled.main.clone(), self.global.clone(), Value::Undefined)
    }

    pub fn console_log(&self) -> &[String] {
        &self.console
    }

    pub fn take_console_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.console)
    }

    pub fn clear_console_log(&mut self) {
        self.console.clear();
    }

    pub fn define_global(&mut self, name: &str, value: Value) {
        define_global(&self.global, name, value);
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        lookup_name(&self.global, name)
    }

    pub fn set_call_depth_limit(&mut self, limit: usize) {
        self.max_call_depth = limit.max(1);
    }

    pub fn set_instruction_limit(&mut self, limit: u64) {
        self.max_instructions = limit.max(1);
    }

    pub fn set_timer_limit(&mut self, limit: usize) {
        self.max_timers = limit.max(1);
    }

    pub fn call_depth_limit(&self) -> usize {
        self.max_call_depth
    }

    pub fn instruction_limit(&self) -> u64 {
        self.max_instructions
    }

    pub fn timer_limit(&self) -> usize {
        self.max_timers
    }

    pub fn clock_ms(&self) -> f64 {
        self.clock_ms
    }

    pub fn pending_timer_count(&self) -> usize {
        self.timers.iter().filter(|timer| !timer.cancelled).count()
    }

    pub fn tick(&mut self, now_ms: f64) -> Vec<TimerRun> {
        self.clock_ms = now_ms.max(0.0);
        self.run_due(now_ms.max(0.0))
    }

    pub fn advance(&mut self, delta_ms: f64) -> Vec<TimerRun> {
        let now = self.clock_ms + delta_ms.max(0.0);
        self.tick(now)
    }

    pub fn tick_with_budget(&mut self, now_ms: f64, budget: u64) -> Vec<TimerRun> {
        self.clock_ms = now_ms.max(0.0);
        self.run_due_with_budget(now_ms.max(0.0), budget)
    }

    pub fn run_due(&mut self, now_ms: f64) -> Vec<TimerRun> {
        self.clock_ms = now_ms.max(0.0);
        let mut results = Vec::new();
        for _ in 0..MAX_TIMERS_PER_TICK {
            let Some(index) = self
                .timers
                .iter()
                .enumerate()
                .filter(|(_, timer)| !timer.cancelled && timer.due_ms <= self.clock_ms)
                .min_by(|(_, a), (_, b)| {
                    a.due_ms
                        .partial_cmp(&b.due_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.id.cmp(&b.id))
                })
                .map(|(index, _)| index)
            else {
                break;
            };

            let timer = self.timers[index].clone();
            if let Some(interval) = timer.interval_ms {
                let next_due = if timer.due_ms + interval <= self.clock_ms {
                    self.clock_ms + interval
                } else {
                    timer.due_ms + interval
                };
                self.timers[index].due_ms = next_due;
            } else {
                self.timers[index].cancelled = true;
            }

            let result = self.invoke(timer.callback, Value::Undefined, timer.args);
            results.push(TimerRun {
                id: timer.id,
                result,
            });
            self.timers.retain(|timer| !timer.cancelled);
        }
        results
    }

    fn run_due_with_budget(&mut self, now_ms: f64, budget: u64) -> Vec<TimerRun> {
        self.clock_ms = now_ms.max(0.0);
        let mut results = Vec::new();
        let mut remaining_budget = budget;
        for _ in 0..MAX_TIMERS_PER_TICK {
            let Some(index) = self
                .timers
                .iter()
                .enumerate()
                .filter(|(_, timer)| !timer.cancelled && timer.due_ms <= self.clock_ms)
                .min_by(|(_, a), (_, b)| {
                    a.due_ms
                        .partial_cmp(&b.due_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.id.cmp(&b.id))
                })
                .map(|(index, _)| index)
            else {
                break;
            };

            let timer = self.timers[index].clone();
            if let Some(interval) = timer.interval_ms {
                let next_due = if timer.due_ms + interval <= self.clock_ms {
                    self.clock_ms + interval
                } else {
                    timer.due_ms + interval
                };
                self.timers[index].due_ms = next_due;
            } else {
                self.timers[index].cancelled = true;
            }

            let result = match self.invoke_with_budget_value(
                timer.callback,
                Value::Undefined,
                timer.args,
                remaining_budget,
            ) {
                Ok((val, used)) => {
                    remaining_budget = remaining_budget.saturating_sub(used);
                    Ok(val)
                }
                Err(err) => {
                    remaining_budget = 0;
                    Err(err)
                }
            };
            results.push(TimerRun {
                id: timer.id,
                result,
            });
            self.timers.retain(|timer| !timer.cancelled);
        }
        results
    }

    fn add_timer(
        &mut self,
        callback: Value,
        delay_ms: f64,
        interval_ms: Option<f64>,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        if !matches!(
            callback,
            Value::Function(_) | Value::NativeFunction(_)
        ) {
            return Err(RuntimeError::new(
                "TypeError",
                "timer callback must be a function",
            ));
        }
        self.timers.retain(|timer| !timer.cancelled);
        if self.timers.len() >= self.max_timers {
            return Err(RuntimeError::new(
                "RangeError",
                format!("too many pending timers (limit {})", self.max_timers),
            ));
        }
        let delay = if delay_ms.is_finite() {
            delay_ms.max(0.0)
        } else {
            0.0
        };
        let interval = interval_ms.map(|value| {
            if value.is_finite() && value > 0.0 {
                value.max(1.0)
            } else {
                1.0
            }
        });
        let id = self.next_timer_id;
        self.next_timer_id = self.next_timer_id.wrapping_add(1).max(1);
        self.timers.push(Timer {
            id,
            due_ms: self.clock_ms + delay,
            interval_ms: interval,
            callback,
            args,
            cancelled: false,
        });
        Ok(Value::Number(id as f64))
    }

    fn clear_timer(&mut self, id: f64) {
        let id = id as u64;
        for timer in &mut self.timers {
            if timer.id == id {
                timer.cancelled = true;
            }
        }
        self.timers.retain(|timer| !timer.cancelled);
    }

    pub fn invoke(
        &mut self,
        callable: Value,
        this: Value,
        args: Vec<Value>,
    ) -> Result<Value, JsError> {
        let budget = self.max_instructions;
        self.invoke_with_budget_value(callable, this, args, budget)
            .map(|(value, _)| value)
    }

    pub(crate) fn invoke_with_budget(
        &mut self,
        callable: Value,
        this: Value,
        args: Vec<Value>,
        budget: u64,
    ) -> Result<u64, JsError> {
        self.invoke_with_budget_value(callable, this, args, budget)
            .map(|(_, used)| used)
    }

    fn invoke_with_budget_value(
        &mut self,
        callable: Value,
        this: Value,
        args: Vec<Value>,
        budget: u64,
    ) -> Result<(Value, u64), JsError> {
        if budget == 0 {
            return Err(JsError::Runtime(RuntimeError::new(
                "RangeError",
                "instruction limit exceeded",
            )));
        }
        match callable {
            Value::Function(function) => self
                .run_function_with_budget(function, this, args, budget)
                .map_err(JsError::Runtime),
            Value::NativeFunction(function) => (function.function)(self, &this, &args)
                .map(|val| (val, 1))
                .map_err(JsError::Runtime),
            other => Err(JsError::Runtime(RuntimeError::new(
                "TypeError",
                format!("{} is not a function", to_string(&other)),
            ))),
        }
    }

    fn run_proto(
        &mut self,
        proto: Rc<FunctionProto>,
        env: EnvRef,
        this_value: Value,
    ) -> Result<Value, JsError> {
        let budget = self.max_instructions;
        let mut executor = Executor {
            vm: self,
            stack: Vec::new(),
            frames: vec![Frame {
                proto,
                env,
                ip: 0,
                this_value,
            }],
            instructions: 0,
            budget,
        };
        executor.run().map_err(JsError::Runtime)
    }

    fn run_function_with_budget(
        &mut self,
        function: Rc<Function>,
        this_value: Value,
        args: Vec<Value>,
        budget: u64,
    ) -> Result<(Value, u64), RuntimeError> {
        let env = Environment::new(
            Some(function.env.clone()),
            function.proto.local_names.len(),
            function.proto.local_names.clone(),
        );
        for (index, arg) in args.into_iter().enumerate() {
            if index < function.proto.param_count as usize {
                env.borrow_mut().set_local(index as u16, arg);
            }
        }
        if !function.proto.name.is_empty() {
            env.borrow_mut().add_binding(
                &function.proto.name,
                Value::Function(function.clone()),
            );
        }
        let mut executor = Executor {
            vm: self,
            stack: Vec::new(),
            frames: vec![Frame {
                proto: function.proto.clone(),
                env,
                ip: 0,
                this_value,
            }],
            instructions: 0,
            budget,
        };
        let value = executor.run()?;
        Ok((value, executor.instructions))
    }
}

struct Frame {
    proto: Rc<FunctionProto>,
    env: EnvRef,
    ip: usize,
    this_value: Value,
}

struct Executor<'a> {
    vm: &'a mut JsRuntime,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    instructions: u64,
    budget: u64,
}

impl<'a> Executor<'a> {
    fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            if self.frames.is_empty() {
                return Ok(self.result());
            }
            let (proto, pc) = {
                let frame = self.frames.last().expect("frame");
                (frame.proto.clone(), frame.ip)
            };
            if pc >= proto.code.len() {
                let value = Value::Undefined;
                if self.do_return(value)? {
                    return Ok(self.result());
                }
                continue;
            }
            let Some((instruction, next)) = decode_instruction(&proto.code, pc) else {
                return Err(self.error("BytecodeError", "invalid or truncated instruction"));
            };
            self.frames.last_mut().expect("frame").ip = next;
            self.instructions += 1;
            if self.instructions > self.budget {
                return Err(self.error(
                    "RangeError",
                    format!(
                        "instruction limit of {} exceeded",
                        self.budget
                    ),
                ));
            }
            self.execute(instruction)?;
        }
    }

    fn result(&self) -> Value {
        self.stack.last().cloned().unwrap_or(Value::Undefined)
    }

    fn error(&self, kind: &'static str, message: impl Into<String>) -> RuntimeError {
        match self.frames.last() {
            Some(frame) => RuntimeError::new(kind, message)
                .at(frame.proto.display_name(), frame.ip),
            None => RuntimeError::new(kind, message),
        }
    }

    fn push(&mut self, value: Value) -> Result<(), RuntimeError> {
        if self.stack.len() >= MAX_VALUE_STACK {
            return Err(self.error("RangeError", "value stack limit exceeded"));
        }
        self.stack.push(value);
        Ok(())
    }

    fn pop(&mut self) -> Result<Value, RuntimeError> {
        self.stack
            .pop()
            .ok_or_else(|| self.error("BytecodeError", "value stack underflow"))
    }

    fn pop_args(&mut self, argc: u16) -> Result<Vec<Value>, RuntimeError> {
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop()?);
        }
        args.reverse();
        Ok(args)
    }

    fn constant(&self, index: u32) -> Result<Constant, RuntimeError> {
        let proto = &self.frames.last().expect("frame").proto;
        proto
            .constants
            .get(index as usize)
            .cloned()
            .ok_or_else(|| self.error("BytecodeError", format!("missing constant {index}")))
    }

    fn string_constant(&self, index: u32) -> Result<String, RuntimeError> {
        match self.constant(index)? {
            Constant::String(value) => Ok(value),
            other => Err(self.error(
                "BytecodeError",
                format!("constant {index} is not a string ({})", other.display()),
            )),
        }
    }

    fn current_env(&self) -> EnvRef {
        self.frames.last().expect("frame").env.clone()
    }

    fn current_this(&self) -> Value {
        self.frames.last().expect("frame").this_value.clone()
    }

    fn execute(&mut self, instruction: Instruction) -> Result<(), RuntimeError> {
        match instruction {
            Instruction::PushConst(index) => {
                let constant = self.constant(index)?;
                self.push(match constant {
                    Constant::Number(value) => Value::Number(value),
                    Constant::String(value) => Value::String(Rc::from(value)),
                    Constant::Function(_) => {
                        return Err(self.error(
                            "BytecodeError",
                            "function prototype must be loaded with MakeFunction",
                        ))
                    }
                })?;
            }
            Instruction::PushUndefined => self.push(Value::Undefined)?,
            Instruction::PushNull => self.push(Value::Null)?,
            Instruction::PushTrue => self.push(Value::Bool(true))?,
            Instruction::PushFalse => self.push(Value::Bool(false))?,
            Instruction::Pop => {
                self.pop()?;
            }
            Instruction::Dup => {
                let value = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| self.error("BytecodeError", "value stack underflow"))?;
                self.push(value)?;
            }
            Instruction::Dup2 => {
                let length = self.stack.len();
                if length < 2 {
                    return Err(self.error("BytecodeError", "value stack underflow"));
                }
                let first = self.stack[length - 2].clone();
                let second = self.stack[length - 1].clone();
                self.push(first)?;
                self.push(second)?;
            }
            Instruction::Swap => {
                let length = self.stack.len();
                if length < 2 {
                    return Err(self.error("BytecodeError", "value stack underflow"));
                }
                self.stack.swap(length - 1, length - 2);
            }
            Instruction::GetLocal(slot) => {
                let value = self.current_env().borrow().get_local(slot);
                self.push(value)?;
            }
            Instruction::SetLocal(slot) => {
                let value = self.pop()?;
                self.current_env().borrow_mut().set_local(slot, value.clone());
                self.push(value)?;
            }
            Instruction::GetName(index) => {
                let name = self.string_constant(index)?;
                let value = lookup_name(&self.current_env(), &name)
                    .ok_or_else(|| self.error("ReferenceError", format!("{name} is not defined")))?;
                self.push(value)?;
            }
            Instruction::SetName(index) => {
                let name = self.string_constant(index)?;
                let value = self.pop()?;
                set_name(&self.current_env(), &name, value.clone());
                self.push(value)?;
            }
            Instruction::TypeofName(index) => {
                let name = self.string_constant(index)?;
                match lookup_name(&self.current_env(), &name) {
                    Some(value) => self.push(Value::String(Rc::from(value.type_name())))?,
                    None => self.push(Value::String(Rc::from("undefined")))?,
                }
            }
            Instruction::GetProp(index) => {
                let name = self.string_constant(index)?;
                let object = self.pop()?;
                let value = self.vm.get_property_value(&object, &name).map_err(|err| {
                    err.at(
                        self.frames.last().expect("frame").proto.display_name(),
                        self.frames.last().expect("frame").ip,
                    )
                })?;
                self.push(value)?;
            }
            Instruction::SetProp(index) => {
                let name = self.string_constant(index)?;
                let value = self.pop()?;
                let object = self.pop()?;
                self.vm
                    .set_property_value(&object, &name, value.clone())
                    .map_err(|err| {
                        err.at(
                            self.frames.last().expect("frame").proto.display_name(),
                            self.frames.last().expect("frame").ip,
                        )
                    })?;
                self.push(value)?;
            }
            Instruction::GetIndex => {
                let key = self.pop()?;
                let object = self.pop()?;
                let value = self
                    .vm
                    .get_property_value(&object, &property_key(&key))
                    .map_err(|err| {
                        err.at(
                            self.frames.last().expect("frame").proto.display_name(),
                            self.frames.last().expect("frame").ip,
                        )
                    })?;
                self.push(value)?;
            }
            Instruction::SetIndex => {
                let value = self.pop()?;
                let key = self.pop()?;
                let object = self.pop()?;
                self.vm
                    .set_property_value(&object, &property_key(&key), value.clone())
                    .map_err(|err| {
                        err.at(
                            self.frames.last().expect("frame").proto.display_name(),
                            self.frames.last().expect("frame").ip,
                        )
                    })?;
                self.push(value)?;
            }
            Instruction::GetIndexKeep => {
                let length = self.stack.len();
                if length < 2 {
                    return Err(self.error("BytecodeError", "value stack underflow"));
                }
                let object = self.stack[length - 2].clone();
                let key = self.stack[length - 1].clone();
                let value = self
                    .vm
                    .get_property_value(&object, &property_key(&key))
                    .map_err(|err| {
                        err.at(
                            self.frames.last().expect("frame").proto.display_name(),
                            self.frames.last().expect("frame").ip,
                        )
                    })?;
                self.push(value)?;
            }
            Instruction::SetPropKeepOld(index) => {
                let name = self.string_constant(index)?;
                let new_value = self.pop()?;
                let old_value = self.pop()?;
                let object = self.pop()?;
                self.vm
                    .set_property_value(&object, &name, new_value)
                    .map_err(|err| {
                        err.at(
                            self.frames.last().expect("frame").proto.display_name(),
                            self.frames.last().expect("frame").ip,
                        )
                    })?;
                self.push(old_value)?;
            }
            Instruction::SetIndexKeepOld => {
                let new_value = self.pop()?;
                let old_value = self.pop()?;
                let key = self.pop()?;
                let object = self.pop()?;
                self.vm
                    .set_property_value(&object, &property_key(&key), new_value)
                    .map_err(|err| {
                        err.at(
                            self.frames.last().expect("frame").proto.display_name(),
                            self.frames.last().expect("frame").ip,
                        )
                    })?;
                self.push(old_value)?;
            }
            Instruction::MakeFunction(index) => {
                let constant = self.constant(index)?;
                let Constant::Function(proto) = constant else {
                    return Err(self.error(
                        "BytecodeError",
                        format!("constant {index} is not a function"),
                    ));
                };
                self.push(Value::Function(Rc::new(Function {
                    proto,
                    env: self.current_env(),
                })))?;
            }
            Instruction::Call(argc) => {
                let args = self.pop_args(argc)?;
                let callee = self.pop()?;
                self.call_value(callee, Value::Undefined, args)?;
            }
            Instruction::CallMethod(argc) => {
                let args = self.pop_args(argc)?;
                let callee = self.pop()?;
                let this_value = self.pop()?;
                self.call_value(callee, this_value, args)?;
            }
            Instruction::Jump(target) => {
                let target = target as usize;
                let proto = &self.frames.last().expect("frame").proto;
                if target > proto.code.len() {
                    return Err(self.error("BytecodeError", "jump target out of range"));
                }
                self.frames.last_mut().expect("frame").ip = target;
            }
            Instruction::JumpIfFalse(target) => {
                let condition = self.pop()?;
                if !condition.is_truthy() {
                    self.frames.last_mut().expect("frame").ip = target as usize;
                }
            }
            Instruction::JumpIfTrue(target) => {
                let condition = self.pop()?;
                if condition.is_truthy() {
                    self.frames.last_mut().expect("frame").ip = target as usize;
                }
            }
            Instruction::EnterScope(slots) => {
                let parent = self.current_env();
                let env = Environment::new(Some(parent), slots as usize, Vec::new());
                self.frames.last_mut().expect("frame").env = env;
            }
            Instruction::ExitScope => {
                let parent = self.current_env().borrow().parent.clone();
                let Some(parent) = parent else {
                    return Err(self.error("BytecodeError", "cannot exit the global scope"));
                };
                self.frames.last_mut().expect("frame").env = parent;
            }
            Instruction::DeclareLocal(slot, name) => {
                let name = self.string_constant(name)?;
                self.current_env().borrow_mut().declare_local(slot, &name);
            }
            Instruction::Return => {
                let value = self.pop()?;
                self.do_return(value)?;
            }
            Instruction::LoadThis => self.push(self.current_this())?,
            Instruction::Add => {
                let right = self.pop()?;
                let left = self.pop()?;
                let value = add_values(&left, &right);
                self.push(value)?;
            }
            Instruction::Sub => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number(to_number(&left) - to_number(&right)))?;
            }
            Instruction::Mul => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number(to_number(&left) * to_number(&right)))?;
            }
            Instruction::Div => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number(to_number(&left) / to_number(&right)))?;
            }
            Instruction::Rem => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number(to_number(&left) % to_number(&right)))?;
            }
            Instruction::Pow => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number(to_number(&left).powf(to_number(&right))))?;
            }
            Instruction::Eq => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(loose_equal(&left, &right)))?;
            }
            Instruction::Ne => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(!loose_equal(&left, &right)))?;
            }
            Instruction::StrictEq => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(strict_equal(&left, &right)))?;
            }
            Instruction::StrictNe => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(!strict_equal(&left, &right)))?;
            }
            Instruction::Lt => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(compare_values(&left, &right) < 0.0))?;
            }
            Instruction::Le => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(compare_values(&left, &right) <= 0.0))?;
            }
            Instruction::Gt => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(compare_values(&left, &right) > 0.0))?;
            }
            Instruction::Ge => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Bool(compare_values(&left, &right) >= 0.0))?;
            }
            Instruction::BitAnd => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number((to_int32(to_number(&left)) & to_int32(to_number(&right))) as f64))?;
            }
            Instruction::BitOr => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number((to_int32(to_number(&left)) | to_int32(to_number(&right))) as f64))?;
            }
            Instruction::BitXor => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.push(Value::Number((to_int32(to_number(&left)) ^ to_int32(to_number(&right))) as f64))?;
            }
            Instruction::Shl => {
                let right = self.pop()?;
                let left = self.pop()?;
                let shift = to_uint32(to_number(&right)) & 31;
                self.push(Value::Number((to_int32(to_number(&left)) << shift) as f64))?;
            }
            Instruction::Shr => {
                let right = self.pop()?;
                let left = self.pop()?;
                let shift = to_uint32(to_number(&right)) & 31;
                self.push(Value::Number((to_int32(to_number(&left)) >> shift) as f64))?;
            }
            Instruction::UShr => {
                let right = self.pop()?;
                let left = self.pop()?;
                let shift = to_uint32(to_number(&right)) & 31;
                self.push(Value::Number((to_uint32(to_number(&left)) >> shift) as f64))?;
            }
            Instruction::Not => {
                let value = self.pop()?;
                self.push(Value::Bool(!value.is_truthy()))?;
            }
            Instruction::Neg => {
                let value = self.pop()?;
                self.push(Value::Number(-to_number(&value)))?;
            }
            Instruction::Pos => {
                let value = self.pop()?;
                self.push(Value::Number(to_number(&value)))?;
            }
            Instruction::BitNot => {
                let value = self.pop()?;
                self.push(Value::Number((!to_int32(to_number(&value))) as f64))?;
            }
            Instruction::Typeof => {
                let value = self.pop()?;
                self.push(Value::String(Rc::from(value.type_name())))?;
            }
            Instruction::Void => {
                self.pop()?;
                self.push(Value::Undefined)?;
            }
            Instruction::Array(count) => {
                let values = self.pop_args(count)?;
                self.push(array_value(values))?;
            }
            Instruction::Object(count) => {
                let mut pairs = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let value = self.pop()?;
                    let key = property_key(&self.pop()?);
                    pairs.push((key, value));
                }
                pairs.reverse();
                self.push(object_value(pairs))?;
            }
        }
        Ok(())
    }

    fn call_value(
        &mut self,
        callee: Value,
        this_value: Value,
        args: Vec<Value>,
    ) -> Result<(), RuntimeError> {
        match callee {
            Value::Function(function) => {
                if self.frames.len() >= self.vm.max_call_depth {
                    return Err(self.error(
                        "RangeError",
                        format!("maximum call depth of {} exceeded", self.vm.max_call_depth),
                    ));
                }
                let env = Environment::new(
                    Some(function.env.clone()),
                    function.proto.local_names.len(),
                    function.proto.local_names.clone(),
                );
                for (index, arg) in args.into_iter().enumerate() {
                    if index < function.proto.param_count as usize {
                        env.borrow_mut().set_local(index as u16, arg);
                    }
                }
                if !function.proto.name.is_empty() {
                    env.borrow_mut().add_binding(
                        &function.proto.name,
                        Value::Function(function.clone()),
                    );
                }
                self.frames.push(Frame {
                    proto: function.proto.clone(),
                    env,
                    ip: 0,
                    this_value,
                });
                Ok(())
            }
            Value::NativeFunction(function) => {
                let result = (function.function)(self.vm, &this_value, &args)
                    .map_err(|err| err.at(
                        self.frames.last().expect("frame").proto.display_name(),
                        self.frames.last().expect("frame").ip,
                    ))?;
                self.push(result)
            }
            other => Err(self.error(
                "TypeError",
                format!("{} is not a function", to_string(&other)),
            )),
        }
    }

    fn do_return(&mut self, value: Value) -> Result<bool, RuntimeError> {
        self.frames.pop();
        if self.frames.is_empty() {
            self.stack.clear();
            self.stack.push(value);
            Ok(true)
        } else {
            self.push(value)?;
            Ok(false)
        }
    }
}

fn add_values(left: &Value, right: &Value) -> Value {
    let left_primitive = matches!(left, Value::Function(_) | Value::NativeFunction(_));
    let right_primitive = matches!(right, Value::Function(_) | Value::NativeFunction(_));
    let left_is_string = matches!(left, Value::String(_))
        || matches!(left, Value::Array(_))
        || matches!(left, Value::Object(_))
        || left_primitive;
    let right_is_string = matches!(right, Value::String(_))
        || matches!(right, Value::Array(_))
        || matches!(right, Value::Object(_))
        || right_primitive;
    if left_is_string || right_is_string {
        Value::String(Rc::from(format!(
            "{}{}",
            to_primitive_string(left),
            to_primitive_string(right)
        )))
    } else {
        Value::Number(to_number(left) + to_number(right))
    }
}

fn compare_values(left: &Value, right: &Value) -> f64 {
    if let (Value::String(a), Value::String(b)) = (left, right) {
        return match a.as_ref().cmp(b.as_ref()) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        };
    }
    let a = to_number(left);
    let b = to_number(right);
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b {
        -1.0
    } else if a > b {
        1.0
    } else {
        0.0
    }
}

impl JsRuntime {
    fn install_globals(&mut self) {
        let console = Object::new();
        {
            let mut console = console.borrow_mut();
            console.set("log", native("console.log", console_log));
            console.set("warn", native("console.warn", console_log));
            console.set("error", native("console.error", console_log));
        }
        self.define_global("console", Value::Object(console));

        let mut math_properties = Vec::new();
        math_properties.push(("PI".to_string(), Value::Number(std::f64::consts::PI)));
        math_properties.push(("E".to_string(), Value::Number(std::f64::consts::E)));
        for (name, function) in [
            ("floor", math_floor as NativeFn),
            ("ceil", math_ceil),
            ("round", math_round_fn),
            ("abs", math_abs),
            ("sqrt", math_sqrt),
            ("trunc", math_trunc),
            ("pow", math_pow),
            ("max", math_max),
            ("min", math_min),
        ] {
            math_properties.push((
                name.to_string(),
                native(&format!("Math.{name}"), function),
            ));
        }
        self.define_global("Math", object_value(math_properties));

        let object_ctor = Object::new();
        object_ctor
            .borrow_mut()
            .set("keys", native("Object.keys", object_keys));
        self.define_global("Object", Value::Object(object_ctor));

        let array_ctor = Object::new();
        array_ctor
            .borrow_mut()
            .set("isArray", native("Array.isArray", array_is_array));
        self.define_global("Array", Value::Object(array_ctor));

        for (name, function) in [
            ("setTimeout", timer_set_timeout as NativeFn),
            ("setInterval", timer_set_interval),
            ("clearTimeout", timer_clear),
            ("clearInterval", timer_clear),
            ("parseInt", global_parse_int),
            ("parseFloat", global_parse_float),
            ("isNaN", global_is_nan),
            ("isFinite", global_is_finite),
            ("String", global_string),
            ("Number", global_number),
            ("Boolean", global_boolean),
        ] {
            self.define_global(name, native(name, function));
        }
        self.define_global("NaN", Value::Number(f64::NAN));
        self.define_global("Infinity", Value::Number(f64::INFINITY));
    }
}

fn console_log(vm: &mut JsRuntime, _this: &Value, args: &[Value]) -> Result<Value, RuntimeError> {
    let mut parts = Vec::with_capacity(args.len());
    for arg in args {
        parts.push(to_string(arg));
    }
    let line = parts.join(" ");
    vm.notify_host_console(&line);
    vm.console.push(line);
    Ok(Value::Undefined)
}

fn timer_set_timeout(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let callback = args.first().cloned().unwrap_or(Value::Undefined);
    let delay = args.get(1).map(to_number).unwrap_or(0.0);
    let extra = args.get(2..).unwrap_or(&[]).to_vec();
    vm.add_timer(callback, delay, None, extra)
}

fn timer_set_interval(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let callback = args.first().cloned().unwrap_or(Value::Undefined);
    let delay = args.get(1).map(to_number).unwrap_or(0.0);
    let extra = args.get(2..).unwrap_or(&[]).to_vec();
    vm.add_timer(callback, delay, Some(delay), extra)
}

fn timer_clear(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    if let Some(id) = args.first() {
        vm.clear_timer(to_number(id));
    }
    Ok(Value::Undefined)
}

fn global_parse_int(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = args
        .first()
        .map(to_string)
        .unwrap_or_default()
        .trim()
        .to_string();
    let radix = args.get(1).map(to_number).unwrap_or(0.0) as u32;
    let (sign, unsigned) = if let Some(rest) = text.strip_prefix('-') {
        (-1.0, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (1.0, rest)
    } else {
        (1.0, text.as_str())
    };
    let (radix, digits) = if radix == 0 {
        if let Some(rest) = unsigned
            .strip_prefix("0x")
            .or_else(|| unsigned.strip_prefix("0X"))
        {
            (16, rest)
        } else {
            (10, unsigned)
        }
    } else if radix == 16 {
        (
            16,
            unsigned
                .strip_prefix("0x")
                .or_else(|| unsigned.strip_prefix("0X"))
                .unwrap_or(unsigned),
        )
    } else {
        (radix, unsigned)
    };
    if !(2..=36).contains(&radix) {
        return Ok(Value::Number(f64::NAN));
    }
    let mut value = 0.0f64;
    let mut any = false;
    for ch in digits.chars() {
        let Some(digit) = ch.to_digit(radix) else {
            break;
        };
        value = value * radix as f64 + digit as f64;
        any = true;
    }
    Ok(Value::Number(if any {
        value * sign
    } else {
        f64::NAN
    }))
}

fn global_parse_float(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = args
        .first()
        .map(to_string)
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(Value::Number(f64::NAN));
    }
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    if matches!(chars.get(index), Some('+') | Some('-')) {
        index += 1;
    }
    let mut digits = 0;
    while chars.get(index).is_some_and(|ch| ch.is_ascii_digit()) {
        index += 1;
        digits += 1;
    }
    if chars.get(index) == Some(&'.') {
        index += 1;
        while chars.get(index).is_some_and(|ch| ch.is_ascii_digit()) {
            index += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return Ok(Value::Number(f64::NAN));
    }
    if matches!(chars.get(index), Some('e') | Some('E')) {
        let exponent_start = index;
        index += 1;
        if matches!(chars.get(index), Some('+') | Some('-')) {
            index += 1;
        }
        let mut exponent_digits = 0;
        while chars.get(index).is_some_and(|ch| ch.is_ascii_digit()) {
            index += 1;
            exponent_digits += 1;
        }
        if exponent_digits == 0 {
            index = exponent_start;
        }
    }
    let literal: String = chars[..index].iter().collect();
    Ok(Value::Number(literal.parse::<f64>().unwrap_or(f64::NAN)))
}

fn global_is_nan(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        to_number(args.first().unwrap_or(&Value::Undefined)).is_nan(),
    ))
}

fn global_is_finite(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        to_number(args.first().unwrap_or(&Value::Undefined)).is_finite(),
    ))
}

fn global_string(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::String(Rc::from(
        args.first().map(to_string).unwrap_or_default(),
    )))
}

fn global_number(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Number(
        args.first().map(to_number).unwrap_or(0.0),
    ))
}

fn global_boolean(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(
        args.first().is_some_and(Value::is_truthy),
    ))
}

fn object_keys(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let keys = match args.first() {
        Some(Value::Object(object)) => object
            .borrow()
            .properties
            .iter()
            .map(|(key, _)| Value::String(Rc::from(key.as_str())))
            .collect(),
        Some(Value::Array(values)) => (0..values.borrow().len())
            .map(|index| Value::String(Rc::from(index.to_string())))
            .collect(),
        _ => Vec::new(),
    };
    Ok(array_value(keys))
}

fn array_is_array(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Bool(matches!(
        args.first(),
        Some(Value::Array(_))
    )))
}

fn math_floor(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Number(
        args.first().map(to_number).unwrap_or(f64::NAN).floor(),
    ))
}

fn math_ceil(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Number(
        args.first().map(to_number).unwrap_or(f64::NAN).ceil(),
    ))
}

fn math_round_fn(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let value = args.first().map(to_number).unwrap_or(f64::NAN);
    Ok(Value::Number(if value.is_finite() {
        (value + 0.5).floor()
    } else {
        value
    }))
}

fn math_abs(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Number(
        args.first().map(to_number).unwrap_or(f64::NAN).abs(),
    ))
}

fn math_sqrt(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Number(
        args.first().map(to_number).unwrap_or(f64::NAN).sqrt(),
    ))
}

fn math_trunc(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::Number(
        args.first().map(to_number).unwrap_or(f64::NAN).trunc(),
    ))
}

fn math_pow(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let base = args.first().map(to_number).unwrap_or(f64::NAN);
    let exponent = args.get(1).map(to_number).unwrap_or(f64::NAN);
    Ok(Value::Number(base.powf(exponent)))
}

fn math_max(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let mut result = f64::NEG_INFINITY;
    for arg in args {
        let value = to_number(arg);
        if value.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        result = result.max(value);
    }
    Ok(Value::Number(result))
}

fn math_min(
    _vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let mut result = f64::INFINITY;
    for arg in args {
        let value = to_number(arg);
        if value.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        result = result.min(value);
    }
    Ok(Value::Number(result))
}

fn array_this(this: &Value) -> Result<Rc<RefCell<Vec<Value>>>, RuntimeError> {
    match this {
        Value::Array(values) => Ok(values.clone()),
        other => Err(RuntimeError::new(
            "TypeError",
            format!("array method called on {}", other.type_name()),
        )),
    }
}

fn array_push(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let values = array_this(this)?;
    let mut values = values.borrow_mut();
    values.extend_from_slice(args);
    Ok(Value::Number(values.len() as f64))
}

fn array_pop(
    _vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    let values = array_this(this)?;
    let value = values.borrow_mut().pop().unwrap_or(Value::Undefined);
    Ok(value)
}

fn array_join(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let values = array_this(this)?;
    let separator = args
        .first()
        .filter(|value| !value.is_undefined())
        .map(to_string)
        .unwrap_or_else(|| ",".to_string());
    let values = values.borrow();
    let mut parts = Vec::with_capacity(values.len());
    for value in values.iter() {
        parts.push(match value {
            Value::Undefined | Value::Null => String::new(),
            other => to_string(other),
        });
    }
    Ok(Value::String(Rc::from(parts.join(&separator))))
}

fn array_index_of(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let values = array_this(this)?;
    let needle = args.first().cloned().unwrap_or(Value::Undefined);
    let start = args.get(1).map(to_number).unwrap_or(0.0);
    let values = values.borrow();
    let start = if start < 0.0 {
        (values.len() as f64 + start).max(0.0) as usize
    } else {
        start as usize
    };
    for (index, value) in values.iter().enumerate().skip(start) {
        if strict_equal(value, &needle) {
            return Ok(Value::Number(index as f64));
        }
    }
    Ok(Value::Number(-1.0))
}

fn array_slice(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let values = array_this(this)?;
    let values = values.borrow();
    let length = values.len() as i64;
    let start = normalize_slice_index(args.first().map(to_number).unwrap_or(0.0), length);
    let end = normalize_slice_index(
        args.get(1).map(to_number).unwrap_or(length as f64),
        length,
    );
    let slice = if end <= start {
        Vec::new()
    } else {
        values[start as usize..end as usize].to_vec()
    };
    Ok(array_value(slice))
}

fn array_to_string(
    _vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::String(Rc::from(to_string(this))))
}

fn normalize_slice_index(value: f64, length: i64) -> i64 {
    let index = if value.is_nan() {
        0
    } else {
        value.trunc() as i64
    };
    if index < 0 {
        (length + index).clamp(0, length)
    } else {
        index.clamp(0, length)
    }
}

fn string_this(this: &Value) -> Result<String, RuntimeError> {
    match this {
        Value::String(value) => Ok(value.to_string()),
        other => Err(RuntimeError::new(
            "TypeError",
            format!("string method called on {}", other.type_name()),
        )),
    }
}

fn string_char_at(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let index = args.first().map(to_number).unwrap_or(0.0).trunc().max(0.0) as usize;
    Ok(Value::String(Rc::from(
        text.chars().nth(index).map(String::from).unwrap_or_default(),
    )))
}

fn string_to_upper(
    _vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::String(Rc::from(string_this(this)?.to_uppercase())))
}

fn string_to_lower(
    _vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::String(Rc::from(string_this(this)?.to_lowercase())))
}

fn string_index_of(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let needle = args.first().map(to_string).unwrap_or_default();
    let from = args.get(1).map(to_number).unwrap_or(0.0).max(0.0) as usize;
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return Ok(Value::Number(from.min(chars.len()) as f64));
    }
    let mut index = from;
    while index + needle.len() <= chars.len() {
        if chars[index..index + needle.len()] == needle {
            return Ok(Value::Number(index as f64));
        }
        index += 1;
    }
    Ok(Value::Number(-1.0))
}

fn string_slice(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let chars: Vec<char> = text.chars().collect();
    let length = chars.len() as i64;
    let start = normalize_slice_index(args.first().map(to_number).unwrap_or(0.0), length);
    let end = normalize_slice_index(
        args.get(1).map(to_number).unwrap_or(length as f64),
        length,
    );
    let slice: String = if end <= start {
        String::new()
    } else {
        chars[start as usize..end as usize].iter().collect()
    };
    Ok(Value::String(Rc::from(slice)))
}

fn string_split(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let separator = args.first().filter(|value| !value.is_undefined()).map(to_string);
    let parts = match separator.as_deref() {
        None => vec![Value::String(Rc::from(text))],
        Some("") => text
            .chars()
            .map(|ch| Value::String(Rc::from(ch.to_string())))
            .collect(),
        Some(separator) => text
            .split(separator)
            .map(|part| Value::String(Rc::from(part)))
            .collect(),
    };
    Ok(array_value(parts))
}

fn string_trim(
    _vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::String(Rc::from(string_this(this)?.trim())))
}

fn string_includes(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let needle = args.first().map(to_string).unwrap_or_default();
    Ok(Value::Bool(text.contains(&needle)))
}

fn string_starts_with(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let needle = args.first().map(to_string).unwrap_or_default();
    Ok(Value::Bool(text.starts_with(&needle)))
}

fn string_ends_with(
    _vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = string_this(this)?;
    let needle = args.first().map(to_string).unwrap_or_default();
    Ok(Value::Bool(text.ends_with(&needle)))
}

fn primitive_to_string(
    _vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    Ok(Value::String(Rc::from(to_string(this))))
}

fn receiver_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        other => other.type_name(),
    }
}

impl JsRuntime {
    fn get_property_value(&mut self, object: &Value, name: &str) -> Result<Value, RuntimeError> {
        match object {
            Value::Undefined | Value::Null => Err(RuntimeError::new(
                "TypeError",
                format!("cannot read property '{name}' of {}", receiver_name(object)),
            )),
            Value::Object(object) => {
                let host_binding = object.borrow().host;
                if let Some(binding) = host_binding {
                    return host_get_property(self, object, binding, name);
                }
                Ok(object
                    .borrow()
                    .get(name)
                    .cloned()
                    .unwrap_or(Value::Undefined))
            }
            Value::Array(values) => {
                if name == "length" {
                    return Ok(Value::Number(values.borrow().len() as f64));
                }
                if let Some(index) = parse_array_index(name) {
                    return Ok(values
                        .borrow()
                        .get(index)
                        .cloned()
                        .unwrap_or(Value::Undefined));
                }
                Ok(match name {
                    "push" => native("Array.push", array_push),
                    "pop" => native("Array.pop", array_pop),
                    "join" => native("Array.join", array_join),
                    "indexOf" => native("Array.indexOf", array_index_of),
                    "slice" => native("Array.slice", array_slice),
                    "toString" => native("Array.toString", array_to_string),
                    _ => Value::Undefined,
                })
            }
            Value::String(text) => {
                if name == "length" {
                    return Ok(Value::Number(text.chars().count() as f64));
                }
                if let Some(index) = parse_array_index(name) {
                    return Ok(Value::String(Rc::from(
                        text.chars().nth(index).map(String::from).unwrap_or_default(),
                    )));
                }
                Ok(match name {
                    "charAt" => native("String.charAt", string_char_at),
                    "toUpperCase" => native("String.toUpperCase", string_to_upper),
                    "toLowerCase" => native("String.toLowerCase", string_to_lower),
                    "indexOf" => native("String.indexOf", string_index_of),
                    "slice" => native("String.slice", string_slice),
                    "split" => native("String.split", string_split),
                    "trim" => native("String.trim", string_trim),
                    "includes" => native("String.includes", string_includes),
                    "startsWith" => native("String.startsWith", string_starts_with),
                    "endsWith" => native("String.endsWith", string_ends_with),
                    "toString" => native("String.toString", primitive_to_string),
                    _ => Value::Undefined,
                })
            }
            Value::Function(function) => Ok(match name {
                "name" => Value::String(Rc::from(function.proto.name.as_str())),
                "length" => Value::Number(function.proto.param_count as f64),
                _ => Value::Undefined,
            }),
            Value::NativeFunction(function) => Ok(match name {
                "name" => Value::String(Rc::from(function.name.as_str())),
                _ => Value::Undefined,
            }),
            Value::Number(_) | Value::Bool(_) => Ok(match name {
                "toString" => native("Number.toString", primitive_to_string),
                _ => Value::Undefined,
            }),
        }
    }

    fn set_property_value(
        &mut self,
        object: &Value,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        match object {
            Value::Undefined | Value::Null => Err(RuntimeError::new(
                "TypeError",
                format!("cannot set property '{name}' of {}", receiver_name(object)),
            )),
            Value::Object(object) => {
                let host_binding = object.borrow().host;
                if let Some(binding) = host_binding {
                    return host_set_property(self, object, binding, name, value);
                }
                object.borrow_mut().set(name, value);
                Ok(())
            }
        Value::Array(values) => {
            if name == "length" {
                let length = to_number(&value);
                if !length.is_finite() || length < 0.0 || length > 1_000_000.0 {
                    return Err(RuntimeError::new(
                        "RangeError",
                        "invalid array length",
                    ));
                }
                let length = length as usize;
                let mut values = values.borrow_mut();
                values.resize(length, Value::Undefined);
                return Ok(());
            }
            if let Some(index) = parse_array_index(name) {
                if index > 1_000_000 {
                    return Err(RuntimeError::new(
                        "RangeError",
                        "array index is too large",
                    ));
                }
                let mut values = values.borrow_mut();
                if values.len() <= index {
                    values.resize(index + 1, Value::Undefined);
                }
                values[index] = value;
            }
            Ok(())
        }
            Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
            Value::Function(_) | Value::NativeFunction(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> (JsRuntime, String) {
        let mut runtime = JsRuntime::new();
        runtime.run_script(source).expect("script runs");
        let output = runtime.console_log().join("\n");
        (runtime, output)
    }

    #[test]
    fn arithmetic_strings_and_coercion() {
        let (_runtime, output) = run(
            "console.log(2 + 3 * 4, (2 + 3) * 4, 2 ** 3, 7 % 4);\n\
             console.log('Привет, ' + 'мир', 'x' + 1, 1 + 'x');\n\
             console.log(true + 1, null + 1, undefined + 1, '5' * 2);\n\
             console.log(1 == '1', 1 === '1', null == undefined, null === undefined);\n\
             console.log(0.1 + 0.2 === 0.3, !0, !'', typeof 1, typeof 'x', typeof missing);",
        );
        assert_eq!(
            output,
            "14 20 8 3\nПривет, мир x1 1x\n2 1 NaN 10\ntrue false true false\nfalse true true number string undefined"
        );
    }

    #[test]
    fn closures_and_recursion() {
        let (_runtime, output) = run(
            "function makeCounter() {\n\
               let n = 0;\n\
               return function () { n += 1; return n; };\n\
             }\n\
             let counter = makeCounter();\n\
             counter(); counter();\n\
             function fact(n) { if (n <= 1) return 1; return n * fact(n - 1); }\n\
             function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }\n\
             console.log(counter(), fact(6), fib(10));",
        );
        assert_eq!(output, "3 720 55");
    }

    #[test]
    fn arrays_objects_and_this_methods() {
        let (_runtime, output) = run(
            "let a = [1, 2, 3];\n\
             a.push(4); a[0] = 9;\n\
             let o = { x: 1, inc: function () { this.x += 1; return this.x; } };\n\
             o.inc(); o['x'] += 3;\n\
             let s = 'Привет';\n\
             console.log(a.join('-'), a.length, a.pop(), a.length);\n\
             console.log(o.x, o['x'], s.length, s.toUpperCase(), s.slice(0, 3));",
        );
        assert_eq!(
            output,
            "9-2-3-4 4 4 3\n5 5 6 ПРИВЕТ При"
        );
    }

    #[test]
    fn loops_break_continue_and_ternary() {
        let (_runtime, output) = run(
            "let sum = 0;\n\
             for (let i = 0; i < 10; i++) {\n\
               if (i === 3) continue;\n\
               if (i === 7) break;\n\
               sum += i;\n\
             }\n\
             let n = 3; let while_sum = 0;\n\
             while (n > 0) { while_sum += n; n--; }\n\
             let label = sum > 10 ? 'big' : 'small';\n\
             console.log(sum, while_sum, label);",
        );
        assert_eq!(output, "18 6 big");
    }

    #[test]
    fn timers_are_deterministic_and_cancellable() {
        let mut runtime = JsRuntime::new();
        runtime
            .run_script(
                "let out = [];\n\
                 let canceled = setTimeout(function () { out.push('never'); }, 1);\n\
                 clearTimeout(canceled);\n\
                 setTimeout(function () { out.push('late'); }, 20);\n\
                 setTimeout(function () { out.push('early'); }, 5);\n\
                 let interval = setInterval(function () { out.push('tick'); }, 10);\n\
                 setTimeout(function () { clearInterval(interval); }, 25);",
            )
            .expect("schedule");
        assert_eq!(runtime.pending_timer_count(), 4);
        assert!(runtime.tick(0.0).is_empty());
        let first = runtime.tick(5.0);
        assert_eq!(first.len(), 1);
        assert!(runtime.console_log().is_empty());
        let second = runtime.tick(10.0);
        assert_eq!(second.len(), 1);
        runtime.tick(20.0);
        runtime.tick(25.0);
        runtime.tick(30.0);
        let out = runtime.get_global("out").expect("out");
        assert_eq!(to_string(&out), "early,tick,late,tick");
    }

    #[test]
    fn instruction_and_call_depth_limits_are_hard() {
        let mut runtime = JsRuntime::new();
        runtime.set_instruction_limit(2_000);
        let err = runtime
            .run_script("while (true) {}")
            .expect_err("instruction limit");
        assert!(err.to_string().contains("instruction limit"));

        let mut runtime = JsRuntime::new();
        runtime.set_call_depth_limit(32);
        let err = runtime
            .run_script("function f() { return f(); } f();")
            .expect_err("call depth");
        assert!(err.to_string().contains("maximum call depth"));

        let mut runtime = JsRuntime::new();
        runtime.set_timer_limit(2);
        let err = runtime
            .run_script(
                "setTimeout(function () {}, 1);\
                 setTimeout(function () {}, 1);\
                 setTimeout(function () {}, 1);",
            )
            .expect_err("timer limit");
        assert!(err.to_string().contains("too many pending timers"));
        assert_eq!(runtime.pending_timer_count(), 2);
    }

    #[test]
    fn runtime_errors_are_descriptive_and_syntax_errors_carry_positions() {
        let mut runtime = JsRuntime::new();
        let err = runtime.run_script("let x = ;").expect_err("syntax");
        assert!(err.to_string().contains("line 1, column 9"));

        let err = runtime.run_script("null.foo").expect_err("runtime");
        assert!(err.to_string().contains("TypeError"));
        assert!(err.to_string().contains("null"));

        let err = runtime.run_script("missingName").expect_err("reference");
        assert!(err.to_string().contains("ReferenceError"));
        assert!(err.to_string().contains("missingName"));
    }

    #[test]
    fn function_hoisting_and_var_scope() {
        let (_runtime, output) = run(
            "console.log(typeof x);\n\
             var x = 1;\n\
             console.log(hoisted());\n\
             function hoisted() { if (x) { var y = 2; return y; } return 0; }\n\
             for (var i = 0; i < 2; i++) {}\n\
             console.log(x, i);",
        );
        assert_eq!(output, "undefined\n2\n1 2");
    }
}
