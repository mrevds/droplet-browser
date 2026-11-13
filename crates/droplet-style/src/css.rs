pub use droplet_gfx::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Px,
    Percent,
    Em,
    Rem,
    Vh,
    Vw,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Keyword(String),
    Length(f32, Unit),
    Color(Color),
    Number(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub name: String,
    pub values: Vec<Value>,
}

impl Declaration {
    pub fn first(&self) -> Option<&Value> {
        self.values.first()
    }
}

pub type Specificity = (usize, usize, usize);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    pub tag_name: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub universal: bool,
    pub hover: bool,
    pub active: bool,
    pub focus: bool,
    pub unsupported_pseudo: bool,
    /// Attribute selectors (`[a]`, `[a=v]`, `[a~=v i]`, ...); empty for most selectors.
    pub attrs: Vec<AttrSelector>,
    /// Structural and logical pseudo-classes (`:nth-child`, `:not`, `:is`, ...).
    pub pseudo: Vec<PseudoClass>,
}

impl CompoundSelector {
    fn specificity(&self) -> Specificity {
        let mut spec = (
            usize::from(self.id.is_some()),
            self.classes.len()
                + self.attrs.len()
                + usize::from(self.hover)
                + usize::from(self.active)
                + usize::from(self.focus),
            usize::from(self.tag_name.is_some()),
        );
        for pseudo in &self.pseudo {
            let add = pseudo.specificity();
            spec = (spec.0 + add.0, spec.1 + add.1, spec.2 + add.2);
        }
        spec
    }

