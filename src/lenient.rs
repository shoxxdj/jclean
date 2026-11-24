//! Analyseur JSON tolérant : accepte ce que les humains (et JavaScript) écrivent
//! à la place du JSON strict, et le convertit en `serde_json::Value`.
//!
//! Tolère :
//! - clés sans guillemets            { apps: [...] }
//! - chaînes entre ' ou `            'texte', `texte`
//! - commentaires                    // ...   /* ... */   # ...
//! - virgules en trop / manquantes   [1, 2,, 3,]   { a: 1 b: 2 }
//! - `=` à la place de `:`
//! - nombres JS                      +1  .5  5.  0x1F  1_000
//! - mots-clés                       True/False/None/undefined/NaN/Infinity
//! - valeurs sans guillemets         { mode: production } -> "production"
//! - `module.exports =` / `var x =` en tête et `;` final

use serde_json::{Map, Number, Value};

pub struct ParseError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
}

pub fn parse_all(text: &str) -> Result<Vec<Value>, ParseError> {
    let mut p = Parser { s: text.chars().collect(), i: 0 };
    p.skip_js_prefix();
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        while matches!(p.peek(), Some(';') | Some(',')) {
            p.i += 1;
            p.skip_ws();
        }
        if p.peek().is_none() {
            break;
        }
        out.push(p.value()?);
    }
    Ok(out)
}

struct Parser {
    s: Vec<char>,
    i: usize,
}

fn is_delim(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | ':' | '=' | '{' | '}' | '[' | ']' | '"' | '\'' | '`' | ';')
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.s.get(self.i + off).copied()
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        self.err_at(self.i, msg)
    }

    fn err_at(&self, pos: usize, msg: impl Into<String>) -> ParseError {
        let before = &self.s[..pos.min(self.s.len())];
        let line = before.iter().filter(|&&c| c == '\n').count() + 1;
        let col = before.iter().rev().take_while(|&&c| c != '\n').count() + 1;
        ParseError { msg: msg.into(), line, col }
    }

    /// Saute `module.exports =`, `export default`, `const x =`, etc.
    fn skip_js_prefix(&mut self) {
        self.skip_ws();
        let start = self.i;
        let rest: String = self.s[self.i..].iter().take(200).collect();
        if let Some(pos) = rest.find(['{', '[']) {
            let head = rest[..pos].trim();
            let looks_js = head.ends_with('=')
                || head == "export default"
                || head.starts_with("module.exports")
                || head.starts_with("exports.");
            if looks_js && !head.contains(['"', '\'']) {
                self.i = start + rest[..pos].chars().count();
            }
        }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '\u{feff}' {
                self.i += 1;
            } else if c == '#' || (c == '/' && self.peek_at(1) == Some('/')) {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.i += 1;
                }
            } else if c == '/' && self.peek_at(1) == Some('*') {
                self.i += 2;
                while self.peek().is_some() && !(self.peek() == Some('*') && self.peek_at(1) == Some('/')) {
                    self.i += 1;
                }
                self.i = (self.i + 2).min(self.s.len());
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            None => Err(self.err("fin de fichier inattendue, une valeur était attendue")),
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some(q @ ('"' | '\'' | '`')) => Ok(Value::String(self.string(q)?)),
            Some(c) if c.is_ascii_digit() || matches!(c, '-' | '+' | '.') => Ok(self.number_or_word()),
            Some(c) if matches!(c, '}' | ']' | ',' | ':') => {
                Err(self.err(format!("caractère '{c}' inattendu, une valeur était attendue")))
            }
            Some(_) => Ok(self.word_value()),
        }
    }

    fn object(&mut self) -> Result<Value, ParseError> {
        let open = self.i;
        self.i += 1; // {
        let mut map = Map::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(self.err_at(open, "accolade '{' jamais refermée")),
                Some('}') => {
                    self.i += 1;
                    return Ok(Value::Object(map));
                }
                Some(',' | ';') => {
                    self.i += 1;
                    continue;
                }
                Some(']') => return Err(self.err("']' trouvé alors qu'on attendait '}'")),
                _ => {}
            }
            let key = self.key()?;
            self.skip_ws();
            match self.peek() {
                Some(':' | '=') => self.i += 1,
                _ => return Err(self.err(format!("':' attendu après la clé \"{key}\""))),
            }
            let v = self.value()?;
            map.insert(key, v);
        }
    }

    fn key(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(q @ ('"' | '\'' | '`')) => self.string(q),
            _ => {
                let start = self.i;
                while let Some(c) = self.peek() {
                    if is_delim(c) {
                        break;
                    }
                    self.i += 1;
                }
                if self.i == start {
                    return Err(self.err("nom de clé attendu"));
                }
                Ok(self.s[start..self.i].iter().collect())
            }
        }
    }

    fn array(&mut self) -> Result<Value, ParseError> {
        let open = self.i;
        self.i += 1; // [
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(self.err_at(open, "crochet '[' jamais refermé")),
                Some(']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                Some(',' | ';') => {
                    self.i += 1;
                    continue;
                }
                Some('}') => return Err(self.err("'}' trouvé alors qu'on attendait ']'")),
                _ => items.push(self.value()?),
            }
        }
    }

    fn string(&mut self, quote: char) -> Result<String, ParseError> {
        let open = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err_at(open, format!("chaîne ouverte par {quote} jamais refermée")));
            };
            self.i += 1;
            if c == quote {
                return Ok(out);
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let Some(e) = self.peek() else { continue };
            self.i += 1;
            match e {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'v' => out.push('\u{b}'),
                '0' => out.push('\0'),
                '\n' => {} // continuation de ligne
                '\r' => {
                    if self.peek() == Some('\n') {
                        self.i += 1;
                    }
                }
                'x' => match self.hex(2) {
                    Some(n) => out.push(char::from_u32(n).unwrap_or('\u{fffd}')),
                    None => out.push('x'),
                },
                'u' => match self.hex(4) {
                    Some(hi @ 0xD800..=0xDBFF)
                        if self.peek() == Some('\\') && self.peek_at(1) == Some('u') =>
                    {
                        self.i += 2;
                        let lo = self.hex(4).unwrap_or(0);
                        let cp = 0x10000 + ((hi - 0xD800) << 10) + lo.wrapping_sub(0xDC00);
                        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                    }
                    Some(n) => out.push(char::from_u32(n).unwrap_or('\u{fffd}')),
                    None => out.push('u'),
                },
                other => out.push(other), // \" \' \\ \/ et le reste tel quel
            }
        }
    }

    fn hex(&mut self, n: usize) -> Option<u32> {
        let txt: String = self.s.get(self.i..self.i + n)?.iter().collect();
        let v = u32::from_str_radix(&txt, 16).ok()?;
        self.i += n;
        Some(v)
    }

    fn read_word(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if is_delim(c) || c == '/' && matches!(self.peek_at(1), Some('/' | '*')) {
                break;
            }
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }

    fn number_or_word(&mut self) -> Value {
        let w = self.read_word();
        match to_number(&w) {
            Some(n) => Value::Number(n),
            None => word_to_value(w),
        }
    }

    /// Valeur sans guillemets : mot-clé, ou texte brut jusqu'au prochain séparateur.
    fn word_value(&mut self) -> Value {
        let start = self.i;
        let w = self.read_word();
        if w.is_empty() {
            // caractère isolé inconnu : on le garde comme chaîne
            self.i = start + 1;
            return Value::String(self.s[start].to_string());
        }
        word_to_value(w)
    }
}

