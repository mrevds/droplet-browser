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
    pub unsupported_pseudo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplexSelector {
    pub parts: Vec<(Option<Combinator>, CompoundSelector)>,
}

impl ComplexSelector {
    pub fn specificity(&self) -> Specificity {
        let mut a = 0;
        let mut b = 0;
        let mut c = 0;
        for (_, compound) in &self.parts {
            if compound.id.is_some() {
                a += 1;
            }
            b += compound.classes.len();
            if compound.hover {
                b += 1;
            }
            if compound.active {
                b += 1;
            }
            if compound.tag_name.is_some() {
                c += 1;
            }
        }
        (a, b, c)
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

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
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
            match self.peek_char() {
                None | Some(',') | Some('{') => break,
                Some('>') => {
                    self.next_char();
                    self.skip_whitespace();
                    let next = self.parse_compound_selector()?;
                    parts.push((Some(Combinator::Child), next));
                }
                Some(_) if had_ws => {
                    let next = self.parse_compound_selector()?;
                    parts.push((Some(Combinator::Descendant), next));
                }
                Some(_) => {
                    // Unsupported combinator or token (e.g., '+', '~', ':', '[')
                    return None;
                }
            }
        }

        Some(ComplexSelector { parts })
    }

    fn parse_compound_selector(&mut self) -> Option<CompoundSelector> {
        let mut sel = CompoundSelector::default();
        let mut parsed_any = false;

        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() || ch == ',' || ch == '{' || ch == '>' {
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
                    let has_args = if self.peek_char() == Some('(') {
                        self.skip_paren_group();
                        true
                    } else {
                        false
                    };
                    if !is_pseudo_element && !has_args {
                        if ident.eq_ignore_ascii_case("hover") {
                            sel.hover = true;
                        } else if ident.eq_ignore_ascii_case("active") {
                            sel.active = true;
                        } else {
                            sel.unsupported_pseudo = true;
                        }
                    } else {
                        sel.unsupported_pseudo = true;
                    }
                    parsed_any = true;
                }
                _ => {
                    // Unsupported attribute ([attr]) or invalid syntax
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

fn parse_values(raw: &str) -> Vec<Value> {
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
            a, a:hover, a:nth-child(2), div::before { color: blue; }
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
}