    /// Whether this compound or any selector nested in its `:not`/`:is`/... arguments uses
    /// `:hover`, `:active` or `:focus`.
    pub fn uses_interaction(&self) -> bool {
        self.hover
            || self.active
            || self.focus
            || self.pseudo.iter().any(|p| {
                p.arguments().iter().any(|sel| {
                    sel.parts.iter().any(|(_, c)| c.uses_interaction())
                })
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    /// `[a]`
    Exists,
    /// `[a=v]`
    Equals,
    /// `[a~=v]`: whitespace-separated word.
    Includes,
    /// `[a|=v]`: `v` or `v-...`.
    DashMatch,
    /// `[a^=v]`
    Prefix,
    /// `[a$=v]`
    Suffix,
    /// `[a*=v]`
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrSelector {
    /// ASCII-lowercase (HTML attribute names are case-insensitive; the DOM lowercases them).
    pub name: String,
    pub op: AttrOp,
    pub value: String,
    /// `i` flag: ASCII case-insensitive value comparison.
    pub case_insensitive: bool,
}

impl AttrSelector {
    pub fn matches(&self, actual: &str) -> bool {
        if !self.case_insensitive {
            let v = self.value.as_str();
            // Fast paths: std's substring search beats the byte-window loop below.
            match self.op {
                AttrOp::Prefix => return !v.is_empty() && actual.starts_with(v),
                AttrOp::Suffix => return !v.is_empty() && actual.ends_with(v),
                AttrOp::Substring => return !v.is_empty() && actual.contains(v),
                _ => {}
            }
        }
        let ci = self.case_insensitive;
        let eq = |a: &[u8], b: &[u8]| if ci { a.eq_ignore_ascii_case(b) } else { a == b };
        let (a, v) = (actual.as_bytes(), self.value.as_bytes());
        // Byte-wise comparison of valid UTF-8 can only match on character boundaries, and
        // ASCII case folding never changes byte lengths, so slicing bytes here is safe.
        match self.op {
            AttrOp::Exists => true,
            AttrOp::Equals => eq(a, v),
            AttrOp::Includes => {
                !v.is_empty()
                    && !self.value.contains(char::is_whitespace)
                    && actual.split_whitespace().any(|w| eq(w.as_bytes(), v))
            }
            AttrOp::DashMatch => {
                eq(a, v) || (a.len() > v.len() && a[v.len()] == b'-' && eq(&a[..v.len()], v))
            }
            AttrOp::Prefix => !v.is_empty() && a.len() >= v.len() && eq(&a[..v.len()], v),
            AttrOp::Suffix => {
                !v.is_empty() && a.len() >= v.len() && eq(&a[a.len() - v.len()..], v)
            }
            AttrOp::Substring => !v.is_empty() && a.windows(v.len()).any(|w| eq(w, v)),
        }
    }
}

/// `An+B` from `:nth-*()`; `odd` is `2n+1`, `even` is `2n`, `:first-child` is `0n+1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nth {
    pub a: i32,
    pub b: i32,
}

impl Nth {
    pub const FIRST: Nth = Nth { a: 0, b: 1 };

    /// Whether the 1-based `index` equals `a*n + b` for some integer `n >= 0`.
    pub fn matches(self, index: usize) -> bool {
        let (a, b, i) = (i64::from(self.a), i64::from(self.b), index as i64);
        if a == 0 {
            return i == b;
        }
        let diff = i - b;
        diff % a == 0 && diff / a >= 0
    }

    fn parse(raw: &str) -> Option<Nth> {
        let s: String = raw
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        match s.as_str() {
            "odd" => return Some(Nth { a: 2, b: 1 }),
            "even" => return Some(Nth { a: 2, b: 0 }),
            _ => {}
        }
        let parse_int = |t: &str| -> Option<i32> {
            let digits = t.strip_prefix('+').unwrap_or(t);
            let body = digits.strip_prefix('-').unwrap_or(digits);
            if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse().ok()
        };
        let Some(n_pos) = s.find('n') else {
            return Some(Nth { a: 0, b: parse_int(&s)? });
        };
        let a = match &s[..n_pos] {
            "" | "+" => 1,
            "-" => -1,
            other => parse_int(other)?,
        };
        let rest = &s[n_pos + 1..];
        let b = if rest.is_empty() {
            0
        } else if rest.starts_with('+') || rest.starts_with('-') {
            parse_int(rest)?
        } else {
            return None;
        };
        Some(Nth { a, b })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PseudoClass {
    /// The document element (`<html>`).
    Root,
    /// No element children and no non-empty text.
    Empty,
    /// `:nth-child` / `:nth-last-child` (`last`), optionally `of S`; also `:first-child`,
    /// `:last-child`.
    NthChild {
        nth: Nth,
        last: bool,
        of: Vec<ComplexSelector>,
    },
    /// `:nth-of-type` / `:nth-last-of-type`; also `:first-of-type`, `:last-of-type`.
    NthOfType { nth: Nth, last: bool },
    OnlyChild,
    OnlyOfType,
    Not(Vec<ComplexSelector>),
    Is(Vec<ComplexSelector>),
    /// Like `:is` but contributes zero specificity.
    Where(Vec<ComplexSelector>),
}

impl PseudoClass {
    fn arguments(&self) -> &[ComplexSelector] {
        match self {
            PseudoClass::NthChild { of, .. } => of,
            PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Where(list) => list,
            _ => &[],
        }
    }

    /// Selectors Level 4: `:is`/`:not` take their most specific argument, `:where` is zero,
    /// `:nth-child(.. of S)` is one pseudo-class plus the most specific `S`.
    fn specificity(&self) -> Specificity {
        let max_of = |list: &[ComplexSelector]| {
            list.iter()
                .map(ComplexSelector::specificity)
                .max()
                .unwrap_or((0, 0, 0))
        };
        match self {
            PseudoClass::Where(_) => (0, 0, 0),
            PseudoClass::Not(list) | PseudoClass::Is(list) => max_of(list),
            PseudoClass::NthChild { of, .. } => {
                let s = max_of(of);
                (s.0, s.1 + 1, s.2)
            }
            _ => (0, 1, 0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    /// `a + b`
    NextSibling,
    /// `a ~ b`
    SubsequentSibling,
}

impl Combinator {
    /// The compound on the left is an ancestor of the one on the right.
    pub fn is_ancestral(self) -> bool {
        matches!(self, Combinator::Descendant | Combinator::Child)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexSelector {
    pub parts: Vec<(Option<Combinator>, CompoundSelector)>,
}

impl ComplexSelector {
    pub fn specificity(&self) -> Specificity {
        let mut spec = (0, 0, 0);
        for (_, compound) in &self.parts {
            let add = compound.specificity();
            spec = (spec.0 + add.0, spec.1 + add.1, spec.2 + add.2);
        }
        spec
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<ComplexSelector>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

pub fn parse(source: &str) -> Stylesheet {
    let cleaned = strip_comments(source);
    let mut parser = Parser::new(&cleaned);
    parser.parse_stylesheet()
}

pub fn parse_inline_style(source: &str) -> Vec<Declaration> {
    let cleaned = strip_comments(source);
    let mut parser = Parser::new(&cleaned);
    parser.parse_declarations(false)
}

fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.char_indices().peekable();
    let mut in_quote: Option<char> = None;
    let mut escaped = false;

    while let Some((_, ch)) = chars.next() {
        if let Some(q) = in_quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                in_quote = None;
            }
            continue;
        }

        if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
            out.push(ch);
            continue;
        }

        if ch == '/' && chars.peek().is_some_and(|&(_, next)| next == '*') {
            chars.next();
            let mut prev_star = false;
            for (_, c) in chars.by_ref() {
                if prev_star && c == '/' {
                    break;
                }
                prev_star = c == '*';
            }
            out.push(' ');
            continue;
        }

        out.push(ch);
    }

    out
}

fn is_ident_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || ch == '-' || (!ch.is_ascii() && !ch.is_whitespace())
}

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-' || (!ch.is_ascii() && !ch.is_whitespace())
}

/// `:not(:is(:not(...)))` nesting limit: keeps parsing and matching recursion bounded.
const MAX_SELECTOR_NESTING: u8 = 8;

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    /// Selector-argument nesting depth (`:not(...)` inside `:is(...)` ...).
    depth: u8,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            depth: 0,
        }
    }

    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn peek_char(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn next_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn consume_while<F>(&mut self, mut pred: F) -> &'a str
    where
        F: FnMut(char) -> bool,
    {
        let start = self.pos;
        let rest = self.rest();
        let mut end = rest.len();
        for (idx, ch) in rest.char_indices() {
            if !pred(ch) {
                end = idx;
                break;
            }
        }
        self.pos += end;
        &self.input[start..self.pos]
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        self.consume_while(char::is_whitespace);
        self.pos > start
    }

    fn parse_stylesheet(&mut self) -> Stylesheet {
        let mut rules = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek_char() {
                None => break,
                Some('@') => {
                    self.skip_at_rule();
                }
                Some('}') => {
                    self.next_char();
                }
                Some(_) => {
                    if let Some(rule) = self.parse_rule() {
                        rules.push(rule);
                    }
                }
            }
        }
        Stylesheet { rules }
    }

    fn skip_at_rule(&mut self) {
        self.next_char(); // '@'
        let mut in_quote: Option<char> = None;
        let mut escaped = false;
        let mut paren_depth: usize = 0;
        let mut brace_depth: usize = 0;

        while let Some(ch) = self.next_char() {
            if let Some(q) = in_quote {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    in_quote = None;
                }
                continue;
            }
            match ch {
                '"' | '\'' => in_quote = Some(ch),
                '(' => paren_depth += 1,
                ')' => paren_depth = paren_depth.saturating_sub(1),
                '{' => brace_depth += 1,
                '}' => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                ';' if brace_depth == 0 && paren_depth == 0 => break,
                _ => {}
            }
        }
    }

    fn parse_rule(&mut self) -> Option<Rule> {
        let selectors = self.parse_selector_list();
        self.skip_whitespace();
        if self.peek_char() != Some('{') {
            // Malformed rule without '{'; skip to next '}' or EOF so parser recovers cleanly.
            while let Some(ch) = self.next_char() {
                if ch == '}' {
                    break;
                }
            }
            return None;
        }
        self.next_char(); // '{'
        let declarations = self.parse_declarations(true);
        if selectors.is_empty() {
            return None;
        }
        Some(Rule {
            selectors,
            declarations,
        })
    }

    fn parse_selector_list(&mut self) -> Vec<ComplexSelector> {
        let mut list = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek_char() {
                None | Some('{') => break,
                Some(',') => {
                    self.next_char();
                    continue;
                }
                _ => {}
            }

            if let Some(sel) = self.parse_complex_selector() {
                list.push(sel);
            } else {
                // Skip invalid/unsupported selector up to ',' or '{'
                self.skip_to_selector_delimiter();
            }

            self.skip_whitespace();
            if self.peek_char() == Some(',') {
                self.next_char();
            } else {
                break;
            }
        }
        list
    }

    fn skip_to_selector_delimiter(&mut self) {
        let mut in_quote: Option<char> = None;
        let mut escaped = false;
        let mut bracket_depth: usize = 0;
        let mut paren_depth: usize = 0;

        while let Some(ch) = self.peek_char() {
            if let Some(q) = in_quote {
                self.next_char();
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    in_quote = None;
                }
                continue;
            }
            match ch {
                '"' | '\'' => {
                    self.next_char();
                    in_quote = Some(ch);
                }
                '[' => {
                    self.next_char();
                    bracket_depth += 1;
                }
                ']' => {
                    self.next_char();
                    bracket_depth = bracket_depth.saturating_sub(1);
                }
                '(' => {
                    self.next_char();
                    paren_depth += 1;
                }
                ')' => {
                    self.next_char();
                    paren_depth = paren_depth.saturating_sub(1);
                }
                ',' | '{' if bracket_depth == 0 && paren_depth == 0 => break,
                _ => {
                    self.next_char();
                }
            }
        }
    }

