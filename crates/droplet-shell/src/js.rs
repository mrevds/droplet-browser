use std::collections::{HashMap, HashSet};

use droplet_dom::{Document, NodeId, NodeKind};
use droplet_js::{DomEventInput, DomEventOutcome, DomHost, DomNodeId, JsRuntime};
use droplet_url::Url;

use crate::forms::{self, FormState};
use crate::tab::DirtyFlags;

/// Maximum number of `<script>` elements collected per page.
pub const MAX_SCRIPTS_PER_PAGE: usize = 32;
/// Maximum number of console log / error entries retained per tab.
pub const MAX_JS_LOG_ENTRIES: usize = 256;
const STATUS_SUMMARY_MAX_CHARS: usize = 120;

/// A script unit collected in document order from an HTML page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptUnit {
    Inline {
        code: String,
    },
    External {
        url: String,
        code: String,
    },
    LoadError {
        origin: String,
        message: String,
    },
}

/// Per-tab JavaScript runtime, captured console output, and error log.
///
/// DOM bindings are intentionally omitted in DROP-007; DROP-008 attaches them via
/// `execute_scripts_with_hook` / `ensure_runtime_with_hook`.
#[derive(Debug, Default)]
pub struct TabJsState {
    pub runtime: Option<JsRuntime>,
    pub logs: Vec<String>,
    pub errors: Vec<String>,
}

impl TabJsState {
    /// Clears the JS runtime and captured logs/errors for navigation or hibernation.
    pub fn reset(&mut self) {
        self.runtime = None;
        self.logs.clear();
        self.logs.shrink_to_fit();
        self.errors.clear();
        self.errors.shrink_to_fit();
    }

    /// Returns the active runtime, initializing it and running `hook` if needed.
    pub fn ensure_runtime_with_hook<F>(&mut self, mut hook: F) -> &mut JsRuntime
    where
        F: FnMut(&mut JsRuntime),
    {
        if self.runtime.is_none() {
            let mut runtime = JsRuntime::new();
            hook(&mut runtime);
            self.runtime = Some(runtime);
        }
        self.runtime.as_mut().expect("runtime initialized")
    }

    /// Executes collected `<script>` units in order in a fresh per-page runtime.
    pub fn execute_scripts(&mut self, scripts: &[ScriptUnit]) {
        self.execute_scripts_with_hook(scripts, |_| {});
    }

    /// Executes collected `<script>` units in order after invoking `hook` on the new runtime.
    ///
    /// Syntax, compile, load, and runtime errors are captured without panicking so later
    /// scripts on the same page continue to execute in isolation.
    pub fn execute_scripts_with_hook<F>(&mut self, scripts: &[ScriptUnit], mut hook: F)
    where
        F: FnMut(&mut JsRuntime),
    {
        self.reset();
        if scripts.is_empty() {
            return;
        }

        let mut runtime = JsRuntime::new();
        hook(&mut runtime);

        for script in scripts {
            match script {
                ScriptUnit::LoadError { origin, message } => {
                    self.push_error(format!("Script load error ({origin}): {message}"));
                }
                ScriptUnit::Inline { code } | ScriptUnit::External { code, .. } => {
                    let result = runtime.run_script(code);
                    for line in runtime.take_console_log() {
                        self.push_log(line);
                    }
                    if let Err(err) = result {
                        self.push_error(err.to_string());
                    }
                }
            }
        }

        self.runtime = Some(runtime);
    }

