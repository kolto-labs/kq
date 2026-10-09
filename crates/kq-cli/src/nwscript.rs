//! Engine function names, read at runtime from the install's own `nwscript.nss`.
//!
//! kq does not ship BioWare's script. The game's copy is the only source, so
//! an ACTION routine id is named only when the install being queried has one.

use regex::Regex;
use serde_json::Value as J;

use kq_format::ResType;
use kq_index::Index;

use crate::read;

/// Prototype names in declaration order. The position is the ACTION routine id.
pub fn parse_action_names(src: &str) -> Vec<String> {
    let block = Regex::new(r"(?s)/\*.*?\*/").unwrap();
    let line = Regex::new(r"//[^\n]*").unwrap();
    let proto = Regex::new(
        r"^\s*(?:void|int|float|string|object|effect|event|location|talent|vector|action|itemproperty)\s+(\w+)\s*\(",
    )
    .unwrap();
    let code = line
        .replace_all(&block.replace_all(src, ""), "")
        .into_owned();
    code.split(';')
        .filter_map(|stmt| proto.captures(stmt).map(|c| c[1].to_string()))
        .collect()
}

/// Names from the `nwscript.nss` the engine would load, or `None` if the
/// install has none.
pub fn action_names(index: &Index) -> Option<Vec<String>> {
    let nss = ResType::from_extension("nss")?;
    let r = index.resolve("nwscript", Some(nss))?;
    let bytes = read::read(index, r).ok()?;
    let names = parse_action_names(&String::from_utf8_lossy(&bytes));
    (!names.is_empty()).then_some(names)
}

/// Signatures from the install's `nwscript.nss`. Empty when it has none, so
/// ACTION calls then stay disassembly comments.
pub fn action_table(index: &Index) -> kq_ncs::ActionTable {
    let Some(nss) = ResType::from_extension("nss") else {
        return kq_ncs::ActionTable::empty();
    };
    let Some(r) = index.resolve("nwscript", Some(nss)) else {
        return kq_ncs::ActionTable::empty();
    };
    match read::read(index, r) {
        Ok(bytes) => kq_ncs::ActionTable::from_nwscript(&String::from_utf8_lossy(&bytes)),
        Err(_) => kq_ncs::ActionTable::empty(),
    }
}

/// Put the install's function names on the `routine` entries of a decoded NCS.
pub fn name_routines(index: &Index, ncs_json: &mut J) {
    let Some(list) = ncs_json.get_mut("instructions").and_then(J::as_array_mut) else {
        return;
    };
    if !list.iter().any(|ins| ins.get("routine").is_some()) {
        return;
    }
    let Some(names) = action_names(index) else {
        return;
    };
    for ins in list {
        let Some(id) = ins.get("routine").and_then(J::as_u64) else {
            continue;
        };
        if let (Some(name), Some(obj)) = (names.get(id as usize), ins.as_object_mut()) {
            obj.insert("name".into(), J::String(name.clone()));
        }
    }
}