    fn parse_complex_selector(&mut self) -> Option<ComplexSelector> {
        let mut parts = Vec::new();
        self.skip_whitespace();
        let first = self.parse_compound_selector()?;
        parts.push((None, first));

        loop {
            let had_ws = self.skip_whitespace();
            let combinator = match self.peek_char() {
                None | Some(',') | Some('{') => break,
                Some('>') => Combinator::Child,
                Some('+') => Combinator::NextSibling,
                Some('~') => Combinator::SubsequentSibling,
                Some(_) if had_ws => Combinator::Descendant,
                Some(_) => return None,
            };
            if combinator != Combinator::Descendant {
                self.next_char();
                self.skip_whitespace();
            }
            let next = self.parse_compound_selector()?;
            parts.push((Some(combinator), next));
        }

        Some(ComplexSelector { parts })
    }

    fn parse_compound_selector(&mut self) -> Option<CompoundSelector> {
        let mut sel = CompoundSelector::default();
        let mut parsed_any = false;

        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || matches!(ch, ',' | '{' | '>' | '+' | '~') {
                break;
            }
            match ch {
                '*' => {
                    self.next_char();
                    sel.universal = true;
                    parsed_any = true;
                }
                '#' => {
                    self.next_char();
                    let ident = self.consume_ident();
                    if ident.is_empty() {
                        return None;
                    }
                    sel.id = Some(ident.to_string());
                    parsed_any = true;
                }
                '.' => {
                    self.next_char();
                    let ident = self.consume_ident();
                    if ident.is_empty() {
                        return None;
                    }
                    sel.classes.push(ident.to_string());
                    parsed_any = true;
                }
                '[' => {
                    self.next_char();
                    sel.attrs.push(self.parse_attribute_selector()?);
                    parsed_any = true;
                }
                c if is_ident_start(c) => {
                    let ident = self.consume_ident();
                    if ident.is_empty() {
                        return None;
                    }
                    sel.tag_name = Some(ident.to_ascii_lowercase());
                    parsed_any = true;
                }
                ':' => {
                    self.next_char();
                    let is_pseudo_element = if self.peek_char() == Some(':') {
                        self.next_char();
                        true
                    } else {
                        false
                    };
                    let ident = self.consume_ident();
                    if ident.is_empty() {
                        return None;
                    }
                    let args = if self.peek_char() == Some('(') {
                        let start = self.pos;
                        self.skip_paren_group();
                        if !self.input[start..self.pos].ends_with(')') {
                            return None;
                        }
                        Some(&self.input[start + 1..self.pos - 1])
                    } else {
                        None
                    };
                    let name = ident.to_ascii_lowercase();
                    if is_pseudo_element {
                        sel.unsupported_pseudo = true;
                    } else if let Some(args) = args {
                        match self.parse_functional_pseudo(&name, args)? {
                            Some(p) => sel.pseudo.push(p),
                            None => sel.unsupported_pseudo = true,
                        }
                    } else {
                        let first = Nth::FIRST;
                        let nth_child = |last| PseudoClass::NthChild {
                            nth: first,
                            last,
                            of: Vec::new(),
                        };
                        match name.as_str() {
                            "hover" => sel.hover = true,
                            "active" => sel.active = true,
                            "focus" => sel.focus = true,
                            "root" => sel.pseudo.push(PseudoClass::Root),
                            "empty" => sel.pseudo.push(PseudoClass::Empty),
                            "first-child" => sel.pseudo.push(nth_child(false)),
                            "last-child" => sel.pseudo.push(nth_child(true)),
                            "only-child" => sel.pseudo.push(PseudoClass::OnlyChild),
                            "first-of-type" => sel.pseudo.push(PseudoClass::NthOfType {
                                nth: first,
                                last: false,
                            }),
                            "last-of-type" => sel.pseudo.push(PseudoClass::NthOfType {
                                nth: first,
                                last: true,
                            }),
                            "only-of-type" => sel.pseudo.push(PseudoClass::OnlyOfType),
                            _ => sel.unsupported_pseudo = true,
                        }
                    }
                    parsed_any = true;
                }
                _ => {
                    // Invalid syntax
                    return None;
                }
            }
        }