    /// Executes collected `<script>` units against the live tab DOM, updating `dirty`
    /// flags for any mutations made by page scripts.
    pub fn execute_scripts_with_dom(
        &mut self,
        scripts: &[ScriptUnit],
        doc: &mut Document,
        forms: &mut FormState,
        dirty: &mut DirtyFlags,
        title: &mut String,
    ) {
        self.reset();
        if scripts.is_empty() {
            return;
        }

        let mut runtime = JsRuntime::new();
        runtime.install_dom_bindings();

        for script in scripts {
            match script {
                ScriptUnit::LoadError { origin, message } => {
                    self.push_error(format!("Script load error ({origin}): {message}"));
                }
                ScriptUnit::Inline { code } | ScriptUnit::External { code, .. } => {
                    let mut host = ShellDomHost {
                        doc,
                        forms,
                        dirty,
                        logs: &mut self.logs,
                        title,
                    };
                    let result = runtime.eval_with_host(code, &mut host);
                    let _ = runtime.take_console_log();
                    if let Err(err) = result {
                        self.push_error(err.to_string());
                    }
                }
            }
        }

        self.runtime = Some(runtime);
    }

    /// Evaluates a single script snippet against the active runtime & live DOM.
    pub fn eval_with_dom(
        &mut self,
        code: &str,
        doc: &mut Document,
        forms: &mut FormState,
        dirty: &mut DirtyFlags,
        title: &mut String,
    ) -> Result<(), String> {
        let runtime = self.runtime.get_or_insert_with(|| {
            let mut rt = JsRuntime::new();
            rt.install_dom_bindings();
            rt
        });
        let mut host = ShellDomHost {
            doc,
            forms,
            dirty,
            logs: &mut self.logs,
            title,
        };
        let res = runtime.eval_with_host(code, &mut host);
        let _ = runtime.take_console_log();
        if let Err(err) = res {
            let msg = err.to_string();
            self.push_error(msg.clone());
            Err(msg)
        } else {
            Ok(())
        }
    }

    /// Dispatches a synthetic DOM event with bubbling if a runtime is initialized.
    pub fn dispatch_event_with_dom(
        &mut self,
        event: &DomEventInput,
        doc: &mut Document,
        forms: &mut FormState,
        dirty: &mut DirtyFlags,
        title: &mut String,
    ) -> DomEventOutcome {
        let Some(runtime) = self.runtime.as_mut() else {
            return DomEventOutcome::default();
        };
        let mut host = ShellDomHost {
            doc,
            forms,
            dirty,
            logs: &mut self.logs,
            title,
        };
        let res = runtime.dispatch_event_with_host(event, &mut host);
        let _ = runtime.take_console_log();
        match res {
            Ok(outcome) => outcome,
            Err(err) => {
                self.push_error(err.to_string());
                DomEventOutcome::default()
            }
        }
    }

    /// Advances the tab's virtual event-loop clock to `now_ms`, firing due timers with DOM access.
    pub fn tick_with_dom(
        &mut self,
        now_ms: f64,
        doc: &mut Document,
        forms: &mut FormState,
        dirty: &mut DirtyFlags,
        title: &mut String,
    ) -> usize {
        let Some(runtime) = self.runtime.as_mut() else {
            return 0;
        };
        let now_u64 = now_ms.max(0.0).round() as u64;
        let mut host = ShellDomHost {
            doc,
            forms,
            dirty,
            logs: &mut self.logs,
            title,
        };
        let res = runtime.tick_timers_with_host(now_u64, &mut host);
        let _ = runtime.take_console_log();
        match res {
            Ok(count) => count,
            Err(err) => {
                self.push_error(err.to_string());
                0
            }
        }
    }

    /// Advances the tab's virtual event-loop clock by `delta_ms` with DOM access.
    pub fn advance_with_dom(
        &mut self,
        delta_ms: f64,
        doc: &mut Document,
        forms: &mut FormState,
        dirty: &mut DirtyFlags,
        title: &mut String,
    ) -> usize {
        let next_ms = self.clock_ms() + delta_ms.max(0.0);
        self.tick_with_dom(next_ms, doc, forms, dirty, title)
    }

