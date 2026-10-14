//! Shared JSON envelope for decoded resources (`cat`).

use serde::Serialize;
use serde_json::{json, Map, Value as J};

use kq_format::tlk::Tlk;
use kq_format::ResType;
use kq_index::{Index, Resource};

use crate::live;
use crate::render::Decoded;

#[derive(Serialize)]
pub struct ResourceJson<'a> {
    pub name: String,
    pub resref: &'a str,
    #[serde(rename = "type")]
    pub restype: String,
    pub path: String,
    pub source: &'static str,
    pub container: &'a str,
    pub module: Option<&'a str>,
    pub file: String,
    pub offset: u64,
    pub size: u64,
    pub content: J,
}

pub fn decoded_to_json(decoded: &Decoded) -> J {
    match decoded {
        Decoded::Value(v) => v.clone(),
        Decoded::Text(t) => J::String(t.clone()),
        Decoded::Opaque { kind, len } => {
            serde_json::json!({ "kind": kind, "bytes": len, "decoded": false })
        }
    }
}

pub fn build_resource_json<'a>(
    index: &'a Index,
    r: &'a Resource,
    decoded: &Decoded,
) -> ResourceJson<'a> {
    let source = index.source(r);
    ResourceJson {
        name: r.filename(),
        resref: &r.resref,
        restype: r.restype.to_string(),
        path: index.virt_path(r),
        source: source.kind.as_str(),
        container: &source.label,
        module: source.module_root.as_deref(),
        file: index.rel_file(r),
        offset: r.offset,
        size: r.size,
        content: decoded_to_json(decoded),
    }
}

/// Resolve talk-table indices in decoded JSON against `dialog.tlk`.
///
/// * GFF `LocString` / `StrRef` objects (`{ "strref": N }`) gain a `text` field.
/// * 2DA cells in strref columns (Description, Name, …) become
///   `{ "strref": N, "text": "…" }` — same shape as GFF, so `jq` is uniform.
pub fn resolve_strrefs(content: &mut J, tlk: &Tlk, restype: ResType) {
    let is_2da = restype.extension() == Some("2da");
    resolve_value(content, tlk, is_2da, None);
}

fn resolve_value(v: &mut J, tlk: &Tlk, in_2da: bool, parent_key: Option<&str>) {
    match v {
        J::Object(map) => {
            // Already a strref object (GFF LocString / StrRef, or prior enrich).
            if let Some(n) = strref_from_object(map) {
                if let Some(entry) = tlk.get(n) {
                    map.entry("text".to_string())
                        .or_insert_with(|| json!(entry.text));
                }
            }
            // Walk children; track key for 2DA cell rewrite.
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if let Some(child) = map.get_mut(&k) {
                    resolve_value(child, tlk, in_2da, Some(&k));
                }
            }
        }
        J::Array(items) => {
            for item in items {
                resolve_value(item, tlk, in_2da, parent_key);
            }
        }
        J::String(s) if in_2da => {
            if let Some(col) = parent_key {
                if live::twoda_strref_column(col) {
                    if let Ok(n) = s.trim().parse::<i64>() {
                        if n >= 0 {
                            let text = tlk.get(n).map(|e| e.text.as_str()).unwrap_or("");
                            *v = json!({ "strref": n, "text": text });
                        }
                    }
                }
            }
        }
        J::Number(num) if in_2da => {
            if let Some(col) = parent_key {
                if live::twoda_strref_column(col) {
                    if let Some(n) = num.as_i64() {
                        if n >= 0 {
                            let text = tlk.get(n).map(|e| e.text.as_str()).unwrap_or("");
                            *v = json!({ "strref": n, "text": text });
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn strref_from_object(map: &Map<String, J>) -> Option<i64> {
    let v = map.get("strref")?;
    match v {
        J::Number(n) => n.as_i64(),
        J::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kq_format::tlk::{Entry, Tlk};

    fn tiny_tlk() -> Tlk {
        Tlk {
            language_id: 0,
            entries: vec![
                Entry {
                    text: "zero".into(),
                    sound: String::new(),
                },
                Entry {
                    text: "Sith Theme".into(),
                    sound: String::new(),
                },
            ],
        }
    }

    #[test]
    fn twoda_description_becomes_strref_object() {
        let tlk = tiny_tlk();
        let mut content = json!([
            { "_row": "1", "description": "1", "resource": "mus_theme_sith" }
        ]);
        let twoda = ResType::from_extension("2da").unwrap();
        resolve_strrefs(&mut content, &tlk, twoda);
        assert_eq!(content[0]["description"]["strref"], 1);
        assert_eq!(content[0]["description"]["text"], "Sith Theme");
        assert_eq!(content[0]["resource"], "mus_theme_sith");
    }

    #[test]
    fn gff_locstring_gains_text() {
        let tlk = tiny_tlk();
        let mut content = json!({ "LocalizedName": { "strref": 1 } });
        let uti = ResType::from_extension("uti").unwrap();
        resolve_strrefs(&mut content, &tlk, uti);
        assert_eq!(content["LocalizedName"]["text"], "Sith Theme");
        assert_eq!(content["LocalizedName"]["strref"], 1);
    }
}
