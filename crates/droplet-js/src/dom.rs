use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::vm::{array_value, native, strict_equal, to_string, JsRuntime, Object, Value};

/// Maximum number of DOM event listeners registered on a single `JsRuntime`.
pub const MAX_DOM_LISTENERS: usize = 1024;
/// Maximum ancestor depth traversed during DOM event bubbling.
pub const MAX_BUBBLE_DEPTH: usize = 4096;

/// Opaque DOM arena node identifier exchanged between `droplet-js` and the host.
pub type DomNodeId = usize;

/// Host-side DOM interface implemented by the browser shell (or a test mock).
///
/// Decouples `droplet-js` from `droplet-dom` / `droplet-shell` while enabling live
/// queries, tree mutations, attributes, `classList`, inline `style`, forms, and logs.
pub trait DomHost {
    fn get_element_by_id(&mut self, id: &str) -> Option<DomNodeId>;
    fn query_selector(&mut self, selector: &str) -> Option<DomNodeId>;
    fn query_selector_all(&mut self, selector: &str) -> Vec<DomNodeId>;
    fn parent_node(&self, node: DomNodeId) -> Option<DomNodeId>;
    fn document_body(&self) -> Option<DomNodeId>;
    fn document_title(&self) -> String;
    fn set_document_title(&mut self, title: &str);

    fn create_element(&mut self, tag: &str) -> DomNodeId;
    fn create_text_node(&mut self, text: &str) -> DomNodeId;
    fn append_child(&mut self, parent: DomNodeId, child: DomNodeId) -> bool;
    fn remove_child(&mut self, parent: DomNodeId, child: DomNodeId) -> bool;

    fn tag_name(&self, node: DomNodeId) -> Option<String>;
    fn text_content(&self, node: DomNodeId) -> String;
    fn set_text_content(&mut self, node: DomNodeId, text: &str);
    fn inner_html(&self, node: DomNodeId) -> String;
    fn set_inner_html(&mut self, node: DomNodeId, html: &str);
    fn get_attribute(&self, node: DomNodeId, name: &str) -> Option<String>;
    fn set_attribute(&mut self, node: DomNodeId, name: &str, value: &str);
    fn remove_attribute(&mut self, node: DomNodeId, name: &str);
    fn has_attribute(&self, node: DomNodeId, name: &str) -> bool;

    fn class_list_contains(&self, node: DomNodeId, class_name: &str) -> bool;
    fn class_list_add(&mut self, node: DomNodeId, class_name: &str);
    fn class_list_remove(&mut self, node: DomNodeId, class_name: &str);
    fn class_list_toggle(&mut self, node: DomNodeId, class_name: &str) -> bool;
    fn get_style_property(&self, node: DomNodeId, prop: &str) -> String;
    fn set_style_property(&mut self, node: DomNodeId, prop: &str, value: &str);

    fn form_value(&self, node: DomNodeId) -> Option<String>;
    fn set_form_value(&mut self, node: DomNodeId, value: &str);
    fn form_checked(&self, node: DomNodeId) -> Option<bool>;
    fn set_form_checked(&mut self, node: DomNodeId, checked: bool);
    fn focus(&mut self, node: DomNodeId);
    fn blur(&mut self, node: DomNodeId);

    fn console_log(&mut self, line: &str);
}

/// Input parameters describing a DOM event dispatched into JS listeners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomEventInput {
    pub event_type: String,
    pub target: DomNodeId,
    pub key: String,
    pub value: Option<String>,
    pub checked: Option<bool>,
}

/// Outcome flags produced by dispatching a DOM event through bubbling listeners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DomEventOutcome {
    pub default_prevented: bool,
    pub propagation_stopped: bool,
    pub invoked_listeners: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostBinding {
    Window,
    Document,
    Element(DomNodeId),
    ClassList(DomNodeId),
    Style(DomNodeId),
    Event(u64),
}

