//! jclean — pretty-prints JSON files, even messy or broken ones.
//!
//! Usage: jclean [-v] [--no-color] [--indent N] <file | ->
//!
//! - ignores tabs, spaces and newlines anywhere
//! - accepts a UTF-8 BOM and several JSON documents in a row (NDJSON)
//! - reads hand-written / JavaScript-style JSON (unquoted keys, ' strings,
//!   comments, extra commas...) through a lenient fallback (see lenient.rs)
//! - keeps key order and number precision
//! - colors output when writing to a terminal

mod lenient;

use serde_json::{Deserializer, Value};
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

struct Style {
    color: bool,
    indent: usize,
}

const RESET: &str = "\x1b[0m";
const KEY: &str = "\x1b[1;34m"; // bold blue
const STR: &str = "\x1b[32m"; // green
const NUM: &str = "\x1b[33m"; // yellow
const BOOL: &str = "\x1b[35m"; // magenta
const NULL: &str = "\x1b[90m"; // grey
const PUNCT: &str = "\x1b[37m"; // white

impl Style {
    fn paint(&self, out: &mut impl Write, color: &str, text: &str) -> io::Result<()> {
        if self.color {
            write!(out, "{color}{text}{RESET}")
        } else {
            out.write_all(text.as_bytes())
        }
    }

    fn pad(&self, out: &mut impl Write, level: usize) -> io::Result<()> {
        write!(out, "{:width$}", "", width = level * self.indent)
    }

    fn write_value(&self, out: &mut impl Write, v: &Value, level: usize) -> io::Result<()> {
        match v {
            Value::Null => self.paint(out, NULL, "null"),
            Value::Bool(b) => self.paint(out, BOOL, if *b { "true" } else { "false" }),
            Value::Number(n) => self.paint(out, NUM, &n.to_string()),
            Value::String(s) => self.paint(out, STR, &quote(s)),
            Value::Array(items) => {
                if items.is_empty() {
                    return self.paint(out, PUNCT, "[]");
                }
                self.paint(out, PUNCT, "[")?;
                for (i, item) in items.iter().enumerate() {
                    out.write_all(b"\n")?;
                    self.pad(out, level + 1)?;
                    self.write_value(out, item, level + 1)?;
                    if i + 1 < items.len() {
                        self.paint(out, PUNCT, ",")?;
                    }
                }
                out.write_all(b"\n")?;
                self.pad(out, level)?;
                self.paint(out, PUNCT, "]")
            }
            Value::Object(map) => {
                if map.is_empty() {
                    return self.paint(out, PUNCT, "{}");
                }
                self.paint(out, PUNCT, "{")?;
                let len = map.len();
                for (i, (k, val)) in map.iter().enumerate() {
                    out.write_all(b"\n")?;
                    self.pad(out, level + 1)?;
                    self.paint(out, KEY, &quote(k))?;
                    self.paint(out, PUNCT, ": ")?;
                    self.write_value(out, val, level + 1)?;
                    if i + 1 < len {
                        self.paint(out, PUNCT, ",")?;
                    }
                }
                out.write_all(b"\n")?;
                self.pad(out, level)?;
                self.paint(out, PUNCT, "}")
            }
        }
    }
}

/// Properly escaped JSON string (quotes included).
fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("a String always serializes")
}

fn usage(prog: &str) -> ExitCode {
    eprintln!("Usage: {prog} [-v] [--no-color] [--indent N] <file>");
    eprintln!("       {prog} -          (read from stdin)");
    eprintln!();
    eprintln!("  -v, --verbose     print parsing info on stderr");
    eprintln!("  -i, --indent N    indentation width (default: 2)");
    eprintln!("      --color       force colors");
    eprintln!("      --no-color    disable colors (also: NO_COLOR env var)");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let prog = args.first().map(String::as_str).unwrap_or("jclean");

    let mut color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut indent = 2usize;
    let mut verbose = false;
    let mut path: Option<String> = None;

    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return usage(prog),
            "-v" | "--verbose" => verbose = true,
            "--no-color" => color = false,
            "--color" => color = true,
            "--indent" | "-i" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) => indent = n,
                None => return usage(prog),
            },
            _ if path.is_none() => path = Some(a.clone()),
            _ => return usage(prog),
        }
    }
    let Some(path) = path else { return usage(prog) };

    // Read file (or stdin)
    let raw = if path == "-" {
        let mut buf = Vec::new();
        if let Err(e) = io::Read::read_to_end(&mut io::stdin(), &mut buf) {
            eprintln!("error: cannot read stdin: {e}");
            return ExitCode::FAILURE;
        }
        buf
    } else {
        match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: cannot read '{path}': {e}");
                return ExitCode::FAILURE;
            }
        }
    };

    let text = String::from_utf8_lossy(&raw);
    if verbose && matches!(text, std::borrow::Cow::Owned(_)) {
        eprintln!("info: invalid UTF-8 bytes replaced with U+FFFD");
    }
    let text = text.trim_start_matches('\u{feff}');

    // 1) strict JSON (exact, fast)
    let strict: Result<Vec<Value>, _> = Deserializer::from_str(text).into_iter::<Value>().collect();
    let docs = match strict {
        Ok(docs) => {
            if verbose {
                eprintln!("info: '{path}': valid JSON");
            }
            docs
        }
        Err(strict_err) => {
            if verbose {
                eprintln!("info: '{path}': not valid JSON ({strict_err})");
                eprintln!("info: falling back to lenient mode");
            }
            // 2) lenient mode
            match lenient::parse_all(text) {
                Ok(docs) => docs,
                Err(e) => {
                    eprintln!("error: cannot parse '{path}': {} (line {}, column {})", e.msg, e.line, e.col);
                    if let Some(ctx) = text.lines().nth(e.line.saturating_sub(1)) {
                        eprintln!("  {}", ctx.replace('\t', " "));
                        eprintln!("  {:>width$}", "^", width = e.col.max(1));
                    }
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    if docs.is_empty() {
        eprintln!("error: '{path}' contains no JSON");
        return ExitCode::FAILURE;
    }
    if verbose {
        eprintln!("info: {} document(s), {} bytes", docs.len(), raw.len());
    }

    let style = Style { color, indent };
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for (i, v) in docs.iter().enumerate() {
        let r = (if i > 0 { out.write_all(b"\n") } else { Ok(()) })
            .and_then(|_| style.write_value(&mut out, v, 0))
            .and_then(|_| out.write_all(b"\n"));
        if r.is_err() {
            return ExitCode::SUCCESS; // e.g. output closed by `| head`
        }
    }
    let _ = out.flush();
    ExitCode::SUCCESS
}