        if parsed_any {
            Some(sel)
        } else {
            None
        }
    }

    /// After `[`: `name]`, `name op value]`, `name op "value" i]`. `None` = invalid
    /// (namespaced attributes `[ns|a]` are not supported and also give `None`).
    fn parse_attribute_selector(&mut self) -> Option<AttrSelector> {
        self.skip_whitespace();
        let name = self.consume_ident().to_ascii_lowercase();
        if name.is_empty() {
            return None;
        }
        self.skip_whitespace();
        let op = match self.next_char()? {
            ']' => {
                return Some(AttrSelector {
                    name,
                    op: AttrOp::Exists,
                    value: String::new(),
                    case_insensitive: false,
                })
            }
            '=' => AttrOp::Equals,
            c => {
                let op = match c {
                    '~' => AttrOp::Includes,
                    '|' => AttrOp::DashMatch,
                    '^' => AttrOp::Prefix,
                    '$' => AttrOp::Suffix,
                    '*' => AttrOp::Substring,
                    _ => return None,
                };
                if self.next_char()? != '=' {
                    return None;
                }
                op
            }
        };
        self.skip_whitespace();
        let value = match self.peek_char()? {
            '"' | '\'' => consume_quoted_string(self),
            _ => {
                let ident = self.consume_ident();
                if ident.is_empty() {
                    return None;
                }
                ident.to_string()
            }
        };
        self.skip_whitespace();
        let mut case_insensitive = false;
        let flag = self.consume_ident();
        if flag.eq_ignore_ascii_case("i") {
            case_insensitive = true;
        } else if !flag.eq_ignore_ascii_case("s") && !flag.is_empty() {
            return None;
        }
        self.skip_whitespace();
        if self.next_char()? != ']' {
            return None;
        }
        Some(AttrSelector {
            name,
            op,
            value,
            case_insensitive,
        })
    }

    /// `Err`-like `None`: invalid, drop the selector. `Some(None)`: valid but unsupported
    /// pseudo-class (never matches). `Some(Some(p))`: supported.
    fn parse_functional_pseudo(&self, name: &str, args: &str) -> Option<Option<PseudoClass>> {
        let pseudo = match name {
            "not" => PseudoClass::Not(self.parse_nested_list(args, false)?),
            "is" | "matches" | "-webkit-any" | "-moz-any" => {
                PseudoClass::Is(self.parse_nested_list(args, true)?)
            }
            "where" => PseudoClass::Where(self.parse_nested_list(args, true)?),
            "nth-child" | "nth-last-child" => {
                let (nth_src, of_src) = split_of_clause(args);
                let of = match of_src {
                    Some(list) => self.parse_nested_list(list, false)?,
                    None => Vec::new(),
                };
                PseudoClass::NthChild {
                    nth: Nth::parse(nth_src)?,
                    last: name == "nth-last-child",
                    of,
                }
            }
            "nth-of-type" | "nth-last-of-type" => PseudoClass::NthOfType {
                nth: Nth::parse(args)?,
                last: name == "nth-last-of-type",
            },
            _ => return Some(None),
        };
        Some(Some(pseudo))
    }

    /// Selector list inside `:not(...)` / `:is(...)`. `forgiving` (`:is`, `:where`) drops
    /// invalid entries instead of invalidating the whole list.
    fn parse_nested_list(&self, src: &str, forgiving: bool) -> Option<Vec<ComplexSelector>> {
        if self.depth >= MAX_SELECTOR_NESTING {
            return None;
        }
        let mut sub = Parser {
            input: src,
            pos: 0,
            depth: self.depth + 1,
        };
        let mut list = Vec::new();
        let mut after_comma = false;
        loop {
            sub.skip_whitespace();
            if sub.peek_char().is_none() {
                if after_comma && !forgiving {
                    return None; // `:not(.a,)`
                }
                break;
            }
            let parsed = sub.parse_complex_selector();
            sub.skip_whitespace();
            let at_end = matches!(sub.peek_char(), None | Some(','));
            match parsed {
                Some(sel) if at_end => list.push(sel),
                _ if forgiving => sub.skip_to_selector_delimiter(),
                _ => return None,
            }
            match sub.peek_char() {
                Some(',') => {
                    sub.next_char();
                    after_comma = true;
                }
                None => break,
                // Stuck on a `{` (cannot appear inside a valid argument).
                Some(_) => return None,
            }
        }
        if list.is_empty() && !forgiving {
            return None;
        }
        Some(list)
    }

    fn skip_paren_group(&mut self) {
        if self.next_char() != Some('(') {
            return;
        }
        let mut depth: usize = 1;
        let mut in_quote: Option<char> = None;
        let mut escaped = false;

        while let Some(ch) = self.next_char() {
            if let Some(q) = in_quote {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    in_quote = None;
                }
                continue;
            }
            match ch {
                '"' | '\'' => in_quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    fn consume_ident(&mut self) -> &'a str {
        self.consume_while(is_ident_char)
    }

    fn parse_declarations(&mut self, stop_at_brace: bool) -> Vec<Declaration> {
        let mut decls = Vec::new();
        loop {
            self.skip_whitespace();
            while self.peek_char() == Some(';') {
                self.next_char();
                self.skip_whitespace();
            }
            match self.peek_char() {
                None => break,
                Some('}') if stop_at_brace => {
                    self.next_char();
                    break;
                }
                _ => {}
            }

            let raw_name = self.consume_decl_name(stop_at_brace);
            self.skip_whitespace();
            if self.peek_char() != Some(':') {
                // Skip invalid declaration up to ';' or '}'
                self.skip_to_decl_end(stop_at_brace);
                if self.peek_char() == Some(';') {
                    self.next_char();
                }
                continue;
            }
            self.next_char(); // ':'

            let raw_value = self.consume_decl_value(stop_at_brace);
            if self.peek_char() == Some(';') {
                self.next_char();
            }

            let name = raw_name.trim().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            let values = parse_values(raw_value);
            if !values.is_empty() {
                decls.push(Declaration { name, values });
            }
        }
        decls
    }

    fn consume_decl_name(&mut self, stop_at_brace: bool) -> &'a str {
        let start = self.pos;
        while let Some(ch) = self.peek_char() {
            if ch == ':' || ch == ';' || (stop_at_brace && ch == '}') {
                break;
            }
            self.next_char();
        }
        &self.input[start..self.pos]
    }

    fn skip_to_decl_end(&mut self, stop_at_brace: bool) {
        let mut in_quote: Option<char> = None;
        let mut escaped = false;
        while let Some(ch) = self.peek_char() {
            if let Some(q) = in_quote {
                self.next_char();
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    in_quote = None;
                }
                continue;
            }
            if ch == '"' || ch == '\'' {
                self.next_char();
                in_quote = Some(ch);
                continue;
            }
            if ch == ';' || (stop_at_brace && ch == '}') {
                break;
            }
            self.next_char();
        }
    }

    fn consume_decl_value(&mut self, stop_at_brace: bool) -> &'a str {
        let start = self.pos;
        let mut in_quote: Option<char> = None;
        let mut escaped = false;
        let mut paren_depth: usize = 0;

        while let Some(ch) = self.peek_char() {
            if let Some(q) = in_quote {
                self.next_char();
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    in_quote = None;
                }
                continue;
            }
            match ch {
                '"' | '\'' => {
                    self.next_char();
                    in_quote = Some(ch);
                }
                '(' => {
                    self.next_char();
                    paren_depth += 1;
                }
                ')' => {
                    self.next_char();
                    paren_depth = paren_depth.saturating_sub(1);
                }
                ';' if paren_depth == 0 => break,
                '}' if stop_at_brace && paren_depth == 0 => break,
                _ => {
                    self.next_char();
                }
            }
        }
        &self.input[start..self.pos]
    }
}