    /// Advances the tab's virtual event-loop clock to `now_ms`, firing due timers.
    pub fn tick(&mut self, now_ms: f64) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        let runs = runtime.tick(now_ms);
        let new_logs = runtime.take_console_log();
        for line in new_logs {
            self.push_log(line);
        }
        for run in runs {
            if let Err(err) = run.result {
                self.push_error(err.to_string());
            }
        }
    }

    /// Advances the tab's virtual event-loop clock by `delta_ms`.
    pub fn advance(&mut self, delta_ms: f64) {
        let next_ms = self.clock_ms() + delta_ms.max(0.0);
        self.tick(next_ms);
    }

    pub fn clock_ms(&self) -> f64 {
        self.runtime.as_ref().map_or(0.0, JsRuntime::clock_ms)
    }

    pub fn pending_timer_count(&self) -> usize {
        self.runtime
            .as_ref()
            .map_or(0, JsRuntime::pending_timer_count)
    }

    /// Returns a compact status-bar summary for the most recent JS error or `console.log`.
    pub fn status_summary(&self) -> Option<String> {
        if let Some(last_err) = self.errors.last() {
            return Some(truncate_chars(
                &format!("JS error: {last_err}"),
                STATUS_SUMMARY_MAX_CHARS,
            ));
        }
        if let Some(last_log) = self.logs.last() {
            return Some(truncate_chars(
                &format!("JS: {last_log}"),
                STATUS_SUMMARY_MAX_CHARS,
            ));
        }
        None
    }

    fn push_log(&mut self, line: String) {
        push_capped(&mut self.logs, line);
    }

    fn push_error(&mut self, err: String) {
        push_capped(&mut self.errors, err);
    }
}

/// Live DOM bridge exposing a single tab's `Document`, `FormState`, `DirtyFlags`,
/// title, and bounded console log buffer to `droplet_js::JsRuntime`.
pub struct ShellDomHost<'a> {
    pub doc: &'a mut Document,
    pub forms: &'a mut FormState,
    pub dirty: &'a mut DirtyFlags,
    pub logs: &'a mut Vec<String>,
    pub title: &'a mut String,
}

impl ShellDomHost<'_> {
    fn sync_caret_after_value_change(&mut self, node: NodeId, new_len: usize) {
        if self.forms.focused == Some(node) {
            if self.forms.caret > new_len {
                self.forms.caret = new_len;
            }
            self.forms.anchor = None;
        }
    }
}