#[derive(Clone)]
pub(crate) struct DomListenerEntry {
    pub node_id: DomNodeId,
    pub event_type: String,
    pub listener: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct EventState {
    pub id: u64,
    pub event_type: String,
    pub target: DomNodeId,
    pub current_target: DomNodeId,
    pub key: String,
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub default_prevented: bool,
    pub propagation_stopped: bool,
}

pub(crate) fn make_host_object(binding: HostBinding) -> Rc<RefCell<Object>> {
    Rc::new(RefCell::new(Object {
        properties: Vec::new(),
        host: Some(binding),
    }))
}

pub(crate) fn host_get_property(
    vm: &mut JsRuntime,
    object: &Rc<RefCell<Object>>,
    binding: HostBinding,
    name: &str,
) -> Result<Value, RuntimeError> {
    match binding {
        HostBinding::Window => {
            if matches!(name, "window" | "self" | "globalThis") {
                return Ok(Value::Object(object.clone()));
            }
            if let Some(local) = object.borrow().get(name).cloned() {
                return Ok(local);
            }
            Ok(vm.get_global(name).unwrap_or(Value::Undefined))
        }
        HostBinding::Document => match name {
            "body" => {
                let body = vm.with_active_host(|host| host.document_body())?;
                Ok(match body {
                    Some(id) => vm.dom_element_value(id),
                    None => Value::Null,
                })
            }
            "title" => {
                let title = vm.with_active_host(|host| host.document_title())?;
                Ok(Value::String(Rc::from(title)))
            }
            "getElementById" => Ok(native("document.getElementById", doc_get_element_by_id)),
            "querySelector" => Ok(native("document.querySelector", doc_query_selector)),
            "querySelectorAll" => Ok(native("document.querySelectorAll", doc_query_selector_all)),
            "createElement" => Ok(native("document.createElement", doc_create_element)),
            "createTextNode" => Ok(native("document.createTextNode", doc_create_text_node)),
            "addEventListener" => Ok(native("document.addEventListener", el_add_event_listener)),
            "removeEventListener" => Ok(native(
                "document.removeEventListener",
                el_remove_event_listener,
            )),
            _ => Ok(object
                .borrow()
                .get(name)
                .cloned()
                .unwrap_or(Value::Undefined)),
        },
        HostBinding::Element(node_id) => match name {
            "__droplet_node_id__" => Ok(Value::Number(node_id as f64)),
            "id" => {
                let id = vm.with_active_host(|host| host.get_attribute(node_id, "id"))?;
                Ok(Value::String(Rc::from(id.unwrap_or_default())))
            }
            "tagName" => {
                let tag = vm.with_active_host(|host| host.tag_name(node_id))?;
                Ok(Value::String(Rc::from(
                    tag.map(|t| t.to_ascii_uppercase()).unwrap_or_default(),
                )))
            }
            "textContent" => {
                let text = vm.with_active_host(|host| host.text_content(node_id))?;
                Ok(Value::String(Rc::from(text)))
            }
            "innerHTML" => {
                let html = vm.with_active_host(|host| host.inner_html(node_id))?;
                Ok(Value::String(Rc::from(html)))
            }
            "value" => {
                let val = vm.with_active_host(|host| host.form_value(node_id))?;
                match val {
                    Some(v) => Ok(Value::String(Rc::from(v))),
                    None => Ok(object
                        .borrow()
                        .get("value")
                        .cloned()
                        .unwrap_or(Value::Undefined)),
                }
            }
            "checked" => {
                let checked = vm.with_active_host(|host| host.form_checked(node_id))?;
                match checked {
                    Some(b) => Ok(Value::Bool(b)),
                    None => Ok(object
                        .borrow()
                        .get("checked")
                        .cloned()
                        .unwrap_or(Value::Undefined)),
                }
            }
            "parentNode" | "parentElement" => {
                let parent = vm.with_active_host(|host| host.parent_node(node_id))?;
                Ok(match parent {
                    Some(pid) => vm.dom_element_value(pid),
                    None => Value::Null,
                })
            }
            "className" => {
                let cls = vm.with_active_host(|host| host.get_attribute(node_id, "class"))?;
                Ok(Value::String(Rc::from(cls.unwrap_or_default())))
            }
            "classList" => Ok(vm.dom_class_list_value(node_id)),
            "style" => Ok(vm.dom_style_value(node_id)),
            "getAttribute" => Ok(native("Element.getAttribute", el_get_attribute)),
            "setAttribute" => Ok(native("Element.setAttribute", el_set_attribute)),
            "removeAttribute" => Ok(native("Element.removeAttribute", el_remove_attribute)),
            "hasAttribute" => Ok(native("Element.hasAttribute", el_has_attribute)),
            "appendChild" => Ok(native("Element.appendChild", el_append_child)),
            "removeChild" => Ok(native("Element.removeChild", el_remove_child)),
            "addEventListener" => Ok(native("Element.addEventListener", el_add_event_listener)),
            "removeEventListener" => Ok(native(
                "Element.removeEventListener",
                el_remove_event_listener,
            )),
            "focus" => Ok(native("Element.focus", el_focus)),
            "blur" => Ok(native("Element.blur", el_blur)),
            "querySelector" => Ok(native("Element.querySelector", doc_query_selector)),
            "querySelectorAll" => Ok(native("Element.querySelectorAll", doc_query_selector_all)),
            _ => Ok(object
                .borrow()
                .get(name)
                .cloned()
                .unwrap_or(Value::Undefined)),
        },
        HostBinding::ClassList(_) => Ok(match name {
            "add" => native("DOMTokenList.add", class_list_add),
            "remove" => native("DOMTokenList.remove", class_list_remove),
            "toggle" => native("DOMTokenList.toggle", class_list_toggle),
            "contains" => native("DOMTokenList.contains", class_list_contains),
            _ => object
                .borrow()
                .get(name)
                .cloned()
                .unwrap_or(Value::Undefined),
        }),
        HostBinding::Style(node_id) => match name {
            "getPropertyValue" => Ok(native("CSSStyleDeclaration.getPropertyValue", style_get_prop)),
            "setProperty" => Ok(native("CSSStyleDeclaration.setProperty", style_set_prop)),
            _ => {
                let value = vm.with_active_host(|host| host.get_style_property(node_id, name))?;
                Ok(Value::String(Rc::from(value)))
            }
        },
        HostBinding::Event(event_id) => {
            if name == "preventDefault" {
                return Ok(native("Event.preventDefault", event_prevent_default));
            }
            if name == "stopPropagation" {
                return Ok(native("Event.stopPropagation", event_stop_propagation));
            }
            let Some(state) = vm.event_state(event_id).cloned() else {
                return Ok(Value::Undefined);
            };
            Ok(match name {
                "type" => Value::String(Rc::from(state.event_type)),
                "target" => vm.dom_element_value(state.target),
                "currentTarget" => vm.dom_element_value(state.current_target),
                "key" => Value::String(Rc::from(state.key)),
                "value" => match state.value {
                    Some(v) => Value::String(Rc::from(v)),
                    None => Value::Undefined,
                },
                "checked" => match state.checked {
                    Some(b) => Value::Bool(b),
                    None => Value::Undefined,
                },
                "defaultPrevented" => Value::Bool(state.default_prevented),
                "cancelBubble" => Value::Bool(state.propagation_stopped),
                _ => object
                    .borrow()
                    .get(name)
                    .cloned()
                    .unwrap_or(Value::Undefined),
            })
        }
    }
}

pub(crate) fn host_set_property(
    vm: &mut JsRuntime,
    object: &Rc<RefCell<Object>>,
    binding: HostBinding,
    name: &str,
    value: Value,
) -> Result<(), RuntimeError> {
    match binding {
        HostBinding::Window => {
            vm.define_global(name, value);
            Ok(())
        }
        HostBinding::Document => match name {
            "title" => {
                let text = to_string(&value);
                vm.with_active_host(|host| host.set_document_title(&text))?;
                Ok(())
            }
            _ => {
                object.borrow_mut().set(name, value);
                Ok(())
            }
        },
        HostBinding::Element(node_id) => match name {
            "id" => {
                let text = to_string(&value);
                vm.with_active_host(|host| host.set_attribute(node_id, "id", &text))?;
                Ok(())
            }
            "className" => {
                let text = to_string(&value);
                vm.with_active_host(|host| host.set_attribute(node_id, "class", &text))?;
                Ok(())
            }
            "textContent" => {
                let text = to_string(&value);
                vm.with_active_host(|host| host.set_text_content(node_id, &text))?;
                Ok(())
            }
            "innerHTML" => {
                let html = to_string(&value);
                vm.with_active_host(|host| host.set_inner_html(node_id, &html))?;
                Ok(())
            }
            "value" => {
                let text = to_string(&value);
                let is_form = vm.with_active_host(|host| {
                    let existing = host.form_value(node_id).is_some();
                    host.set_form_value(node_id, &text);
                    existing
                })?;
                if !is_form {
                    object.borrow_mut().set("value", value);
                }
                Ok(())
            }
            "checked" => {
                let checked = value.is_truthy();
                let is_form = vm.with_active_host(|host| {
                    let existing = host.form_checked(node_id).is_some();
                    host.set_form_checked(node_id, checked);
                    existing
                })?;
                if !is_form {
                    object.borrow_mut().set("checked", Value::Bool(checked));
                }
                Ok(())
            }
            _ => {
                object.borrow_mut().set(name, value);
                Ok(())
            }
        },
        HostBinding::ClassList(_) => {
            object.borrow_mut().set(name, value);
            Ok(())
        }
        HostBinding::Style(node_id) => {
            let text = to_string(&value);
            vm.with_active_host(|host| host.set_style_property(node_id, name, &text))?;
            Ok(())
        }
        HostBinding::Event(_) => {
            object.borrow_mut().set(name, value);
            Ok(())
        }
    }
}

fn receiver_host_binding(this: &Value) -> Option<HostBinding> {
    let Value::Object(object) = this else {
        return None;
    };
    object.borrow().host
}

fn receiver_node_id(this: &Value, allow_document: bool) -> Result<DomNodeId, RuntimeError> {
    match receiver_host_binding(this) {
        Some(HostBinding::Element(id)) => Ok(id),
        Some(HostBinding::Document) if allow_document => Ok(0),
        _ => Err(RuntimeError::new(
            "TypeError",
            "DOM method called on non-Element receiver",
        )),
    }
}

fn arg_node_id(value: Option<&Value>, label: &str) -> Result<DomNodeId, RuntimeError> {
    let Some(Value::Object(object)) = value else {
        return Err(RuntimeError::new(
            "TypeError",
            format!("{label} must be a DOM node"),
        ));
    };
    match object.borrow().host {
        Some(HostBinding::Element(id)) => Ok(id),
        _ => Err(RuntimeError::new(
            "TypeError",
            format!("{label} must be a DOM node"),
        )),
    }
}

fn doc_get_element_by_id(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let id = args.first().map(to_string).unwrap_or_default();
    if id.is_empty() {
        return Ok(Value::Null);
    }
    let found = vm.with_active_host(|host| host.get_element_by_id(&id))?;
    Ok(match found {
        Some(node_id) => vm.dom_element_value(node_id),
        None => Value::Null,
    })
}

fn doc_query_selector(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let selector = args.first().map(to_string).unwrap_or_default();
    if selector.trim().is_empty() {
        return Ok(Value::Null);
    }
    let found = vm.with_active_host(|host| host.query_selector(&selector))?;
    Ok(match found {
        Some(node_id) => vm.dom_element_value(node_id),
        None => Value::Null,
    })
}

fn doc_query_selector_all(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let selector = args.first().map(to_string).unwrap_or_default();
    if selector.trim().is_empty() {
        return Ok(array_value(Vec::new()));
    }
    let nodes = vm.with_active_host(|host| host.query_selector_all(&selector))?;
    let wrapped = nodes
        .into_iter()
        .map(|node_id| vm.dom_element_value(node_id))
        .collect();
    Ok(array_value(wrapped))
}

fn doc_create_element(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let tag = args.first().map(to_string).unwrap_or_default();
    if tag.trim().is_empty() {
        return Err(RuntimeError::new(
            "TypeError",
            "createElement requires a tag name",
        ));
    }
    let node_id = vm.with_active_host(|host| host.create_element(&tag))?;
    Ok(vm.dom_element_value(node_id))
}

fn doc_create_text_node(
    vm: &mut JsRuntime,
    _this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let text = args.first().map(to_string).unwrap_or_default();
    let node_id = vm.with_active_host(|host| host.create_text_node(&text))?;
    Ok(vm.dom_element_value(node_id))
}

fn el_get_attribute(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, false)?;
    let name = args.first().map(to_string).unwrap_or_default();
    let val = vm.with_active_host(|host| host.get_attribute(node_id, &name))?;
    Ok(match val {
        Some(s) => Value::String(Rc::from(s)),
        None => Value::Null,
    })
}

