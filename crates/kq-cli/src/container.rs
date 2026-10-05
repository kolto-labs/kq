//! `--from NAME`: pick one container, then the resource inside it.
//!
//! `NAME` is a container label as `kq which` prints it: `data/templates.bif`,
//! `danm13.mod`, `Override`. A bare file name such as `templates.bif` also
//! works when exactly one container has that name. Every failure says what
//! to type instead.

use kq_format::ResType;
use kq_index::{Index, Resource};

/// Find `name` (of type `want`, when given) inside the container `from`.
///
/// `flag` is the option that named the container (`--from`, `--against`),
/// so the error can show the exact thing to type. The error is a complete
/// sentence for the user, without the `kq:` prefix.
pub fn find<'a>(
    index: &'a Index,
    name: &str,
    want: Option<ResType>,
    flag: &str,
    from: &str,
) -> Result<&'a Resource, String> {
    let label = label(index, name, want, flag, from)?;
    copies(index, name, want)
        .find(|r| index.source(r).label == label)
        .ok_or_else(|| {
            let what = display(name, want);
            match holders(index, name, want).as_slice() {
                [] => format!("no resource named {what} in {label}"),
                held => format!(
                    "no resource named {what} in {label}; it is in {}",
                    held.join(", ")
                ),
            }
        })
}

/// The one container label `from` names, matched without regard to case.
fn label<'a>(
    index: &'a Index,
    name: &str,
    want: Option<ResType>,
    flag: &str,
    from: &str,
) -> Result<&'a str, String> {
    let from = from.replace('\\', "/");
    let from = from.trim_end_matches('/');
    let mut labels: Vec<&str> = index.sources.iter().map(|s| s.label.as_str()).collect();
    labels.sort_unstable();
    labels.dedup();

    if let Some(exact) = labels.iter().find(|l| l.eq_ignore_ascii_case(from)) {
        return Ok(exact);
    }
    let by_file_name: Vec<&str> = labels
        .iter()
        .copied()
        .filter(|l| file_name(l).eq_ignore_ascii_case(from))
        .collect();
    match by_file_name.as_slice() {
        [one] => Ok(one),
        [] => {
            let what = display(name, want);
            Err(match holders(index, name, want).as_slice() {
                [] => format!(
                    "no container named {from}; `kq which <name>` shows each copy's container"
                ),
                [one] => format!("no container named {from}; {what} is in {one}: use {flag} {one}"),
                held => format!(
                    "no container named {from}; {what} is in {}: pass one of those to {flag}",
                    held.join(", ")
                ),
            })
        }
        many => Err(format!(
            "{flag} {from} matches {} containers ({}); pass the full label",
            many.len(),
            many.join(", ")
        )),
    }
}

/// Copies of `name`, in engine resolve order.
fn copies<'a>(
    index: &'a Index,
    name: &str,
    want: Option<ResType>,
) -> impl Iterator<Item = &'a Resource> {
    index
        .lookup(name)
        .iter()
        .map(|&i| &index.resources[i as usize])
        .filter(move |r| want.is_none_or(|t| r.restype == t))
}

/// Labels of the containers that hold a copy of `name`, in resolve order.
fn holders<'a>(index: &'a Index, name: &str, want: Option<ResType>) -> Vec<&'a str> {
    let mut labels: Vec<&str> = Vec::new();
    for r in copies(index, name, want) {
        let label = index.source(r).label.as_str();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

fn file_name(label: &str) -> &str {
    label.rsplit('/').next().unwrap_or(label)
}

fn display(name: &str, want: Option<ResType>) -> String {
    match want.and_then(|t| t.extension()) {
        Some(ext) => format!("{name}.{ext}"),
        None => name.to_string(),
    }
}
