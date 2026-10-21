use crate::ty::Ty;

/// One engine-function parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamSig {
    pub ty: Ty,
    pub default: Option<String>,
}

/// One engine-function signature, indexed by its ACTION routine id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionSig {
    pub name: String,
    pub ret: Ty,
    pub params: Vec<ParamSig>,
}

/// Engine-function signatures supplied by the caller.
///
/// kotor-ncs ships no engine-function data. Build the table from the
/// `nwscript.nss` of the install being decompiled, or from your own list.
/// The position of a prototype is its ACTION routine id.
#[derive(Clone, Debug, Default)]
pub struct ActionTable {
    actions: Vec<Option<ActionSig>>,
}

impl ActionTable {
    /// A table with no functions. ACTION instructions fail to decompile.
    pub fn empty() -> Self {
        Self::default()
    }

    /// A table from signatures indexed by routine id; `None` marks a gap.
    pub fn new(actions: Vec<Option<ActionSig>>) -> Self {
        Self { actions }
    }

    /// Parse the function prototypes of an `nwscript.nss` source.
    ///
    /// Comments are skipped. Constants and anything that is not a
    /// function prototype are ignored.
    pub fn from_nwscript(src: &str) -> Self {
        let code = strip_comments(src);
        let actions = code
            .split(';')
            .filter_map(parse_prototype)
            .map(Some)
            .collect();
        Self { actions }
    }

    pub fn get(&self, id: u16) -> Option<&ActionSig> {
        self.actions.get(id as usize).and_then(|s| s.as_ref())
    }

    pub fn len(&self) -> usize {
        self.actions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        if in_str {
            let c = src[i..].chars().next().unwrap();
            out.push(c);
            if c == '\\' && i + 1 < b.len() {
                let n = src[i + 1..].chars().next().unwrap();
                out.push(n);
                i += 1 + n.len_utf8();
                continue;
            }
            if c == '"' {
                in_str = false;
            }
            i += c.len_utf8();
        } else if b[i] == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(' ');
        } else {
            let c = src[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn parse_ty(word: &str) -> Option<Ty> {
    Some(match word {
        "void" => Ty::Void,
        "int" => Ty::Int,
        "float" => Ty::Float,
        "string" => Ty::Str,
        "object" => Ty::Object,
        "effect" => Ty::Effect,
        "event" => Ty::Event,
        "location" => Ty::Location,
        "talent" => Ty::Talent,
        "vector" => Ty::Vector,
        "action" => Ty::Action,
        _ => return None,
    })
}

fn parse_prototype(stmt: &str) -> Option<ActionSig> {
    let stmt = stmt.trim();
    let open = stmt.find('(')?;
    let close = stmt.rfind(')')?;
    let mut head = stmt[..open].split_whitespace();
    let ret = parse_ty(head.next()?)?;
    let name = head.next()?;
    if head.next().is_some() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let mut params = Vec::new();
    for raw in split_params(&stmt[open + 1..close]) {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (decl, default) = match raw.split_once('=') {
            Some((d, v)) => (d, Some(v.trim().to_string())),
            None => (raw, None),
        };
        let ty = parse_ty(decl.split_whitespace().next()?)?;
        params.push(ParamSig { ty, default });
    }
    Some(ActionSig {
        name: name.to_string(),
        ret,
        params,
    })
}

/// Split on commas outside brackets and string literals.
fn split_params(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut in_str, mut start) = (Vec::new(), 0i32, false, 0);
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '[' | '(' if !in_str => depth += 1,
            ']' | ')' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}