pub(crate) fn parse_values(raw: &str) -> Vec<Value> {
    let mut values = Vec::new();
    let mut p = Parser::new(raw);

    loop {
        p.consume_while(|c| c.is_whitespace() || c == ',');
        let Some(ch) = p.peek_char() else {
            break;
        };

        // Ignore !important flag
        if ch == '!' {
            p.next_char();
            p.skip_whitespace();
            let kw = p.consume_ident();
            if kw.eq_ignore_ascii_case("important") {
                continue;
            }
            if !kw.is_empty() {
                values.push(Value::Keyword(format!("!{}", kw.to_ascii_lowercase())));
            }
            continue;
        }

        // Quoted string
        if ch == '"' || ch == '\'' {
            let s = consume_quoted_string(&mut p);
            values.push(Value::Keyword(s));
            continue;
        }

        // Hex color
        if ch == '#' {
            p.next_char();
            let hex_part = p.consume_while(|c| c.is_ascii_alphanumeric());
            let full = format!("#{}", hex_part);
            if let Some(color) = Color::from_name_or_hex(&full) {
                values.push(Value::Color(color));
            } else {
                values.push(Value::Keyword(full));
            }
            continue;
        }

        // Number or Length
        if starts_number(p.rest()) {
            let start = p.pos;
            if let Some(num) = consume_number(&mut p) {
                if p.peek_char() == Some('%') {
                    p.next_char();
                    values.push(Value::Length(num, Unit::Percent));
                    continue;
                }
                if p.peek_char().is_some_and(|c| c.is_alphabetic()) {
                    let unit_str = p.consume_ident();
                    match unit_str.to_ascii_lowercase().as_str() {
                        "px" => values.push(Value::Length(num, Unit::Px)),
                        "em" => values.push(Value::Length(num, Unit::Em)),
                        "rem" => values.push(Value::Length(num, Unit::Rem)),
                        "vh" => values.push(Value::Length(num, Unit::Vh)),
                        "vw" => values.push(Value::Length(num, Unit::Vw)),
                        _ => values.push(Value::Keyword(p.input[start..p.pos].to_ascii_lowercase())),
                    }
                    continue;
                }
                if num == 0.0 {
                    values.push(Value::Length(0.0, Unit::Px));
                } else {
                    values.push(Value::Number(num));
                }
                continue;
            }
        }

        // Identifier, named color, or CSS function call
        if is_ident_start(ch) {
            let ident = p.consume_ident();
            if p.peek_char() == Some('(') {
                let args = consume_paren_group(&mut p);
                let lower = ident.to_ascii_lowercase();
                if lower == "rgb" || lower == "rgba" {
                    let after_open = args.strip_prefix('(').unwrap_or(&args);
                    let inner = after_open.strip_suffix(')').unwrap_or(after_open);
                    if let Some(color) = parse_rgb_function(inner) {
                        values.push(Value::Color(color));
                        continue;
                    }
                }
                values.push(Value::Keyword(format!("{}{}", lower, args)));
                continue;
            }
            let lower = ident.to_ascii_lowercase();
            if let Some(color) = Color::from_name_or_hex(&lower) {
                values.push(Value::Color(color));
            } else {
                values.push(Value::Keyword(lower));
            }
            continue;
        }

        // Fallback single character (e.g. '/')
        let c = p.next_char().unwrap();
        values.push(Value::Keyword(c.to_string()));
    }

    values
}