fn el_set_attribute(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, false)?;
    let name = args.first().map(to_string).unwrap_or_default();
    let value = args.get(1).map(to_string).unwrap_or_default();
    if !name.is_empty() {
        vm.with_active_host(|host| host.set_attribute(node_id, &name, &value))?;
    }
    Ok(Value::Undefined)
}

fn el_remove_attribute(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, false)?;
    let name = args.first().map(to_string).unwrap_or_default();
    if !name.is_empty() {
        vm.with_active_host(|host| host.remove_attribute(node_id, &name))?;
    }
    Ok(Value::Undefined)
}

fn el_has_attribute(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, false)?;
    let name = args.first().map(to_string).unwrap_or_default();
    let has = vm.with_active_host(|host| host.has_attribute(node_id, &name))?;
    Ok(Value::Bool(has))
}

fn el_append_child(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let parent_id = receiver_node_id(this, false)?;
    let child_val = args.first().cloned().unwrap_or(Value::Undefined);
    let child_id = arg_node_id(Some(&child_val), "appendChild argument")?;
    vm.with_active_host(|host| host.append_child(parent_id, child_id))?;
    Ok(child_val)
}

fn el_remove_child(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let parent_id = receiver_node_id(this, false)?;
    let child_val = args.first().cloned().unwrap_or(Value::Undefined);
    let child_id = arg_node_id(Some(&child_val), "removeChild argument")?;
    vm.with_active_host(|host| host.remove_child(parent_id, child_id))?;
    Ok(child_val)
}

