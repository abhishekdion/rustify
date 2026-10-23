//! The runtime prelude emitted alongside every transpiled module.
//!
//! It provides the Python semantics that Rust's standard library does not
//! offer directly: negative indexing, Python floor division and modulo,
//! byte packing/unpacking with struct format codes, and a small dynamic
//! `PyValue` used as the fallback type for values the compiler cannot type
//! statically.

pub const PRELUDE: &str = r##"//! py2rs runtime prelude — generated support code.
#![allow(dead_code, unused_imports, unused_mut, clippy::all)]

pub use std::collections::{HashMap, HashSet};
pub use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PyErr {
    pub kind: String,
    pub message: String,
}

impl PyErr {
    pub fn new(kind: &str, message: &str) -> Self {
        PyErr {
            kind: kind.to_string(),
            message: message.to_string(),
        }
    }
}

impl std::fmt::Display for PyErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for PyErr {}

// ---------------------------------------------------------------------------
// Numeric helpers
// ---------------------------------------------------------------------------

/// Python's floor division: rounds towards negative infinity.
pub fn py_floordiv_i64(a: i64, b: i64) -> i64 {
    if b == 0 {
        panic!("ZeroDivisionError: integer division or modulo by zero");
    }
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

/// Python's modulo: the sign follows the divisor.
pub fn py_mod_i64(a: i64, b: i64) -> i64 {
    if b == 0 {
        panic!("ZeroDivisionError: integer division or modulo by zero");
    }
    let r = a % b;
    if r != 0 && ((r < 0) != (b < 0)) {
        r + b
    } else {
        r
    }
}

pub fn py_round_to(v: f64, digits: i64) -> f64 {
    let f = 10f64.powi(digits as i32);
    (v * f).round() / f
}

// ---------------------------------------------------------------------------
// Sequence helpers (Python index semantics)
// ---------------------------------------------------------------------------

fn py_norm_index(i: i64, len: i64) -> i64 {
    if i < 0 {
        i + len
    } else {
        i
    }
}

pub fn py_bytes_get(v: &[u8], i: i64) -> i64 {
    let idx = py_norm_index(i, v.len() as i64);
    if idx < 0 || idx >= v.len() as i64 {
        panic!("IndexError: byte index out of range");
    }
    v[idx as usize] as i64
}

pub fn py_list_get<T: Clone>(v: &[T], i: i64) -> T {
    let idx = py_norm_index(i, v.len() as i64);
    if idx < 0 || idx >= v.len() as i64 {
        panic!("IndexError: list index out of range");
    }
    v[idx as usize].clone()
}

pub fn py_str_get(v: &str, i: i64) -> String {
    let chars: Vec<char> = v.chars().collect();
    let idx = py_norm_index(i, chars.len() as i64);
    if idx < 0 || idx >= chars.len() as i64 {
        panic!("IndexError: string index out of range");
    }
    chars[idx as usize].to_string()
}

pub fn py_bytes_slice(v: &[u8], start: Option<i64>, stop: Option<i64>) -> Vec<u8> {
    let len = v.len() as i64;
    let mut s = start.map(|x| py_norm_index(x, len)).unwrap_or(0);
    let mut e = stop.map(|x| py_norm_index(x, len)).unwrap_or(len);
    if s < 0 {
        s = 0;
    }
    if e < 0 {
        e = 0;
    }
    if e > len {
        e = len;
    }
    if s >= e {
        return Vec::new();
    }
    v[s as usize..e as usize].to_vec()
}

pub fn py_list_slice<T: Clone>(v: &[T], start: Option<i64>, stop: Option<i64>) -> Vec<T> {
    let len = v.len() as i64;
    let mut s = start.map(|x| py_norm_index(x, len)).unwrap_or(0);
    let mut e = stop.map(|x| py_norm_index(x, len)).unwrap_or(len);
    if s < 0 {
        s = 0;
    }
    if e < 0 {
        e = 0;
    }
    if e > len {
        e = len;
    }
    if s >= e {
        return Vec::new();
    }
    v[s as usize..e as usize].to_vec()
}

pub fn py_str_slice(v: &str, start: Option<i64>, stop: Option<i64>) -> String {
    let chars: Vec<char> = v.chars().collect();
    let len = chars.len() as i64;
    let mut s = start.map(|x| py_norm_index(x, len)).unwrap_or(0);
    let mut e = stop.map(|x| py_norm_index(x, len)).unwrap_or(len);
    if s < 0 {
        s = 0;
    }
    if e < 0 {
        e = 0;
    }
    if e > len {
        e = len;
    }
    if s >= e {
        return String::new();
    }
    chars[s as usize..e as usize].iter().collect()
}

pub fn py_bytes_contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// Length / aggregates
// ---------------------------------------------------------------------------

pub fn py_len_bytes(v: &[u8]) -> i64 {
    v.len() as i64
}
pub fn py_len_str(v: &str) -> i64 {
    v.chars().count() as i64
}
pub fn py_len_vec<T>(v: &[T]) -> i64 {
    v.len() as i64
}
pub fn py_len_map<K, V>(v: &HashMap<K, V>) -> i64 {
    v.len() as i64
}

pub fn py_sum_bytes(v: &[u8], start: i64) -> i64 {
    v.iter().map(|b| *b as i64).sum::<i64>() + start
}

/// Numeric element trait for the aggregate helpers.
pub trait PyNum: Copy {
    fn py_i64(self) -> i64;
}
impl PyNum for i64 {
    fn py_i64(self) -> i64 {
        self
    }
}
impl PyNum for u8 {
    fn py_i64(self) -> i64 {
        self as i64
    }
}
impl PyNum for f64 {
    fn py_i64(self) -> i64 {
        self as i64
    }
}

pub fn py_sum_vec<T: PyNum>(v: &[T], start: i64) -> i64 {
    v.iter().map(|x| x.py_i64()).sum::<i64>() + start
}

pub fn py_sum_f64_vec<T: Copy + Into<f64>>(v: &[T], start: f64) -> f64 {
    v.iter().map(|x| (*x).into()).sum::<f64>() + start
}

pub fn py_min_bytes(v: &[u8]) -> i64 {
    v.iter().map(|b| *b as i64).min().unwrap_or(0)
}
pub fn py_max_bytes(v: &[u8]) -> i64 {
    v.iter().map(|b| *b as i64).max().unwrap_or(0)
}

pub fn py_min_vec<T: Copy + PartialOrd>(v: &[T]) -> T {
    let mut it = v.iter();
    let first = *it.next().expect("min() arg is an empty sequence");
    let mut best = first;
    for x in it {
        if *x < best {
            best = *x;
        }
    }
    best
}

pub fn py_max_vec<T: Copy + PartialOrd>(v: &[T]) -> T {
    let mut it = v.iter();
    let first = *it.next().expect("max() arg is an empty sequence");
    let mut best = first;
    for x in it {
        if *x > best {
            best = *x;
        }
    }
    best
}

pub fn py_any<T: Copy + Into<i64>>(v: &[T]) -> bool {
    v.iter().any(|x| (*x).into() != 0)
}

pub fn py_all<T: Copy + Into<i64>>(v: &[T]) -> bool {
    v.iter().all(|x| (*x).into() != 0)
}

pub fn py_list_index<T: PartialEq>(v: &[T], needle: T) -> i64 {
    match v.iter().position(|x| x == &needle) {
        Some(i) => i as i64,
        None => panic!("ValueError: value is not in list"),
    }
}

pub fn py_list_count<T: PartialEq>(v: &[T], needle: T) -> i64 {
    v.iter().filter(|x| **x == needle).count() as i64
}

pub fn py_sort<T: PartialOrd>(v: &mut [T]) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

pub fn py_dict_items<K: Clone, V: Clone>(d: &HashMap<K, V>) -> Vec<(K, V)> {
    d.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

// ---------------------------------------------------------------------------
// struct module emulation
// ---------------------------------------------------------------------------

pub fn py_pack_u32_be(v: i64) -> Vec<u8> {
    ((v as u32).to_be_bytes()).to_vec()
}
pub fn py_pack_u16_be(v: i64) -> Vec<u8> {
    ((v as u16).to_be_bytes()).to_vec()
}
pub fn py_pack_i32_be(v: i64) -> Vec<u8> {
    ((v as i32).to_be_bytes()).to_vec()
}
pub fn py_pack_i16_be(v: i64) -> Vec<u8> {
    ((v as i16).to_be_bytes()).to_vec()
}
pub fn py_pack_u8(v: i64) -> Vec<u8> {
    vec![(v as u8)]
}
pub fn py_pack_i8(v: i64) -> Vec<u8> {
    vec![(v as i8 as u8)]
}

pub fn py_unpack_u32_be(v: &[u8]) -> (i64,) {
    assert!(v.len() >= 4, "struct.error: unpack requires a buffer of 4 bytes");
    ((u32::from_be_bytes([v[0], v[1], v[2], v[3]])) as i64,)
}
pub fn py_unpack_u16_be(v: &[u8]) -> (i64,) {
    assert!(v.len() >= 2, "struct.error: unpack requires a buffer of 2 bytes");
    ((u16::from_be_bytes([v[0], v[1]])) as i64,)
}
pub fn py_unpack_i32_be(v: &[u8]) -> (i64,) {
    assert!(v.len() >= 4, "struct.error: unpack requires a buffer of 4 bytes");
    ((i32::from_be_bytes([v[0], v[1], v[2], v[3]])) as i64,)
}
pub fn py_unpack_i16_be(v: &[u8]) -> (i64,) {
    assert!(v.len() >= 2, "struct.error: unpack requires a buffer of 2 bytes");
    ((i16::from_be_bytes([v[0], v[1]])) as i64,)
}
pub fn py_unpack_u8(v: &[u8]) -> (i64,) {
    assert!(v.len() >= 1, "struct.error: unpack requires a buffer of 1 bytes");
    ((v[0]) as i64,)
}
pub fn py_unpack_i8(v: &[u8]) -> (i64,) {
    assert!(v.len() >= 1, "struct.error: unpack requires a buffer of 1 bytes");
    ((v[0] as i8) as i64,)
}

// ---------------------------------------------------------------------------
// Bytes / strings
// ---------------------------------------------------------------------------

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn py_bytes_hex(v: &[u8]) -> String {
    let mut s = String::with_capacity(v.len() * 2);
    for b in v {
        s.push(HEX_DIGITS[(b >> 4) as usize] as char);
        s.push(HEX_DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

pub fn py_bytes_hex_sep(v: &[u8], sep: &str) -> String {
    if v.is_empty() {
        return String::new();
    }
    let mut s = String::with_capacity(v.len() * (2 + sep.len()));
    for (i, b) in v.iter().enumerate() {
        if i > 0 {
            s.push_str(sep);
        }
        s.push(HEX_DIGITS[(b >> 4) as usize] as char);
        s.push(HEX_DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

pub fn py_bytes_decode(v: &[u8], encoding: &str, errors: &str) -> String {
    match (encoding, errors) {
        ("ascii", "replace") => v
            .iter()
            .map(|b| {
                if *b < 128 {
                    (*b as char).to_string()
                } else {
                    "\u{fffd}".to_string()
                }
            })
            .collect(),
        ("ascii", _) => {
            let mut s = String::with_capacity(v.len());
            for b in v {
                assert!(*b < 128, "UnicodeDecodeError: 'ascii' codec can't decode byte");
                s.push(*b as char);
            }
            s
        }
        (_, "replace") => String::from_utf8_lossy(v).to_string(),
        _ => String::from_utf8(v.to_vec()).expect("UnicodeDecodeError: invalid utf-8"),
    }
}

pub fn py_str_opt<T: std::fmt::Display>(v: &Option<T>) -> String {
    match v {
        Some(x) => x.to_string(),
        None => "None".to_string(),
    }
}

pub fn py_str_split(v: &str, sep: &str) -> Vec<String> {
    if sep.is_empty() {
        return v.chars().map(|c| c.to_string()).collect();
    }
    v.split(sep).map(|s| s.to_string()).collect()
}

pub fn py_str_rsplit(v: &str, sep: &str) -> Vec<String> {
    py_str_split(v, sep)
}

pub fn py_str_splitlines(v: &str) -> Vec<String> {
    v.lines().map(|s| s.to_string()).collect()
}

pub fn py_str_join(parts: &[String], sep: &str) -> String {
    parts.join(sep)
}

pub fn py_str_find(v: &str, needle: &str) -> i64 {
    match v.find(needle) {
        Some(i) => v[..i].chars().count() as i64,
        None => -1,
    }
}

pub fn py_str_count(v: &str, needle: &str) -> i64 {
    if needle.is_empty() {
        return v.chars().count() as i64 + 1;
    }
    v.matches(needle).count() as i64
}

pub trait PyDisplay {
    fn py_display(&self) -> String;
}
impl PyDisplay for i64 {
    fn py_display(&self) -> String {
        self.to_string()
    }
}
impl PyDisplay for f64 {
    fn py_display(&self) -> String {
        self.to_string()
    }
}
impl PyDisplay for bool {
    fn py_display(&self) -> String {
        if *self { "True".into() } else { "False".into() }
    }
}
impl PyDisplay for String {
    fn py_display(&self) -> String {
        self.clone()
    }
}
impl PyDisplay for &str {
    fn py_display(&self) -> String {
        self.to_string()
    }
}
impl PyDisplay for Vec<u8> {
    fn py_display(&self) -> String {
        String::from_utf8_lossy(self).to_string()
    }
}
impl PyDisplay for Vec<i64> {
    fn py_display(&self) -> String {
        py_repr_list(self)
    }
}
impl PyDisplay for Vec<f64> {
    fn py_display(&self) -> String {
        py_repr_list(self)
    }
}
impl PyDisplay for Vec<String> {
    fn py_display(&self) -> String {
        py_repr_list(self)
    }
}
impl PyDisplay for PyValue {
    fn py_display(&self) -> String {
        self.to_string()
    }
}

fn py_repr_list<T: PyDisplay>(v: &[T]) -> String {
    let parts: Vec<String> = v.iter().map(|x| x.py_display()).collect();
    format!("[{}]", parts.join(", "))
}

/// Minimal `str.format` supporting positional `{}` placeholders.
pub fn py_str_format<T: PyDisplay>(fmt: &str, args: &[T]) -> String {
    let mut out = String::new();
    let mut rest = fmt;
    let mut i = 0usize;
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if i < args.len() {
            out.push_str(&args[i].py_display());
            i += 1;
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

pub fn py_hex(v: i64) -> String {
    format!("{:x}", v)
}
pub fn py_bin(v: i64) -> String {
    format!("{:b}", v)
}
pub fn py_oct(v: i64) -> String {
    format!("{:o}", v)
}
pub fn py_ord(c: &str) -> i64 {
    c.chars().next().map(|ch| ch as i64).unwrap_or(0)
}
pub fn py_chr(v: i64) -> String {
    char::from_u32(v as u32).map(|c| c.to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/// Current UTC time as an ISO-8601 string (millisecond precision).
pub fn py_now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        millis
    )
}

/// Convert days since the Unix epoch to a (year, month, day) triple.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Dynamic value (fallback for untyped Python values)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum PyValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<PyValue>),
    Dict(Vec<(PyValue, PyValue)>),
    Tuple(Vec<PyValue>),
}

impl Default for PyValue {
    fn default() -> Self {
        PyValue::None
    }
}

impl std::fmt::Display for PyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PyValue::None => write!(f, "None"),
            PyValue::Bool(b) => write!(f, "{}", if *b { "True" } else { "False" }),
            PyValue::Int(i) => write!(f, "{}", i),
            PyValue::Float(x) => write!(f, "{}", x),
            PyValue::Str(s) => write!(f, "{}", s),
            PyValue::Bytes(b) => write!(f, "{}", py_bytes_hex(b)),
            PyValue::List(v) => write!(f, "{}", py_repr_list(v)),
            PyValue::Tuple(v) => write!(f, "({})", py_repr_list(v)),
            PyValue::Dict(v) => {
                let parts: Vec<String> = v
                    .iter()
                    .map(|(k, val)| format!("{}: {}", k, val))
                    .collect();
                write!(f, "{{{}}}", parts.join(", "))
            }
        }
    }
}

impl From<i64> for PyValue {
    fn from(v: i64) -> Self {
        PyValue::Int(v)
    }
}
impl From<f64> for PyValue {
    fn from(v: f64) -> Self {
        PyValue::Float(v)
    }
}
impl From<bool> for PyValue {
    fn from(v: bool) -> Self {
        PyValue::Bool(v)
    }
}
impl From<String> for PyValue {
    fn from(v: String) -> Self {
        PyValue::Str(v)
    }
}
impl From<Vec<u8>> for PyValue {
    fn from(v: Vec<u8>) -> Self {
        PyValue::Bytes(v)
    }
}
impl From<Vec<i64>> for PyValue {
    fn from(v: Vec<i64>) -> Self {
        PyValue::List(v.into_iter().map(PyValue::Int).collect())
    }
}
impl From<Vec<f64>> for PyValue {
    fn from(v: Vec<f64>) -> Self {
        PyValue::List(v.into_iter().map(PyValue::Float).collect())
    }
}
impl From<Vec<String>> for PyValue {
    fn from(v: Vec<String>) -> Self {
        PyValue::List(v.into_iter().map(PyValue::Str).collect())
    }
}
impl<T: Into<PyValue>> From<Option<T>> for PyValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(x) => x.into(),
            None => PyValue::None,
        }
    }
}

pub fn py_repr<T: std::fmt::Debug>(v: &T) -> String {
    format!("{:?}", v)
}

// ---------------------------------------------------------------------------
// Python format specifications
// ---------------------------------------------------------------------------

/// Apply a Python format specification to a value.
///
/// Supports the subset used by protocol code: `[[fill]align][sign][#][0][width]
/// [,][.precision][type]` with the `d`, `b`, `o`, `x`, `X`, `f`, `e`, `s` and
/// `g` type characters.
pub fn py_format(value: PyValue, spec: &str) -> String {
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0usize;

    let mut fill = ' ';
    let mut align: Option<char> = None;
    if chars.len() >= 2 && matches!(chars[1], '<' | '>' | '^') {
        fill = chars[0];
        align = Some(chars[1]);
        i = 2;
    } else if !chars.is_empty() && matches!(chars[0], '<' | '>' | '^') {
        align = Some(chars[0]);
        i = 1;
    }
    let mut sign = ' ';
    if i < chars.len() && matches!(chars[i], '+' | '-' | ' ') {
        sign = chars[i];
        i += 1;
    }
    if i < chars.len() && chars[i] == '#' {
        i += 1;
    }
    let mut zero_pad = false;
    if i < chars.len() && chars[i] == '0' {
        zero_pad = true;
        i += 1;
    }
    let mut width = 0usize;
    while i < chars.len() && chars[i].is_ascii_digit() {
        width = width * 10 + chars[i].to_digit(10).unwrap() as usize;
        i += 1;
    }
    let mut precision: Option<usize> = None;
    if i < chars.len() && chars[i] == '.' {
        i += 1;
        let mut p = 0usize;
        while i < chars.len() && chars[i].is_ascii_digit() {
            p = p * 10 + chars[i].to_digit(10).unwrap() as usize;
            i += 1;
        }
        precision = Some(p);
    }
    let ty = if i < chars.len() { chars[i] } else { '\0' };

    let mut body = match &value {
        PyValue::Int(n) => match ty {
            'x' => format!("{:x}", n),
            'X' => format!("{:X}", n),
            'o' => format!("{:o}", n),
            'b' => format!("{:b}", n),
            'c' => char::from_u32(*n as u32)
                .map(|c| c.to_string())
                .unwrap_or_default(),
            _ => {
                let mut s = n.to_string();
                if sign == '+' && *n >= 0 {
                    s.insert(0, '+');
                }
                s
            }
        },
        PyValue::Float(x) => match ty {
            'e' => format!("{:.*e}", precision.unwrap_or(6), x),
            'g' => format!("{}", x),
            _ => format!("{:.*}", precision.unwrap_or(6), x),
        },
        PyValue::Str(s) => match precision {
            Some(p) => s.chars().take(p).collect(),
            None => s.clone(),
        },
        PyValue::Bool(b) => (if *b { "True" } else { "False" }).to_string(),
        other => other.to_string(),
    };

    if body.chars().count() < width {
        let pad = width - body.chars().count();
        if zero_pad && align.is_none() {
            let (prefix, rest) = match body.strip_prefix('-') {
                Some(r) => ("-".to_string(), r.to_string()),
                None => (String::new(), body.clone()),
            };
            body = format!("{}{}{}", prefix, "0".repeat(pad), rest);
        } else {
            let align = align.unwrap_or('>');
            let filler: String = std::iter::repeat(fill).take(pad).collect();
            body = match align {
                '<' => format!("{}{}", body, filler),
                '^' => {
                    let left = pad / 2;
                    let right = pad - left;
                    format!(
                        "{}{}{}",
                        std::iter::repeat(fill).take(left).collect::<String>(),
                        body,
                        std::iter::repeat(fill).take(right).collect::<String>()
                    )
                }
                _ => format!("{}{}", filler, body),
            };
        }
    }
    body
}
"##;