impl DomHost for ShellDomHost<'_> {
    fn get_element_by_id(&mut self, id: &str) -> Option<DomNodeId> {
        self.doc.get_element_by_id(id)
    }

    fn query_selector(&mut self, selector: &str) -> Option<DomNodeId> {
        self.doc.query_selector(selector)
    }

    fn query_selector_all(&mut self, selector: &str) -> Vec<DomNodeId> {
        self.doc.query_selector_all(selector)
    }

    fn parent_node(&self, node: DomNodeId) -> Option<DomNodeId> {
        self.doc.parent_node(node)
    }

    fn document_body(&self) -> Option<DomNodeId> {
        self.doc.document_body()
    }

    fn document_title(&self) -> String {
        self.doc
            .find_title()
            .unwrap_or_else(|| self.title.clone())
    }

    fn set_document_title(&mut self, title: &str) {
        self.doc.set_document_title(title);
        *self.title = title.to_string();
        self.dirty.mark_title();
    }

    fn create_element(&mut self, tag: &str) -> DomNodeId {
        self.doc.create_element(tag, Vec::new())
    }

    fn create_text_node(&mut self, text: &str) -> DomNodeId {
        self.doc.create_text(text)
    }

    fn append_child(&mut self, parent: DomNodeId, child: DomNodeId) -> bool {
        let ok = self.doc.append_child(parent, child);
        if ok {
            self.dirty.mark_style_and_layout();
        }
        ok
    }

    fn remove_child(&mut self, parent: DomNodeId, child: DomNodeId) -> bool {
        let ok = self.doc.remove_child(parent, child);
        if ok {
            if self.forms.focused == Some(child) {
                self.forms.focused = None;
                self.forms.anchor = None;
                self.forms.control_dragging = false;
            }
            self.dirty.mark_style_and_layout();
        }
        ok
    }

    fn tag_name(&self, node: DomNodeId) -> Option<String> {
        self.doc.element(node).map(|e| e.tag_name.clone())
    }

    fn text_content(&self, node: DomNodeId) -> String {
        self.doc.text_content_of(node)
    }

    fn set_text_content(&mut self, node: DomNodeId, text: &str) {
        if self.doc.get(node).is_none() {
            return;
        }
        let is_title_node = self
            .doc
            .element(node)
            .is_some_and(|e| e.tag_name.eq_ignore_ascii_case("title"));
        self.doc.set_text_content_of(node, text);
        if is_title_node {
            *self.title = text.trim().to_string();
            self.dirty.mark_title();
        } else {
            self.sync_caret_after_value_change(node, text.len());
            self.dirty.mark_style_and_layout();
        }
    }

    fn inner_html(&self, node: DomNodeId) -> String {
        self.doc.serialize_inner_html(node)
    }

    fn set_inner_html(&mut self, node: DomNodeId, html: &str) {
        if self.doc.set_inner_html_fragment(node, html) {
            self.dirty.mark_style_and_layout();
        }
    }

    fn get_attribute(&self, node: DomNodeId, name: &str) -> Option<String> {
        self.doc
            .element(node)
            .and_then(|e| e.attr(name).map(ToString::to_string))
    }

    fn set_attribute(&mut self, node: DomNodeId, name: &str, value: &str) {
        if self.doc.element(node).is_some() {
            self.doc.set_attr(node, name, value);
            if name == "value" {
                self.sync_caret_after_value_change(node, value.len());
            }
            self.dirty.mark_style_and_layout();
        }
    }

    fn remove_attribute(&mut self, node: DomNodeId, name: &str) {
        if self.doc.element(node).is_some() {
            self.doc.remove_attr(node, name);
            if name == "value" {
                self.sync_caret_after_value_change(node, 0);
            }
            self.dirty.mark_style_and_layout();
        }
    }

    fn has_attribute(&self, node: DomNodeId, name: &str) -> bool {
        self.doc
            .element(node)
            .is_some_and(|e| e.attr(name).is_some())
    }

    fn class_list_contains(&self, node: DomNodeId, class_name: &str) -> bool {
        self.doc.class_list_contains(node, class_name)
    }

    fn class_list_add(&mut self, node: DomNodeId, class_name: &str) {
        let before = self.doc.class_list_contains(node, class_name);
        self.doc.class_list_add(node, class_name);
        if !before && self.doc.class_list_contains(node, class_name) {
            self.dirty.mark_style_and_layout();
        }
    }

    fn class_list_remove(&mut self, node: DomNodeId, class_name: &str) {
        if self.doc.class_list_contains(node, class_name) {
            self.doc.class_list_remove(node, class_name);
            self.dirty.mark_style_and_layout();
        }
    }

    fn class_list_toggle(&mut self, node: DomNodeId, class_name: &str) -> bool {
        let res = self.doc.class_list_toggle(node, class_name);
        if self.doc.element(node).is_some() {
            self.dirty.mark_style_and_layout();
        }
        res
    }

    fn get_style_property(&self, node: DomNodeId, prop: &str) -> String {
        self.doc.style_prop_get(node, prop)
    }

    fn set_style_property(&mut self, node: DomNodeId, prop: &str, value: &str) {
        if self.doc.element(node).is_some() {
            self.doc.style_prop_set(node, prop, value);
            self.dirty.mark_style_and_layout();
        }
    }

    fn form_value(&self, node: DomNodeId) -> Option<String> {
        forms::control_kind(self.doc, node)?;
        Some(forms::value_of(self.doc, node))
    }

    fn set_form_value(&mut self, node: DomNodeId, value: &str) {
        if forms::control_kind(self.doc, node).is_some() {
            forms::set_value(self.doc, node, value);
            self.sync_caret_after_value_change(node, value.len());
            self.dirty.mark_style_and_layout();
        }
    }

    fn form_checked(&self, node: DomNodeId) -> Option<bool> {
        let kind = forms::control_kind(self.doc, node)?;
        if matches!(
            kind,
            droplet_layout::ControlKind::Checkbox | droplet_layout::ControlKind::Radio
        ) {
            Some(self.doc.element(node).is_some_and(|e| e.attr("checked").is_some()))
        } else {
            None
        }
    }

    fn set_form_checked(&mut self, node: DomNodeId, checked: bool) {
        match forms::control_kind(self.doc, node) {
            Some(droplet_layout::ControlKind::Checkbox) => {
                if checked {
                    self.doc.set_attr(node, "checked", "");
                } else {
                    self.doc.remove_attr(node, "checked");
                }
                self.dirty.mark_style_and_layout();
            }
            Some(droplet_layout::ControlKind::Radio) => {
                if checked {
                    forms::check_radio(self.doc, node);
                } else {
                    self.doc.remove_attr(node, "checked");
                }
                self.dirty.mark_style_and_layout();
            }
            _ => {}
        }
    }

    fn focus(&mut self, node: DomNodeId) {
        if !forms::is_focusable(self.doc, node) {
            return;
        }
        let changed = self.forms.focused != Some(node);
        let caret = if forms::is_text_like(self.doc, node) {
            forms::value_of(self.doc, node).len()
        } else {
            0
        };
        self.forms.focused = Some(node);
        self.forms.caret = caret;
        self.forms.anchor = None;
        self.forms.control_dragging = false;
        self.forms.page_drag_anchor = None;
        if changed {
            self.dirty.mark_style_and_layout();
        } else {
            self.dirty.mark_paint();
        }
    }

    fn blur(&mut self, node: DomNodeId) {
        if self.forms.focused == Some(node) {
            self.forms.focused = None;
            self.forms.anchor = None;
            self.forms.control_dragging = false;
            self.dirty.mark_style_and_layout();
        }
    }

    fn console_log(&mut self, line: &str) {
        push_capped(self.logs, line.to_string());
    }
}

