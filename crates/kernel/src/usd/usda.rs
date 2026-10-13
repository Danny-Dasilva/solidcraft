//! The part of the USD text format (`.usda`) that a `BrepArray` needs.
//!
//! The reader walks the prim tree, keeps the stage's `metersPerUnit` and `upAxis`, and keeps the
//! attributes of every `def BrepArray` prim wherever it sits. Everything else (meshes, materials,
//! relationships, dictionaries, time samples, variant sets, composition arcs) is skipped without
//! being kept. Files are untrusted: nesting, string length and the count of kept numbers are
//! capped, and any malformed input is an error, never a panic.

use std::collections::HashMap;

type R<T> = Result<T, String>;

/// Deepest nesting of prims, lists and dictionaries.
const MAX_DEPTH: usize = 64;
/// Most numbers kept from `BrepArray` attributes (8 bytes each).
const MAX_NUMBERS: usize = 24 << 20;
/// Most strings kept from `BrepArray` attributes.
const MAX_STRINGS: usize = 8 << 20;
/// Longest string read.
const MAX_STRING: usize = 1 << 20;
/// Most `BrepArray` prims kept.
const MAX_PRIMS: usize = 10_000;
/// Longest `<path>` or `@asset@` read.
const MAX_REFERENCE: usize = 1 << 16;

/// Attribute names kept from a `BrepArray` prim: the schema's own namespaces.
const KEPT: &[&str] =
    &["brep:", "region:", "shell:", "faceuse:", "face:", "loop:", "edgeuse:", "edge:", "wireEdge:", "vertex:", "primvars:displayColor"];

/// A value as far as the importer needs it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Num(f64),
    /// A string, or a bare word such as a token or `None`.
    Str(String),
    /// An array of numbers or of tuples of numbers (`width` numbers per element), or one tuple.
    Nums {
        width: usize,
        data: Vec<f64>,
    },
    Strs(Vec<String>),
    /// Anything the importer does not read.
    Other,
}

impl Value {
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(x) => Some(*x),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The numbers of an array whose elements have `width` numbers (an empty array fits any).
    pub(crate) fn nums(&self, width: usize) -> Option<&[f64]> {
        match self {
            Value::Nums { width: w, data } if *w == width || data.is_empty() => Some(data),
            _ => None,
        }
    }

    /// The numbers of an array of numbers or of tuples of any size.
    pub(crate) fn flat(&self) -> Option<&[f64]> {
        match self {
            Value::Nums { data, .. } => Some(data),
            _ => None,
        }
    }

    /// The strings of an array of strings (an empty array of anything is one).
    pub(crate) fn strs(&self) -> Option<&[String]> {
        match self {
            Value::Strs(v) => Some(v),
            Value::Nums { data, .. } if data.is_empty() => Some(&[]),
            _ => None,
        }
    }
}

/// A `def BrepArray` prim.
#[derive(Debug)]
pub(crate) struct BrepPrim {
    pub path: String,
    pub name: String,
    pub attrs: HashMap<String, Value>,
    /// The transform operations of the prim and its ancestors that have any, outermost first.
    pub xforms: Vec<Xform>,
    /// The name in `customData` (`solidcraft:name`), if the prim has one.
    pub title: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Layer {
    pub meters_per_unit: Option<f64>,
    pub up_axis: Option<String>,
    pub breps: Vec<BrepPrim>,
}

/// The transform operations one prim lists in `xformOpOrder`, with their values.
#[derive(Clone, Debug, Default)]
pub(crate) struct Xform {
    /// `xformOpOrder`: `xformOp:translate`, `!invert!xformOp:translate:pivot`, `!resetXformStack!`, ...
    pub order: Vec<String>,
    /// Each `xformOp:*` attribute with a plain value.
    pub ops: HashMap<String, Value>,
    /// An operation has time samples, which are not followed.
    pub animated: bool,
}

/// An open prim while its body is read.
struct Frame {
    name: String,
    xform: Xform,
}

struct Meta {
    active: bool,
    /// `customData` string `solidcraft:name`, nested or flat.
    title: Option<String>,
    /// An attribute's `interpolation`.
    interpolation: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Empty,
    Nums(usize),
    Strs,
    Other,
}

/// The elements of a list being read.
struct Acc {
    kind: Kind,
    nums: Vec<f64>,
    strs: Vec<String>,
}

struct P<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    numbers: usize,
    strings: usize,
    layer: Layer,
    stack: Vec<Frame>,
}