fn word_to_value(w: String) -> Value {
    match w.as_str() {
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        "null" | "Null" | "NULL" | "None" | "nil" | "undefined" => Value::Null,
        // non représentables en JSON
        "NaN" | "Infinity" | "-Infinity" | "+Infinity" => Value::Null,
        _ => Value::String(w),
    }
}

fn to_number(w: &str) -> Option<Number> {
    let w = w.replace('_', "");
    let (neg, body) = match w.as_bytes().first()? {
        b'-' => (true, &w[1..]),
        b'+' => (false, &w[1..]),
        _ => (false, &w[..]),
    };
    let sign = if neg { "-" } else { "" };

    // hexadécimal / octal / binaire
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = body.strip_prefix(prefix) {
            let n = u128::from_str_radix(digits, radix).ok()?;
            return serde_json::from_str(&format!("{sign}{n}")).ok();
        }
    }

    // .5 -> 0.5   5. -> 5.0   5.e3 -> 5.0e3   007 -> 7
    let mut b = body.to_string();
    if b.starts_with('.') {
        b.insert(0, '0');
    }
    if let Some(p) = b.find('.') {
        if !b[p + 1..].starts_with(|c: char| c.is_ascii_digit()) {
            b.insert(p + 1, '0');
        }
    }
    let int_end = b.find(['.', 'e', 'E']).unwrap_or(b.len());
    let int_part = b[..int_end].trim_start_matches('0');
    let int_part = if int_part.is_empty() { "0" } else { int_part };
    let b = format!("{sign}{int_part}{}", &b[int_end..]);
    serde_json::from_str(&b).ok()
}