fn el_add_event_listener(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, true)?;
    let event_type = args.first().map(to_string).unwrap_or_default();
    let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
    if event_type.is_empty() {
        return Ok(Value::Undefined);
    }
    if !matches!(listener, Value::Function(_) | Value::NativeFunction(_)) {
        return Err(RuntimeError::new(
            "TypeError",
            "addEventListener listener must be a function",
        ));
    }
    vm.add_dom_listener(node_id, event_type, listener)?;
    Ok(Value::Undefined)
}

fn el_remove_event_listener(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, true)?;
    let event_type = args.first().map(to_string).unwrap_or_default();
    let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
    vm.remove_dom_listener(node_id, &event_type, &listener);
    Ok(Value::Undefined)
}

fn el_focus(vm: &mut JsRuntime, this: &Value, _args: &[Value]) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, false)?;
    vm.with_active_host(|host| host.focus(node_id))?;
    Ok(Value::Undefined)
}

fn el_blur(vm: &mut JsRuntime, this: &Value, _args: &[Value]) -> Result<Value, RuntimeError> {
    let node_id = receiver_node_id(this, false)?;
    vm.with_active_host(|host| host.blur(node_id))?;
    Ok(Value::Undefined)
}

fn class_list_node_id(this: &Value) -> Result<DomNodeId, RuntimeError> {
    match receiver_host_binding(this) {
        Some(HostBinding::ClassList(id)) => Ok(id),
        _ => Err(RuntimeError::new(
            "TypeError",
            "classList method called on invalid receiver",
        )),
    }
}

fn class_list_add(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = class_list_node_id(this)?;
    for arg in args {
        let cls = to_string(arg);
        if !cls.is_empty() {
            vm.with_active_host(|host| host.class_list_add(node_id, &cls))?;
        }
    }
    Ok(Value::Undefined)
}

fn class_list_remove(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = class_list_node_id(this)?;
    for arg in args {
        let cls = to_string(arg);
        if !cls.is_empty() {
            vm.with_active_host(|host| host.class_list_remove(node_id, &cls))?;
        }
    }
    Ok(Value::Undefined)
}