/// Parse USD text. Only the stage metadata and the `BrepArray` prims are kept.
pub(crate) fn parse(text: &str) -> R<Layer> {
    let mut p = P { s: text, b: text.as_bytes(), pos: 0, numbers: 0, strings: 0, layer: Layer::default(), stack: Vec::new() };
    p.layer()?;
    Ok(p.layer)
}

fn special_number(w: &str) -> Option<f64> {
    match w.to_ascii_lowercase().as_str() {
        "nan" => Some(f64::NAN),
        "inf" | "infinity" => Some(f64::INFINITY),
        _ => None,
    }
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'.') || c >= 0x80
}

fn is_number_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-')
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn at(&self, k: usize) -> Option<u8> {
        self.b.get(self.pos + k).copied()
    }

    fn err<T>(&self, msg: impl std::fmt::Display) -> R<T> {
        let line = 1 + self.b.get(..self.pos.min(self.b.len())).unwrap_or(&[]).iter().filter(|c| **c == b'\n').count();
        Err(format!("line {line}: {msg}"))
    }

    /// Skip white space and `#` comments.
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b'#' => {
                    while self.peek().is_some_and(|c| c != b'\n') {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// A run of name characters (possibly empty), with a trailing `[]` of an array type.
    fn word(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_word_byte) {
            self.pos += 1;
        }
        let mut w = self.s.get(start..self.pos).unwrap_or("").to_string();
        if !w.is_empty() && self.peek() == Some(b'[') && self.at(1) == Some(b']') {
            self.pos += 2;
            w.push_str("[]");
        }
        w
    }

    fn layer(&mut self) -> R<()> {
        if self.s.starts_with('\u{feff}') {
            self.pos = '\u{feff}'.len_utf8();
        }
        let head = self.s.get(self.pos..).unwrap_or("");
        if head.starts_with("PXR-USDC") {
            return Err("this is a binary USD file (.usdc); save it as text (.usda) first".into());
        }
        // `#usda 1.0` for .usda files, `#sdf 1.4.32` for the older text layers: the same syntax.
        let magic = ["#usda", "#sdf"].into_iter().find(|m| head.starts_with(m));
        if !magic.is_some_and(|m| head.get(m.len()..).is_some_and(|r| r.starts_with([' ', '\t']))) {
            return Err("not a USD text file (the first line must be '#usda 1.0')".into());
        }
        self.skip_ws();
        if self.peek() == Some(b'(') {
            self.metadata(true)?;
        }
        loop {
            self.skip_ws();
            if self.peek().is_none() {
                return Ok(());
            }
            let w = self.word();
            match w.as_str() {
                "def" | "over" | "class" => self.prim(&w, 0, true)?,
                "reorder" => {
                    // reorder rootPrims = [...]
                    self.skip_ws();
                    self.word();
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return self.err("expected '='");
                    }
                    self.pos += 1;
                    self.value(false, 0)?;
                }
                "" => return self.err("unexpected character"),
                _ => return self.err(format!("unexpected '{w}'")),
            }
        }
    }

    /// A `( key = value ... )` block, at its opening parenthesis.
    fn metadata(&mut self, stage: bool) -> R<Meta> {
        self.pos += 1;
        let mut meta = Meta { active: true, title: None, interpolation: None };
        loop {
            self.skip_ws();
            match self.peek() {
                None => return self.err("unexpected end of file in metadata"),
                Some(b')') => {
                    self.pos += 1;
                    return Ok(meta);
                }
                Some(b';') => {
                    self.pos += 1;
                    continue;
                }
                // A lone string is the prim's documentation.
                Some(b'"') | Some(b'\'') => {
                    self.string()?;
                    continue;
                }
                _ => {}
            }
            // List-edit words (`prepend`, `add`, ...) come before the key.
            let mut key = self.word();
            loop {
                if key.is_empty() {
                    return self.err("unexpected character in metadata");
                }
                self.skip_ws();
                if self.peek() == Some(b'=') {
                    break;
                }
                key = self.word();
            }
            self.pos += 1;
            self.skip_ws();
            if !stage && key == "customData" && self.peek() == Some(b'{') {
                // The dictionary is read for the name; one this reader does not follow is skipped.
                let start = self.pos;
                let mut entries = Vec::new();
                if self.dictionary("", &mut entries, 0).is_err() {
                    self.pos = start;
                    self.skip_braced()?;
                    entries.clear();
                }
                if let Some((_, name)) = entries.into_iter().find(|(k, _)| k == "solidcraft:name") {
                    meta.title = Some(name);
                }
                continue;
            }
            let v = self.value(true, 0)?;
            match (stage, key.as_str()) {
                (true, "metersPerUnit") => self.layer.meters_per_unit = v.as_f64(),
                (true, "upAxis") => self.layer.up_axis = v.as_str().map(str::to_string),
                (false, "active") => meta.active = v.as_str() != Some("false"),
                (false, "interpolation") => meta.interpolation = v.as_str().map(str::to_string),
                _ => {}
            }
        }
    }

    /// A prim after its specifier: `[Type] "name" [( metadata )] { body }`.
    fn prim(&mut self, spec: &str, depth: usize, live: bool) -> R<()> {
        if depth > MAX_DEPTH {
            return self.err("prims are nested too deeply");
        }
        self.skip_ws();
        let type_name = if self.peek() == Some(b'"') { String::new() } else { self.word() };
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return self.err("expected a prim name in quotes");
        }
        let name = self.string()?;
        self.skip_ws();
        let meta = if self.peek() == Some(b'(') { self.metadata(false)? } else { Meta { active: true, title: None, interpolation: None } };
        let active = meta.active;
        self.skip_ws();
        if self.peek() != Some(b'{') {
            return self.err("expected '{'");
        }
        self.pos += 1;
        let live = live && active && spec == "def";
        let collect = live && type_name == "BrepArray";
        if collect && self.layer.breps.len() >= MAX_PRIMS {
            return self.err(format!("more than {MAX_PRIMS} BrepArray prims"));
        }
        self.stack.push(Frame { name: name.clone(), xform: Xform::default() });
        let mut attrs = HashMap::new();
        let body = self.body(depth, live, collect, &mut attrs);
        let frame = self.stack.pop();
        body?;
        if collect {
            let path = format!("/{}", self.stack.iter().map(|f| f.name.as_str()).chain(std::iter::once(name.as_str())).collect::<Vec<_>>().join("/"));
            let xforms: Vec<Xform> = self.stack.iter().chain(frame.as_ref()).map(|f| f.xform.clone()).filter(|x| !x.order.is_empty()).collect();
            self.layer.breps.push(BrepPrim { path, name, attrs, xforms, title: meta.title });
        }
        Ok(())
    }

    /// The statements of a prim, up to its closing brace.
    fn body(&mut self, depth: usize, live: bool, collect: bool, attrs: &mut HashMap<String, Value>) -> R<()> {
        loop {
            self.skip_ws();
            match self.peek() {
                None => return self.err("unexpected end of file (a prim is not closed)"),
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(b';') => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            let w = self.word();
            match w.as_str() {
                "" => return self.err("unexpected character in a prim"),
                "def" | "over" | "class" => self.prim(&w, depth + 1, live)?,
                "variantSet" => {
                    self.skip_ws();
                    self.string()?;
                    self.skip_ws();
                    if self.peek() != Some(b'=') {
                        return self.err("expected '='");
                    }
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() != Some(b'{') {
                        return self.err("expected '{'");
                    }
                    self.skip_braced()?;
                }
                _ => self.property(w, collect, attrs)?,
            }
        }
    }

    /// An attribute or relationship, after its first word.
    fn property(&mut self, first: String, collect: bool, attrs: &mut HashMap<String, Value>) -> R<()> {
        let mut w = first;
        while matches!(w.as_str(), "custom" | "uniform" | "varying" | "config" | "add" | "prepend" | "append" | "delete" | "reorder") {
            self.skip_ws();
            w = self.word();
        }
        if w.is_empty() {
            return self.err("incomplete property");
        }
        self.skip_ws();
        // The first word is the type, except in `reorder nameChildren = [...]`.
        let name = if self.peek() == Some(b'=') {
            w
        } else {
            let n = self.word();
            if n.is_empty() {
                return self.err(format!("expected a name after '{w}'"));
            }
            n
        };
        let keep = (collect && KEPT.iter().any(|p| name.starts_with(p))) || name.starts_with("xformOp:") || name == "xformOpOrder";
        self.skip_ws();
        let mut value = Value::Other;
        if self.peek() == Some(b'=') {
            self.pos += 1;
            value = self.value(keep, 0)?;
        }
        self.skip_ws();
        let mut interpolation = None;
        if self.peek() == Some(b'(') {
            interpolation = self.metadata(false)?.interpolation;
        }
        if let Some(f) = self.stack.last_mut() {
            if name == "xformOpOrder" {
                if let Value::Strs(order) = &value {
                    f.xform.order = order.clone();
                }
            } else if name.starts_with("xformOp:") {
                if name.contains('.') {
                    f.xform.animated = true;
                } else {
                    f.xform.ops.insert(name.clone(), value.clone());
                }
            }
        }
        if keep && collect && !name.contains('.') && !name.starts_with("xformOp") {
            // A primvar's interpolation is metadata on it, or a property of its own.
            if let Some(i) = interpolation {
                attrs.entry(format!("{name}:interpolation")).or_insert(Value::Str(i));
            }
            attrs.insert(name, value);
        }
        Ok(())
    }

    /// Any value; `keep` stores the numbers and strings of lists, otherwise they are skipped.
    fn value(&mut self, keep: bool, depth: usize) -> R<Value> {
        if depth > MAX_DEPTH {
            return self.err("values are nested too deeply");
        }
        self.skip_ws();
        match self.peek() {
            None => self.err("unexpected end of file in a value"),
            Some(b'"') | Some(b'\'') => Ok(Value::Str(self.string()?)),
            Some(b'@') => {
                self.skip_asset()?;
                Ok(Value::Other)
            }
            Some(b'<') => {
                self.skip_path()?;
                Ok(Value::Other)
            }
            Some(b'{') => {
                self.skip_braced()?;
                Ok(Value::Other)
            }
            Some(b'[') => self.list(keep, depth),
            Some(b'(') => {
                let mut nums = Vec::new();
                match self.tuple(&mut nums, keep, depth)? {
                    Some(n) => Ok(Value::Nums { width: n, data: nums }),
                    None => Ok(Value::Other),
                }
            }
            Some(c) if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() => Ok(Value::Num(self.number()?)),
            Some(_) => {
                let w = self.word();
                if w.is_empty() {
                    return self.err("unexpected character in a value");
                }
                Ok(special_number(&w).map_or(Value::Str(w), Value::Num))
            }
        }
    }

    fn number(&mut self) -> R<f64> {
        let start = self.pos;
        while self.peek().is_some_and(is_number_byte) {
            self.pos += 1;
        }
        let t = self.s.get(start..self.pos).unwrap_or("");
        match t.parse::<f64>() {
            Ok(x) => Ok(x),
            Err(_) => self.err(format!("bad number '{}'", t.chars().take(24).collect::<String>())),
        }
    }

    /// A tuple `(a, b, ...)`, nested tuples flattened, at its opening parenthesis. The numbers
    /// are appended to `out` (when `keep`) and counted; `None` when it holds anything else.
    fn tuple(&mut self, out: &mut Vec<f64>, keep: bool, depth: usize) -> R<Option<usize>> {
        if depth > MAX_DEPTH {
            return self.err("values are nested too deeply");
        }
        self.pos += 1;
        let mut count = 0usize;
        let mut numeric = true;
        loop {
            self.skip_ws();
            match self.peek() {
                None => return self.err("unexpected end of file in a tuple"),
                Some(b')') => {
                    self.pos += 1;
                    break;
                }
                Some(b',') => {
                    self.pos += 1;
                    continue;
                }
                Some(b'(') => match self.tuple(out, keep, depth + 1)? {
                    Some(n) => count += n,
                    None => numeric = false,
                },
                Some(c) if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() => {
                    let x = self.number()?;
                    count += 1;
                    if keep {
                        self.numbers += 1;
                        if self.numbers > MAX_NUMBERS {
                            return self.err("too many numbers");
                        }
                        out.push(x);
                    }
                }
                Some(_) => {
                    let v = self.value(false, depth + 1)?;
                    match v {
                        Value::Num(x) => {
                            count += 1;
                            if keep {
                                self.numbers += 1;
                                if self.numbers > MAX_NUMBERS {
                                    return self.err("too many numbers");
                                }
                                out.push(x);
                            }
                        }
                        _ => numeric = false,
                    }
                }
            }
        }
        Ok(numeric.then_some(count))
    }

    /// A list `[a, b, ...]`, at its opening bracket.
    fn list(&mut self, keep: bool, depth: usize) -> R<Value> {
        self.pos += 1;
        let mut acc = Acc { kind: Kind::Empty, nums: Vec::new(), strs: Vec::new() };
        loop {
            self.skip_ws();
            match self.peek() {
                None => return self.err("unexpected end of file in a list"),
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                Some(b',') => {
                    self.pos += 1;
                    continue;
                }
                Some(b'(') => {
                    let before = acc.nums.len();
                    match self.tuple(&mut acc.nums, keep, depth + 1)? {
                        Some(n) if matches!(acc.kind, Kind::Empty) || acc.kind == Kind::Nums(n) => acc.kind = Kind::Nums(n),
                        _ => {
                            acc.kind = Kind::Other;
                            acc.nums.truncate(before);
                        }
                    }
                }
                Some(c) if c == b'"' || c == b'\'' => {
                    let s = self.string()?;
                    if matches!(acc.kind, Kind::Empty | Kind::Strs) {
                        acc.kind = Kind::Strs;
                        if keep {
                            self.strings += 1;
                            if self.strings > MAX_STRINGS {
                                return self.err("too many strings");
                            }
                            acc.strs.push(s);
                        }
                    } else {
                        acc.kind = Kind::Other;
                    }
                }
                Some(_) => match self.value(false, depth + 1)? {
                    Value::Num(x) if matches!(acc.kind, Kind::Empty | Kind::Nums(1)) => {
                        acc.kind = Kind::Nums(1);
                        if keep {
                            self.numbers += 1;
                            if self.numbers > MAX_NUMBERS {
                                return self.err("too many numbers");
                            }
                            acc.nums.push(x);
                        }
                    }
                    _ => acc.kind = Kind::Other,
                },
            }
            if acc.kind == Kind::Other {
                acc.nums.clear();
                acc.strs.clear();
            }
        }
        Ok(match acc.kind {
            Kind::Empty => Value::Nums { width: 0, data: Vec::new() },
            Kind::Nums(w) => Value::Nums { width: w, data: acc.nums },
            Kind::Strs => Value::Strs(acc.strs),
            Kind::Other => Value::Other,
        })
    }

    /// A quoted string (`"…"`, `'…'`, or triple-quoted over several lines), at its opening quote.
    fn string(&mut self) -> R<String> {
        let Some(q) = self.peek() else { return self.err("expected a string") };
        self.pos += 1;
        let triple = self.peek() == Some(q) && self.at(1) == Some(q);
        if triple {
            self.pos += 2;
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(c) = self.peek() else { return self.err("a string is not closed") };
            self.pos += 1;
            if c == q {
                if !triple {
                    break;
                }
                if self.peek() == Some(q) && self.at(1) == Some(q) {
                    self.pos += 2;
                    break;
                }
            } else if c == b'\\' {
                let Some(n) = self.peek() else { return self.err("a string is not closed") };
                self.pos += 1;
                out.push(match n {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    other => other,
                });
                continue;
            } else if (c == b'\n' || c == b'\r') && !triple {
                return self.err("a string is not closed on its line");
            }
            out.push(c);
            if out.len() > MAX_STRING {
                return self.err("a string is too long");
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// `<path>`, at its opening angle bracket.
    fn skip_path(&mut self) -> R<()> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == b'>' {
                return Ok(());
            }
            if c == b'\n' || self.pos - start > MAX_REFERENCE {
                break;
            }
        }
        self.err("a path is not closed")
    }

    /// `@asset@` or `@@@asset@@@`, with the `<path>` that may follow, at its first at-sign.
    fn skip_asset(&mut self) -> R<()> {
        let triple = self.at(1) == Some(b'@') && self.at(2) == Some(b'@');
        let start = self.pos;
        self.pos += if triple { 3 } else { 1 };
        loop {
            let Some(c) = self.peek() else { return self.err("an asset path is not closed") };
            if self.pos - start > MAX_REFERENCE || (!triple && c == b'\n') {
                return self.err("an asset path is not closed");
            }
            if c == b'\\' && triple && self.at(1) == Some(b'@') {
                self.pos += 2;
                continue;
            }
            self.pos += 1;
            if c == b'@' && (!triple || (self.peek() == Some(b'@') && self.at(1) == Some(b'@'))) {
                if triple {
                    self.pos += 2;
                }
                break;
            }
        }
        if self.peek() == Some(b'<') {
            self.skip_path()?;
        }
        Ok(())
    }

    /// A dictionary `{ type key = value ... }`, at its opening brace: the string entries, with the
    /// keys of nested dictionaries joined by `:` (`solidcraft:name`).
    fn dictionary(&mut self, prefix: &str, out: &mut Vec<(String, String)>, depth: usize) -> R<()> {
        if depth > MAX_DEPTH {
            return self.err("dictionaries are nested too deeply");
        }
        self.pos += 1;
        loop {
            self.skip_ws();
            match self.peek() {
                None => return self.err("unexpected end of file in a dictionary"),
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(b';') => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            let ty = self.word();
            self.skip_ws();
            let key = if matches!(self.peek(), Some(b'"') | Some(b'\'')) { self.string()? } else { self.word() };
            self.skip_ws();
            if ty.is_empty() || key.is_empty() || self.peek() != Some(b'=') {
                return self.err("unexpected text in a dictionary");
            }
            self.pos += 1;
            self.skip_ws();
            if self.peek() == Some(b'{') {
                self.dictionary(&format!("{prefix}{key}:"), out, depth + 1)?;
            } else if let Value::Str(v) = self.value(false, depth + 1)? {
                out.push((format!("{prefix}{key}"), v));
            }
        }
    }

    /// A `{ ... }` block (dictionary, time samples, variants), at its opening brace.
    fn skip_braced(&mut self) -> R<()> {
        let mut depth = 0usize;
        loop {
            let Some(c) = self.peek() else { return self.err("unexpected end of file in a dictionary") };
            match c {
                b'{' => {
                    depth += 1;
                    if depth > MAX_DEPTH {
                        return self.err("dictionaries are nested too deeply");
                    }
                    self.pos += 1;
                }
                b'}' => {
                    self.pos += 1;
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Ok(());
                    }
                }
                b'"' | b'\'' => {
                    self.string()?;
                }
                b'@' => self.skip_asset()?,
                b'<' => self.skip_path()?,
                b'#' => self.skip_ws(),
                _ => self.pos += 1,
            }
        }
    }
}