fn consume_quoted_string(p: &mut Parser<'_>) -> String {
    let Some(quote) = p.next_char() else {
        return String::new();
    };
    let mut out = String::new();
    let mut escaped = false;
    while let Some(ch) = p.next_char() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Splits `2n+1 of .a, .b` into (`2n+1`, Some(`.a, .b`)).
fn split_of_clause(args: &str) -> (&str, Option<&str>) {
    let bytes = args.as_bytes();
    for i in 0..bytes.len().saturating_sub(3) {
        if bytes[i].is_ascii_whitespace()
            && bytes[i + 1..i + 3].eq_ignore_ascii_case(b"of")
            && bytes[i + 3].is_ascii_whitespace()
        {
            return (&args[..i], Some(&args[i + 4..]));
        }
    }
    (args, None)
}

fn consume_paren_group(p: &mut Parser<'_>) -> String {
    let start = p.pos;
    p.skip_paren_group();
    p.input[start..p.pos].to_string()
}

fn starts_number(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('-') | Some('+') => match chars.next() {
            Some(c) if c.is_ascii_digit() => true,
            Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
            _ => false,
        },
        Some('.') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

fn consume_number(p: &mut Parser<'_>) -> Option<f32> {
    let start = p.pos;
    if matches!(p.peek_char(), Some('-') | Some('+')) {
        p.next_char();
    }
    let mut has_digits = false;
    while p.peek_char().is_some_and(|c| c.is_ascii_digit()) {
        p.next_char();
        has_digits = true;
    }
    if p.peek_char() == Some('.') {
        let after_dot = p.rest().chars().nth(1);
        if after_dot.is_some_and(|c| c.is_ascii_digit()) {
            p.next_char(); // '.'
            while p.peek_char().is_some_and(|c| c.is_ascii_digit()) {
                p.next_char();
                has_digits = true;
            }
        }
    }
    if !has_digits {
        p.pos = start;
        return None;
    }
    let slice = &p.input[start..p.pos];
    match slice.parse::<f32>() {
        Ok(v) if v.is_finite() => Some(if v == 0.0 { 0.0 } else { v }),
        _ => {
            p.pos = start;
            None
        }
    }
}

fn parse_rgb_function(inner: &str) -> Option<Color> {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return None;
    }

    let owned_parts: Vec<String> = if trimmed.contains(',') {
        trimmed.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        trimmed
            .split(|c: char| c.is_whitespace() || c == '/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    };

    if owned_parts.len() != 3 && owned_parts.len() != 4 {
        return None;
    }

    let r = parse_rgb_channel(&owned_parts[0])?;
    let g = parse_rgb_channel(&owned_parts[1])?;
    let b = parse_rgb_channel(&owned_parts[2])?;
    let a = if owned_parts.len() == 4 {
        parse_alpha_channel(&owned_parts[3])?
    } else {
        255
    };
    Some(Color::rgba(r, g, b, a))
}

fn parse_rgb_channel(token: &str) -> Option<u8> {
    if let Some(pct) = token.strip_suffix('%') {
        let v: f32 = pct.trim().parse().ok()?;
        return Some(((v / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    let v: f32 = token.parse().ok()?;
    Some(v.round().clamp(0.0, 255.0) as u8)
}

fn parse_alpha_channel(token: &str) -> Option<u8> {
    if let Some(pct) = token.strip_suffix('%') {
        let v: f32 = pct.trim().parse().ok()?;
        return Some(((v / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    let v: f32 = token.parse().ok()?;
    Some((v * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colors_hex_named_and_rgb() {
        assert_eq!(Color::from_name_or_hex("#abc"), Some(Color::rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(Color::from_name_or_hex("#0f172a"), Some(Color::rgb(15, 23, 42)));
        assert_eq!(
            Color::from_name_or_hex("#ff000080"),
            Some(Color::rgba(255, 0, 0, 128))
        );
        assert_eq!(Color::from_name_or_hex("White"), Some(Color::rgb(255, 255, 255)));
        assert_eq!(
            Color::from_name_or_hex("transparent"),
            Some(Color::rgba(0, 0, 0, 0))
        );
        assert_eq!(Color::from_name_or_hex("grey"), Color::from_name_or_hex("gray"));
        assert!(Color::from_name_or_hex("slate").is_some());
        assert_eq!(Color::rgb(0x12, 0x34, 0x56).to_u32_xrgb(), 0x0012_3456);

        let ss = parse("p { color: rgb(10, 20, 30); background-color: rgba(255, 0, 0, 0.5); }");
        assert_eq!(ss.rules.len(), 1);
        assert_eq!(
            ss.rules[0].declarations[0].first(),
            Some(&Value::Color(Color::rgb(10, 20, 30)))
        );
        assert_eq!(
            ss.rules[0].declarations[1].first(),
            Some(&Value::Color(Color::rgba(255, 0, 0, 128)))
        );
    }

    #[test]
    fn test_comments_and_at_rules_skipped() {
        let css = r#"
            @charset "UTF-8";
            @import url("other.css");
            /* Комментарий с кириллицей и /* внутри */
            @media (max-width: 700px) {
                .card { width: 100%; }
                div > p { color: red; }
            }
            @keyframes fade {
                0% { opacity: 0; }
                100% { opacity: 1; }
            }
            .card#main { margin: -4px 1.5em 0 2rem; width: 50%; }
        "#;
        let ss = parse(css);
        assert_eq!(ss.rules.len(), 1);
        let sel = &ss.rules[0].selectors[0];
        assert_eq!(sel.specificity(), (1, 1, 0));
        assert_eq!(
            ss.rules[0].declarations[0].values,
            vec![
                Value::Length(-4.0, Unit::Px),
                Value::Length(1.5, Unit::Em),
                Value::Length(0.0, Unit::Px),
                Value::Length(2.0, Unit::Rem),
            ]
        );
        assert_eq!(
            ss.rules[0].declarations[1].values,
            vec![Value::Length(50.0, Unit::Percent)]
        );
    }

    #[test]
    fn test_selectors_combinators_and_specificity() {
        let ss = parse("div.card#main, ul > li a.link, * { color: navy; }");
        assert_eq!(ss.rules.len(), 1);
        let sels = &ss.rules[0].selectors;
        assert_eq!(sels.len(), 3);
        assert_eq!(sels[0].specificity(), (1, 1, 1));
        assert_eq!(sels[1].specificity(), (0, 1, 3));
        assert_eq!(sels[1].parts.len(), 3);
        assert_eq!(sels[1].parts[0].0, None);
        assert_eq!(sels[1].parts[1].0, Some(Combinator::Child));
        assert_eq!(sels[1].parts[2].0, Some(Combinator::Descendant));
        assert_eq!(sels[2].specificity(), (0, 0, 0));
        assert!(sels[2].parts[0].1.universal);
    }

    #[test]
    fn test_multi_value_and_inline_declarations() {
        let decls = parse_inline_style(
            "padding: 8px 12px 16px; border: 1px solid #cbd5e1; font-weight: 700; color: teal !important",
        );
        assert_eq!(decls.len(), 4);
        assert_eq!(decls[0].name, "padding");
        assert_eq!(decls[0].values.len(), 3);
        assert_eq!(decls[1].name, "border");
        assert_eq!(
            decls[1].values,
            vec![
                Value::Length(1.0, Unit::Px),
                Value::Keyword("solid".into()),
                Value::Color(Color::rgb(0xcb, 0xd5, 0xe1)),
            ]
        );
        assert_eq!(decls[2].values, vec![Value::Number(700.0)]);
        assert_eq!(
            decls[3].values,
            vec![Value::Color(Color::rgb(0, 128, 128))]
        );
    }

    #[test]
    fn test_utf8_and_hover_selector_resilience() {
        let css = r#"
            /* Привет мир 🎨 */
            a, a:hover, a:nth-foo(2), div::before { color: blue; }
            .кнопка { padding: 4px; }
        "#;
        let ss = parse(css);
        assert_eq!(ss.rules.len(), 2);
        assert_eq!(ss.rules[0].selectors.len(), 4);
        assert_eq!(ss.rules[0].selectors[0].parts[0].1.tag_name.as_deref(), Some("a"));
        assert!(!ss.rules[0].selectors[0].parts[0].1.hover);
        assert!(!ss.rules[0].selectors[0].parts[0].1.unsupported_pseudo);
        assert!(ss.rules[0].selectors[1].parts[0].1.hover);
        assert!(!ss.rules[0].selectors[1].parts[0].1.unsupported_pseudo);
        assert!(ss.rules[0].selectors[2].parts[0].1.unsupported_pseudo);
        assert!(ss.rules[0].selectors[3].parts[0].1.unsupported_pseudo);
        assert_eq!(ss.rules[1].selectors[0].parts[0].1.classes, vec!["кнопка"]);
    }

    #[test]
    fn test_pseudo_classes_and_specificity() {
        let ss = parse("a:hover, .btn:active:hover, :Active { color: red; }");
        assert_eq!(ss.rules.len(), 1);
        let sels = &ss.rules[0].selectors;
        assert_eq!(sels.len(), 3);

        assert_eq!(sels[0].specificity(), (0, 1, 1));
        assert!(sels[0].parts[0].1.hover);
        assert!(!sels[0].parts[0].1.active);

        assert_eq!(sels[1].specificity(), (0, 3, 0));
        assert!(sels[1].parts[0].1.hover);
        assert!(sels[1].parts[0].1.active);
        assert_eq!(sels[1].parts[0].1.classes, vec!["btn"]);

        assert_eq!(sels[2].specificity(), (0, 1, 0));
        assert!(sels[2].parts[0].1.active);
        assert!(!sels[2].parts[0].1.hover);
    }

    #[test]
    fn test_focus_pseudo_selector_specificity_and_utf8() {
        let ss = parse("input:focus, button:focus:hover, :Focus { border-color: #2563eb; }");
        assert_eq!(ss.rules.len(), 1);
        let sels = &ss.rules[0].selectors;
        assert_eq!(sels[0].specificity(), (0, 1, 1));
        assert!(sels[0].parts[0].1.focus);
        assert_eq!(sels[1].specificity(), (0, 2, 1));
        assert!(sels[1].parts[0].1.focus);
        assert!(sels[1].parts[0].1.hover);
        assert!(sels[2].parts[0].1.focus);
        assert!(!sels[2].parts[0].1.unsupported_pseudo);
        assert!(!sels[0].parts[0].1.unsupported_pseudo);

        let cyr = parse("textarea:focus { color: #0f172a; } /* Кириллица */");
        assert!(cyr.rules[0].selectors[0].parts[0].1.focus);
    }

    #[test]
    fn test_relative_units_vh_vw_rem_em_percent_and_flex_keywords() {
        let ss = parse(
            r#"
            .panel {
                display: flex;
                flex-direction: row-reverse;
                flex-wrap: wrap-reverse;
                justify-content: space-evenly;
                align-items: baseline;
                width: 50%;
                min-height: 50vh;
                max-width: 25VW;
                padding: 1.5rem 2em;
                border-radius: 8px;
                font-family: "JetBrains Mono", Consolas, monospace;
            }
            "#,
        );
        assert_eq!(ss.rules.len(), 1);
        let decls = &ss.rules[0].declarations;
        assert_eq!(decls[0].first(), Some(&Value::Keyword("flex".into())));
        assert_eq!(decls[1].first(), Some(&Value::Keyword("row-reverse".into())));
        assert_eq!(decls[2].first(), Some(&Value::Keyword("wrap-reverse".into())));
        assert_eq!(decls[3].first(), Some(&Value::Keyword("space-evenly".into())));
        assert_eq!(decls[4].first(), Some(&Value::Keyword("baseline".into())));
        assert_eq!(decls[5].first(), Some(&Value::Length(50.0, Unit::Percent)));
        assert_eq!(decls[6].first(), Some(&Value::Length(50.0, Unit::Vh)));
        assert_eq!(decls[7].first(), Some(&Value::Length(25.0, Unit::Vw)));
        assert_eq!(
            decls[8].values,
            vec![
                Value::Length(1.5, Unit::Rem),
                Value::Length(2.0, Unit::Em),
            ]
        );
        assert_eq!(decls[9].first(), Some(&Value::Length(8.0, Unit::Px)));
        assert_eq!(
            decls[10].values,
            vec![
                Value::Keyword("JetBrains Mono".into()),
                Value::Keyword("consolas".into()),
                Value::Keyword("monospace".into()),
            ]
        );
    }

    fn only_compound(src: &str) -> CompoundSelector {
        let ss = parse(&format!("{src} {{ color: red }}"));
        assert_eq!(ss.rules.len(), 1, "{src}");
        let sel = &ss.rules[0].selectors;
        assert_eq!(sel.len(), 1, "{src}");
        assert_eq!(sel[0].parts.len(), 1, "{src}");
        sel[0].parts[0].1.clone()
    }

    #[test]
    fn test_attribute_selectors_parse_and_match() {
        let c = only_compound(r#"a[HREF^="https://"][data-тип='кнопка' i][lang|=ru]"#);
        assert_eq!(c.tag_name.as_deref(), Some("a"));
        assert_eq!(c.attrs.len(), 3);
        assert_eq!(c.attrs[0].name, "href");
        assert_eq!(c.attrs[0].op, AttrOp::Prefix);
        assert_eq!(c.attrs[1].name, "data-тип");
        assert_eq!(c.attrs[1].value, "кнопка");
        assert!(c.attrs[1].case_insensitive);
        assert_eq!(c.attrs[2].op, AttrOp::DashMatch);
        assert!(!c.unsupported_pseudo);

        let attr = |src: &str| only_compound(src).attrs[0].clone();
        // (selector, attribute value, expected)
        let table: &[(&str, &str, bool)] = &[
            ("[x]", "", true),
            ("[x=a]", "a", true),
            ("[x=a]", "A", false),
            ("[x=a i]", "A", true),
            ("[x='привет']", "привет", true),
            ("[x='привет' i]", "ПРИВЕТ", false), // `i` is ASCII-only
            ("[x~=b]", "a b  c", true),
            ("[x~=b]", "ab", false),
            ("[x~='']", "", false),
            ("[x~=мир]", "привет мир", true),
            ("[x|=en]", "en", true),
            ("[x|=en]", "en-US", true),
            ("[x|=en]", "eng", false),
            ("[x|=EN i]", "en-gb", true),
            ("[x^=при]", "привет", true),
            ("[x^=вет]", "привет", false),
            ("[x^='']", "a", false),
            ("[x$=вет]", "привет", true),
            ("[x$='.PDF' i]", "file.pdf", true),
            ("[x*=ив]", "привет", true),
            ("[x*=zz]", "привет", false),
            ("[x*='']", "a", false),
        ];
        for &(sel, value, expected) in table {
            assert_eq!(attr(sel).matches(value), expected, "{sel} vs {value:?}");
        }
    }

    #[test]
    fn test_invalid_attribute_selectors_drop_only_that_selector() {
        let ss = parse("[a=], [ns|a], [a=b x], [=b], [a^b], p { color: red } [ok] { color: blue }");
        assert_eq!(ss.rules.len(), 2);
        assert_eq!(ss.rules[0].selectors.len(), 1);
        assert_eq!(ss.rules[0].selectors[0].parts[0].1.tag_name.as_deref(), Some("p"));
        assert_eq!(ss.rules[1].selectors[0].parts[0].1.attrs[0].op, AttrOp::Exists);
    }

    #[test]
    fn test_nth_parsing_and_matching() {
        let n = |a, b| Some(Nth { a, b });
        let table: &[(&str, Option<Nth>)] = &[
            ("odd", n(2, 1)),
            ("EVEN", n(2, 0)),
            ("3", n(0, 3)),
            ("+5", n(0, 5)),
            ("-2", n(0, -2)),
            ("n", n(1, 0)),
            ("-n+3", n(-1, 3)),
            ("+n", n(1, 0)),
            ("2n+1", n(2, 1)),
            (" 2n - 1 ", n(2, -1)),
            ("-3n+10", n(-3, 10)),
            ("10N", n(10, 0)),
            ("2n1", None),
            ("x", None),
            ("", None),
            ("2n+", None),
        ];
        for &(src, expected) in table {
            assert_eq!(Nth::parse(src), expected, "{src:?}");
        }
        let hits = |nth: Nth| (1..=10).filter(|&i| nth.matches(i)).collect::<Vec<_>>();
        assert_eq!(hits(Nth { a: 2, b: 1 }), vec![1, 3, 5, 7, 9]);
        assert_eq!(hits(Nth { a: 2, b: 0 }), vec![2, 4, 6, 8, 10]);
        assert_eq!(hits(Nth { a: -1, b: 3 }), vec![1, 2, 3]);
        assert_eq!(hits(Nth { a: 3, b: -1 }), vec![2, 5, 8]);
        assert_eq!(hits(Nth { a: 0, b: 4 }), vec![4]);
        assert_eq!(hits(Nth { a: 0, b: -1 }), Vec::<usize>::new());
    }

    #[test]
    fn test_structural_and_logical_pseudo_classes_parse() {
        let c = only_compound("li:nth-child(odd):first-child:last-of-type:only-child:empty:root");
        assert!(!c.unsupported_pseudo);
        assert_eq!(c.pseudo.len(), 6);
        assert_eq!(
            c.pseudo[0],
            PseudoClass::NthChild { nth: Nth { a: 2, b: 1 }, last: false, of: Vec::new() }
        );
        let c = only_compound("li:nth-last-child(2n of .a, .b)");
        let PseudoClass::NthChild { nth, last, of } = &c.pseudo[0] else {
            panic!("{:?}", c.pseudo);
        };
        assert_eq!((*nth, *last, of.len()), (Nth { a: 2, b: 0 }, true, 2));

        let c = only_compound("p:not(.skip, div > .x):is(.кнопка, :hover):where(#id)");
        assert_eq!(c.pseudo.len(), 3);
        assert!(c.uses_interaction(), ":hover nested in :is()");
        // Unknown pseudo-class / invalid nth / invalid :not argument.
        assert!(only_compound("p:nth-foo(2)").unsupported_pseudo);
        let ss = parse("p:nth-child(2x), p:not(), p:not([), p:not(.a,), em { color: red }");
        assert_eq!(ss.rules[0].selectors.len(), 1);
        // :is() is forgiving: the bad argument is dropped, the good one kept.
        let c = only_compound(":is(.a, [, .б)");
        let PseudoClass::Is(list) = &c.pseudo[0] else { panic!() };
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_sibling_combinators_parse() {
        let ss = parse("h1 + p, h1+p, .a ~ .б, ul > li ~ li a { color: red }");
        let sels = &ss.rules[0].selectors;
        assert_eq!(sels.len(), 4);
        assert_eq!(sels[0].parts[1].0, Some(Combinator::NextSibling));
        assert_eq!(sels[1], sels[0]);
        assert_eq!(sels[2].parts[1].0, Some(Combinator::SubsequentSibling));
        let combs: Vec<_> = sels[3].parts.iter().map(|(c, _)| *c).collect();
        assert_eq!(
            combs,
            vec![
                None,
                Some(Combinator::Child),
                Some(Combinator::SubsequentSibling),
                Some(Combinator::Descendant)
            ]
        );
    }

    #[test]
    fn test_level4_specificity() {
        let spec = |src: &str| parse(&format!("{src} {{ color: red }}")).rules[0].selectors[0].specificity();
        assert_eq!(spec("[data-x]"), (0, 1, 0));
        assert_eq!(spec("a[href][title]"), (0, 2, 1));
        assert_eq!(spec("li:nth-child(2n+1)"), (0, 1, 1));
        assert_eq!(spec("li:first-child"), (0, 1, 1));
        assert_eq!(spec("li:not(.skip)"), (0, 1, 1));
        assert_eq!(spec(":not(#a, .b, p)"), (1, 0, 0));
        assert_eq!(spec(":is(#a, .b)"), (1, 0, 0));
        assert_eq!(spec("p:is(.a .b, em)"), (0, 2, 1));
        assert_eq!(spec(":where(#a, .b) p"), (0, 0, 1));
        assert_eq!(spec("li:nth-child(odd of #x, .y)"), (1, 1, 1));
        assert_eq!(spec("h1 + p ~ .к"), (0, 1, 2));
    }

    #[test]
    fn test_deep_nesting_is_bounded() {
        let deep = format!("{}p{} {{ color: red }}", ":not(".repeat(5000), ")".repeat(5000));
        let ss = parse(&deep);
        assert!(ss.rules.is_empty() || ss.rules[0].selectors.is_empty());
        let ok = format!("{}p{} {{ color: red }}", ":is(".repeat(4), ")".repeat(4));
        assert_eq!(parse(&ok).rules[0].selectors.len(), 1);
    }
}