fn class_list_toggle(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = class_list_node_id(this)?;
    let cls = args.first().map(to_string).unwrap_or_default();
    if cls.is_empty() {
        return Ok(Value::Bool(false));
    }
    let present = vm.with_active_host(|host| host.class_list_toggle(node_id, &cls))?;
    Ok(Value::Bool(present))
}

fn class_list_contains(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = class_list_node_id(this)?;
    let cls = args.first().map(to_string).unwrap_or_default();
    if cls.is_empty() {
        return Ok(Value::Bool(false));
    }
    let present = vm.with_active_host(|host| host.class_list_contains(node_id, &cls))?;
    Ok(Value::Bool(present))
}

fn style_node_id(this: &Value) -> Result<DomNodeId, RuntimeError> {
    match receiver_host_binding(this) {
        Some(HostBinding::Style(id)) => Ok(id),
        _ => Err(RuntimeError::new(
            "TypeError",
            "style method called on invalid receiver",
        )),
    }
}

fn style_get_prop(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = style_node_id(this)?;
    let prop = args.first().map(to_string).unwrap_or_default();
    let val = vm.with_active_host(|host| host.get_style_property(node_id, &prop))?;
    Ok(Value::String(Rc::from(val)))
}

fn style_set_prop(
    vm: &mut JsRuntime,
    this: &Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let node_id = style_node_id(this)?;
    let prop = args.first().map(to_string).unwrap_or_default();
    let val = args.get(1).map(to_string).unwrap_or_default();
    if !prop.is_empty() {
        vm.with_active_host(|host| host.set_style_property(node_id, &prop, &val))?;
    }
    Ok(Value::Undefined)
}

fn event_id_from_this(this: &Value) -> Result<u64, RuntimeError> {
    match receiver_host_binding(this) {
        Some(HostBinding::Event(id)) => Ok(id),
        _ => Err(RuntimeError::new(
            "TypeError",
            "Event method called on non-Event receiver",
        )),
    }
}

fn event_prevent_default(
    vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    let event_id = event_id_from_this(this)?;
    if let Some(state) = vm.event_state_mut(event_id) {
        state.default_prevented = true;
    }
    Ok(Value::Undefined)
}

fn event_stop_propagation(
    vm: &mut JsRuntime,
    this: &Value,
    _args: &[Value],
) -> Result<Value, RuntimeError> {
    let event_id = event_id_from_this(this)?;
    if let Some(state) = vm.event_state_mut(event_id) {
        state.propagation_stopped = true;
    }
    Ok(Value::Undefined)
}