enum CollectedScript {
    Inline(String),
    DataUri(String),
    External { raw: String, resolved: String },
    InvalidUrl { raw: String, message: String },
}

/// Collects inline `<script>` and external `<script src>` entries in document order,
/// fetching external URLs concurrently when `fetch_subresources` is true.
pub fn collect_page_scripts(
    doc: &Document,
    base_url: &Url,
    fetch_subresources: bool,
) -> Vec<ScriptUnit> {
    let mut collected = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut fetch_urls = Vec::new();
    let mut stack = vec![doc.root()];

    while let Some(curr) = stack.pop() {
        let Some(node) = doc.get(curr) else {
            continue;
        };
        if let NodeKind::Element(data) = &node.kind {
            if data.tag_name.eq_ignore_ascii_case("script") {
                if collected.len() >= MAX_SCRIPTS_PER_PAGE {
                    continue;
                }
                if !is_javascript_script_type(data.attr("type")) {
                    continue;
                }
                if let Some(src) = data.attr("src").map(str::trim).filter(|s| !s.is_empty()) {
                    if fetch_subresources {
                        if is_data_uri(src) {
                            collected.push(CollectedScript::DataUri(src.to_string()));
                        } else {
                            match base_url.resolve(src) {
                                Ok(resolved) => {
                                    let resolved_str = resolved.as_str().to_string();
                                    if seen_urls.insert(resolved_str.clone()) {
                                        fetch_urls.push(resolved);
                                    }
                                    collected.push(CollectedScript::External {
                                        raw: src.to_string(),
                                        resolved: resolved_str,
                                    });
                                }
                                Err(err) => {
                                    collected.push(CollectedScript::InvalidUrl {
                                        raw: src.to_string(),
                                        message: err.to_string(),
                                    });
                                }
                            }
                        }
                    }
                } else {
                    let code = doc.text_content(curr);
                    if !code.trim().is_empty() {
                        collected.push(CollectedScript::Inline(code));
                    }
                }
                continue;
            }
        }
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }

    if collected.is_empty() {
        return Vec::new();
    }

    let mut fetched: HashMap<String, Result<String, String>> = HashMap::new();
    if !fetch_urls.is_empty() {
        let responses = droplet_net::fetch_many(&fetch_urls);
        for (url, res) in fetch_urls.into_iter().zip(responses) {
            let outcome = match res {
                Ok(resp) if resp.status < 400 => Ok(resp.body),
                Ok(resp) => Err(format!("HTTP status {}", resp.status)),
                Err(err) => Err(err.to_string()),
            };
            fetched.insert(url.as_str().to_string(), outcome);
        }
    }

    let mut out = Vec::with_capacity(collected.len());
    for entry in collected {
        match entry {
            CollectedScript::Inline(code) => out.push(ScriptUnit::Inline { code }),
            CollectedScript::DataUri(uri) => match droplet_net::fetch_uri_bytes(&uri, base_url) {
                Ok((_, bytes)) => {
                    let code = String::from_utf8(bytes)
                        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
                    if !code.trim().is_empty() {
                        out.push(ScriptUnit::External { url: uri, code });
                    }
                }
                Err(err) => out.push(ScriptUnit::LoadError {
                    origin: "data:".to_string(),
                    message: err.to_string(),
                }),
            },
            CollectedScript::External { raw, resolved } => match fetched.get(&resolved) {
                Some(Ok(code)) => {
                    if !code.trim().is_empty() {
                        out.push(ScriptUnit::External {
                            url: resolved,
                            code: code.clone(),
                        });
                    }
                }
                Some(Err(message)) => out.push(ScriptUnit::LoadError {
                    origin: raw,
                    message: message.clone(),
                }),
                None => out.push(ScriptUnit::LoadError {
                    origin: raw,
                    message: "not fetched".to_string(),
                }),
            },
            CollectedScript::InvalidUrl { raw, message } => {
                out.push(ScriptUnit::LoadError {
                    origin: raw,
                    message,
                });
            }
        }
    }
    out
}