pub(crate) fn same_listener(a: &Value, b: &Value) -> bool {
    strict_equal(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::JsError;
    use std::collections::HashMap;

    #[derive(Clone)]
    struct MockNode {
        tag: Option<String>,
        text: String,
        inner_html: String,
        parent: Option<DomNodeId>,
        children: Vec<DomNodeId>,
        attrs: HashMap<String, String>,
        styles: HashMap<String, String>,
        value: Option<String>,
        checked: Option<bool>,
    }

    struct MockDomHost {
        nodes: Vec<MockNode>,
        body_id: DomNodeId,
        title: String,
        focused: Option<DomNodeId>,
        logs: Vec<String>,
    }

    impl MockDomHost {
        fn new() -> Self {
            let doc = MockNode {
                tag: None,
                text: String::new(),
                inner_html: String::new(),
                parent: None,
                children: vec![1],
                attrs: HashMap::new(),
                styles: HashMap::new(),
                value: None,
                checked: None,
            };
            let body = MockNode {
                tag: Some("body".to_string()),
                text: String::new(),
                inner_html: String::new(),
                parent: Some(0),
                children: Vec::new(),
                attrs: HashMap::new(),
                styles: HashMap::new(),
                value: None,
                checked: None,
            };
            Self {
                nodes: vec![doc, body],
                body_id: 1,
                title: "Initial".to_string(),
                focused: None,
                logs: Vec::new(),
            }
        }

        fn add_element(&mut self, parent: DomNodeId, tag: &str, id: Option<&str>) -> DomNodeId {
            let nid = self.create_element(tag);
            if let Some(id_val) = id {
                self.set_attribute(nid, "id", id_val);
            }
            self.append_child(parent, nid);
            nid
        }

        fn matches_simple(&self, node_id: DomNodeId, selector: &str) -> bool {
            let Some(node) = self.nodes.get(node_id) else {
                return false;
            };
            let Some(tag) = &node.tag else {
                return false;
            };
            let sel = selector.trim();
            if let Some(id) = sel.strip_prefix('#') {
                node.attrs.get("id").map(String::as_str) == Some(id)
            } else if let Some(cls) = sel.strip_prefix('.') {
                self.class_list_contains(node_id, cls)
            } else {
                tag.eq_ignore_ascii_case(sel)
            }
        }
    }

    impl DomHost for MockDomHost {
        fn get_element_by_id(&mut self, id: &str) -> Option<DomNodeId> {
            self.nodes
                .iter()
                .enumerate()
                .find(|(_, n)| n.attrs.get("id").map(String::as_str) == Some(id))
                .map(|(idx, _)| idx)
        }

        fn query_selector(&mut self, selector: &str) -> Option<DomNodeId> {
            (0..self.nodes.len()).find(|&idx| self.matches_simple(idx, selector))
        }

        fn query_selector_all(&mut self, selector: &str) -> Vec<DomNodeId> {
            (0..self.nodes.len())
                .filter(|&idx| self.matches_simple(idx, selector))
                .collect()
        }

        fn parent_node(&self, node: DomNodeId) -> Option<DomNodeId> {
            self.nodes.get(node).and_then(|n| n.parent)
        }

        fn document_body(&self) -> Option<DomNodeId> {
            Some(self.body_id)
        }

        fn document_title(&self) -> String {
            self.title.clone()
        }

        fn set_document_title(&mut self, title: &str) {
            self.title = title.to_string();
        }

        fn create_element(&mut self, tag: &str) -> DomNodeId {
            let id = self.nodes.len();
            self.nodes.push(MockNode {
                tag: Some(tag.to_ascii_lowercase()),
                text: String::new(),
                inner_html: String::new(),
                parent: None,
                children: Vec::new(),
                attrs: HashMap::new(),
                styles: HashMap::new(),
                value: None,
                checked: None,
            });
            id
        }

        fn create_text_node(&mut self, text: &str) -> DomNodeId {
            let id = self.nodes.len();
            self.nodes.push(MockNode {
                tag: None,
                text: text.to_string(),
                inner_html: String::new(),
                parent: None,
                children: Vec::new(),
                attrs: HashMap::new(),
                styles: HashMap::new(),
                value: None,
                checked: None,
            });
            id
        }

        fn append_child(&mut self, parent: DomNodeId, child: DomNodeId) -> bool {
            if parent >= self.nodes.len() || child >= self.nodes.len() || parent == child {
                return false;
            }
            if let Some(old_parent) = self.nodes[child].parent {
                self.nodes[old_parent].children.retain(|&c| c != child);
            }
            self.nodes[child].parent = Some(parent);
            self.nodes[parent].children.push(child);
            true
        }

        fn remove_child(&mut self, parent: DomNodeId, child: DomNodeId) -> bool {
            if parent >= self.nodes.len() || child >= self.nodes.len() {
                return false;
            }
            let before = self.nodes[parent].children.len();
            self.nodes[parent].children.retain(|&c| c != child);
            if self.nodes[parent].children.len() < before {
                self.nodes[child].parent = None;
                true
            } else {
                false
            }
        }

        fn tag_name(&self, node: DomNodeId) -> Option<String> {
            self.nodes.get(node).and_then(|n| n.tag.clone())
        }

        fn text_content(&self, node: DomNodeId) -> String {
            let Some(n) = self.nodes.get(node) else {
                return String::new();
            };
            if !n.text.is_empty() || n.children.is_empty() {
                return n.text.clone();
            }
            let mut out = String::new();
            for &child in &n.children {
                out.push_str(&self.text_content(child));
            }
            out
        }

        fn set_text_content(&mut self, node: DomNodeId, text: &str) {
            if let Some(n) = self.nodes.get_mut(node) {
                n.text = text.to_string();
                n.children.clear();
            }
        }

        fn inner_html(&self, node: DomNodeId) -> String {
            self.nodes
                .get(node)
                .map(|n| n.inner_html.clone())
                .unwrap_or_default()
        }

        fn set_inner_html(&mut self, node: DomNodeId, html: &str) {
            if let Some(n) = self.nodes.get_mut(node) {
                n.inner_html = html.to_string();
            }
        }

        fn get_attribute(&self, node: DomNodeId, name: &str) -> Option<String> {
            self.nodes
                .get(node)
                .and_then(|n| n.attrs.get(name).cloned())
        }

        fn set_attribute(&mut self, node: DomNodeId, name: &str, value: &str) {
            if let Some(n) = self.nodes.get_mut(node) {
                n.attrs.insert(name.to_string(), value.to_string());
            }
        }

        fn remove_attribute(&mut self, node: DomNodeId, name: &str) {
            if let Some(n) = self.nodes.get_mut(node) {
                n.attrs.remove(name);
            }
        }

        fn has_attribute(&self, node: DomNodeId, name: &str) -> bool {
            self.nodes
                .get(node)
                .is_some_and(|n| n.attrs.contains_key(name))
        }

        fn class_list_contains(&self, node: DomNodeId, class_name: &str) -> bool {
            self.get_attribute(node, "class")
                .is_some_and(|cls| cls.split_whitespace().any(|c| c == class_name))
        }

        fn class_list_add(&mut self, node: DomNodeId, class_name: &str) {
            let mut classes: Vec<String> = self
                .get_attribute(node, "class")
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            if !classes.iter().any(|c| c == class_name) {
                classes.push(class_name.to_string());
                self.set_attribute(node, "class", &classes.join(" "));
            }
        }

        fn class_list_remove(&mut self, node: DomNodeId, class_name: &str) {
            let classes: Vec<String> = self
                .get_attribute(node, "class")
                .unwrap_or_default()
                .split_whitespace()
                .filter(|&c| c != class_name)
                .map(str::to_string)
                .collect();
            self.set_attribute(node, "class", &classes.join(" "));
        }

        fn class_list_toggle(&mut self, node: DomNodeId, class_name: &str) -> bool {
            if self.class_list_contains(node, class_name) {
                self.class_list_remove(node, class_name);
                false
            } else {
                self.class_list_add(node, class_name);
                true
            }
        }

        fn get_style_property(&self, node: DomNodeId, prop: &str) -> String {
            self.nodes
                .get(node)
                .and_then(|n| n.styles.get(prop).cloned())
                .unwrap_or_default()
        }

        fn set_style_property(&mut self, node: DomNodeId, prop: &str, value: &str) {
            if let Some(n) = self.nodes.get_mut(node) {
                if value.is_empty() {
                    n.styles.remove(prop);
                } else {
                    n.styles.insert(prop.to_string(), value.to_string());
                }
            }
        }

        fn form_value(&self, node: DomNodeId) -> Option<String> {
            self.nodes.get(node).and_then(|n| n.value.clone())
        }

        fn set_form_value(&mut self, node: DomNodeId, value: &str) {
            if let Some(n) = self.nodes.get_mut(node) {
                n.value = Some(value.to_string());
            }
        }

        fn form_checked(&self, node: DomNodeId) -> Option<bool> {
            self.nodes.get(node).and_then(|n| n.checked)
        }

        fn set_form_checked(&mut self, node: DomNodeId, checked: bool) {
            if let Some(n) = self.nodes.get_mut(node) {
                n.checked = Some(checked);
            }
        }

        fn focus(&mut self, node: DomNodeId) {
            self.focused = Some(node);
        }

        fn blur(&mut self, node: DomNodeId) {
            if self.focused == Some(node) {
                self.focused = None;
            }
        }

        fn console_log(&mut self, line: &str) {
            self.logs.push(line.to_string());
        }
    }

    #[test]
    fn dom_queries_title_identity_and_cyrillic_text_content() {
        let mut host = MockDomHost::new();
        let box_id = host.add_element(host.body_id, "div", Some("box"));
        host.set_text_content(box_id, "Привет");

        let mut rt = JsRuntime::new();
        rt.eval_with_host(
            r##"
            let a = document.getElementById("box");
            let b = window.document.querySelector("#box");
            console.log(a === b, a.id, a.tagName, document.title);
            a.textContent += ", мир!";
            a.innerHTML = "<span>Привет</span>";
            document.title = "Заголовок";
            let items = document.querySelectorAll("div");
            console.log(items.length, items[0] === a, document.body.tagName);
            "##,
            &mut host,
        )
        .expect("dom script should succeed");

        assert_eq!(host.text_content(box_id), "Привет, мир!");
        assert_eq!(host.inner_html(box_id), "<span>Привет</span>");
        assert_eq!(host.document_title(), "Заголовок");
        assert_eq!(
            host.logs,
            vec!["true box DIV Initial", "1 true BODY"]
        );
    }

    #[test]
    fn dom_attributes_class_list_style_and_tree_mutations() {
        let mut host = MockDomHost::new();
        let container = host.add_element(host.body_id, "section", Some("wrap"));

        let mut rt = JsRuntime::new();
        rt.eval_with_host(
            r#"
            let wrap = document.getElementById("wrap");
            let p = document.createElement("p");
            p.id = "msg";
            p.setAttribute("data-role", "status");
            let txt = document.createTextNode("OK");
            p.appendChild(txt);
            wrap.appendChild(p);

            p.classList.add("active");
            let toggledOn = p.classList.toggle("hidden");
            let toggledOff = p.classList.toggle("hidden");
            p.classList.remove("active");
            p.classList.add("ready");

            p.style.backgroundColor = "navy";
            p.style.setProperty("color", "white");

            console.log(
                p.hasAttribute("data-role"),
                p.getAttribute("data-role"),
                p.classList.contains("ready"),
                p.classList.contains("active"),
                toggledOn,
                toggledOff,
                p.style.backgroundColor,
                p.style.getPropertyValue("color"),
                p.parentNode === wrap,
                p.textContent
            );

            p.removeAttribute("data-role");
            console.log(p.hasAttribute("data-role"), p.getAttribute("data-role"));
            wrap.removeChild(p);
            console.log(p.parentNode === null);
            "#,
            &mut host,
        )
        .expect("mutations script should succeed");

        assert_eq!(
            host.logs,
            vec![
                "true status true false true false navy white true OK",
                "false null",
                "true",
            ]
        );
        assert!(host.nodes[container].children.is_empty());
    }

    #[test]
    fn dom_form_properties_focus_and_blur() {
        let mut host = MockDomHost::new();
        let input = host.add_element(host.body_id, "input", Some("name"));
        let check = host.add_element(host.body_id, "input", Some("agree"));
        host.set_form_value(input, "Alice");
        host.set_form_checked(check, false);

        let mut rt = JsRuntime::new();
        rt.eval_with_host(
            r#"
            let nameEl = document.getElementById("name");
            let agreeEl = document.getElementById("agree");
            console.log(nameEl.value, agreeEl.checked);
            nameEl.value = "Боб";
            agreeEl.checked = true;
            nameEl.focus();
            "#,
            &mut host,
        )
        .expect("form script");

        assert_eq!(host.form_value(input).as_deref(), Some("Боб"));
        assert_eq!(host.form_checked(check), Some(true));
        assert_eq!(host.focused, Some(input));

        rt.eval_with_host(r#"document.getElementById("name").blur();"#, &mut host)
            .expect("blur script");
        assert_eq!(host.focused, None);
        assert_eq!(host.logs, vec!["Alice false"]);
    }

    #[test]
    fn dom_events_bubble_stop_propagation_prevent_default_and_remove() {
        let mut host = MockDomHost::new();
        let parent = host.add_element(host.body_id, "div", Some("parent"));
        let child = host.add_element(parent, "button", Some("btn"));

        let mut rt = JsRuntime::new();
        rt.eval_with_host(
            r#"
            let parent = document.getElementById("parent");
            let btn = document.getElementById("btn");
            let stopNow = false;

            function onChild(e) {
                console.log("child", e.type, e.target === btn, e.currentTarget === btn, e.key);
                e.preventDefault();
                if (stopNow) {
                    e.stopPropagation();
                }
            }
            function onParent(e) {
                console.log("parent", e.target === btn, e.currentTarget === parent);
            }
            function onDoc(e) {
                console.log("doc", e.target === btn);
            }

            btn.addEventListener("click", onChild);
            parent.addEventListener("click", onParent);
            document.addEventListener("click", onDoc);
            "#,
            &mut host,
        )
        .expect("register listeners");

        let ev = DomEventInput {
            event_type: "click".to_string(),
            target: child,
            key: "Enter".to_string(),
            value: None,
            checked: None,
        };
        let outcome = rt.dispatch_event_with_host(&ev, &mut host).expect("dispatch 1");
        assert!(outcome.default_prevented);
        assert!(!outcome.propagation_stopped);
        assert_eq!(outcome.invoked_listeners, 3);
        assert_eq!(
            host.logs,
            vec!["child click true true Enter", "parent true true", "doc true"]
        );

        host.logs.clear();
        rt.eval_with_host(
            r#"
            stopNow = true;
            document.getElementById("parent").removeEventListener("click", onParent);
            "#,
            &mut host,
        )
        .expect("update stopNow and remove parent");

        let outcome2 = rt.dispatch_event_with_host(&ev, &mut host).expect("dispatch 2");
        assert!(outcome2.default_prevented);
        assert!(outcome2.propagation_stopped);
        assert_eq!(outcome2.invoked_listeners, 1);
        assert_eq!(host.logs, vec!["child click true true Enter"]);
    }

    #[test]
    fn dom_timers_mutate_dom_and_listener_limit_and_budget_hold() {
        let mut host = MockDomHost::new();
        let el = host.add_element(host.body_id, "div", Some("timer-target"));

        let mut rt = JsRuntime::new();
        rt.eval_with_host(
            r#"
            setTimeout(function() {
                document.getElementById("timer-target").textContent = "fired";
                console.log("timer ran");
            }, 50);
            "#,
            &mut host,
        )
        .expect("schedule timer");

        let ran_early = rt.tick_timers_with_host(10, &mut host).expect("tick 10");
        assert_eq!(ran_early, 0);
        assert_eq!(host.text_content(el), "");

        let ran_due = rt.tick_timers_with_host(50, &mut host).expect("tick 50");
        assert_eq!(ran_due, 1);
        assert_eq!(host.text_content(el), "fired");
        assert_eq!(host.logs, vec!["timer ran"]);

        // Calling DOM without an active host fails cleanly without panic.
        let no_host_err = rt
            .run_script(r#"document.getElementById("timer-target");"#)
            .expect_err("DOM call without host must error");
        assert!(matches!(no_host_err, JsError::Runtime(_)));

        // Listener cap enforces MAX_DOM_LISTENERS.
        let cap_err = rt
            .eval_with_host(
                &format!(
                    r#"
                    let t = document.getElementById("timer-target");
                    for (let i = 0; i <= {}; i++) {{
                        t.addEventListener("custom", function() {{}});
                    }}
                    "#,
                    MAX_DOM_LISTENERS
                ),
                &mut host,
            )
            .expect_err("listener limit must trigger");
        assert!(cap_err.to_string().contains("RangeError"));

        // Event listener budget limit protects dispatch_event_with_host from infinite loops.
        let mut loop_rt = JsRuntime::new();
        loop_rt.set_instruction_limit(200);
        loop_rt
            .eval_with_host(
                r#"
                document.getElementById("timer-target").addEventListener("spin", function() {
                    while (true) {}
                });
                "#,
                &mut host,
            )
            .expect("register infinite listener");
        let spin_err = loop_rt
            .dispatch_event_with_host(
                &DomEventInput {
                    event_type: "spin".to_string(),
                    target: el,
                    key: String::new(),
                    value: None,
                    checked: None,
                },
                &mut host,
            )
            .expect_err("infinite listener must hit instruction budget");
        assert!(spin_err.to_string().contains("instruction limit"));
    }
}