fn is_javascript_script_type(type_attr: Option<&str>) -> bool {
    let Some(raw) = type_attr else {
        return true;
    };
    let mime = raw.split(';').next().unwrap_or("").trim();
    if mime.is_empty() {
        return true;
    }
    mime.eq_ignore_ascii_case("text/javascript")
        || mime.eq_ignore_ascii_case("application/javascript")
        || mime.eq_ignore_ascii_case("text/ecmascript")
        || mime.eq_ignore_ascii_case("application/ecmascript")
        || mime.eq_ignore_ascii_case("application/x-javascript")
}

fn is_data_uri(uri: &str) -> bool {
    uri.as_bytes()
        .get(..5)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"data:"))
}

fn push_capped(vec: &mut Vec<String>, entry: String) {
    if vec.len() >= MAX_JS_LOG_ENTRIES {
        vec.remove(0);
    }
    vec.push(entry);
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    for (count, (idx, _)) in text.char_indices().enumerate() {
        if count >= max_chars {
            let mut out = text[..idx].to_string();
            out.push_str("...");
            return out;
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use droplet_dom::html;

    #[test]
    fn truncate_chars_respects_utf8_cyrillic_boundaries() {
        let s = "Привет, мир!";
        assert_eq!(truncate_chars(s, 6), "Привет...");
        assert_eq!(truncate_chars(s, 50), s);
    }

    #[test]
    fn collect_page_scripts_handles_data_uri_and_type_filtering() {
        let doc = html::parse(
            "<html><head>\
             <script type=\"application/ld+json\">{\"a\":1}</script>\
             <script type=\"text/javascript; charset=utf-8\">var x = 1;</script>\
             <script src=\"data:text/javascript,console.log('данные')\"></script>\
             </head></html>",
        );
        let base = Url::parse("about:blank").unwrap();
        let scripts = collect_page_scripts(&doc, &base, true);
        assert_eq!(scripts.len(), 2);

        let mut js = TabJsState::default();
        js.execute_scripts(&scripts);
        assert!(js.errors.is_empty());
        assert_eq!(js.logs, vec!["данные".to_string()]);
    }
}
