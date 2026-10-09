//! Engine function names and signatures, read at runtime from a `nwscript.nss`
//! the user supplies.
//!
//! kq does not ship BioWare's script. It comes from `--nwscript FILE`, or
//! from the install named by `--install`, found the way the engine finds it:
//! `Override` first, then the game's archives.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value as J;

use kq_format::ResType;
use kq_index::Index;
use kq_ncs::ActionTable;

use crate::{read, Ctx};

const NEED: &str = "kq needs nwscript.nss for engine function names and does not ship it. \
Pass --nwscript <file>, or --install <path> to an install that has one";

/// The table for `restype`, or `None` when it is not an NCS and needs none.
pub fn for_type<'a>(
    ctx: &'a Ctx,
    index: Option<&Index>,
    restype: Option<ResType>,
) -> Result<Option<&'a ActionTable>> {
    if restype.and_then(|t| t.extension()) != Some("ncs") {
        return Ok(None);
    }
    table(ctx, index).map(Some)
}

/// Signatures from `--nwscript`, else from the install's own `nwscript.nss`.
/// Parsed once per run.
pub fn table<'a>(ctx: &'a Ctx, index: Option<&Index>) -> Result<&'a ActionTable> {
    ctx.actions
        .get_or_init(|| load(ctx, index).map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
}

fn load(ctx: &Ctx, index: Option<&Index>) -> Result<ActionTable> {
    if let Some(file) = &ctx.nwscript {
        let src = std::fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
        return parse(&src).with_context(|| file.display().to_string());
    }
    let opened;
    let index = match index {
        Some(i) => i,
        None => {
            opened = ctx.index().map_err(|_| anyhow!(NEED))?;
            &opened
        }
    };
    let nss = ResType::from_extension("nss").context("no nss resource type")?;
    let Some(r) = index.resolve("nwscript", Some(nss)) else {
        bail!(
            "{} has no nwscript.nss in Override or the game archives. \
             Pass --nwscript <file>",
            index.root.display()
        );
    };
    let src = read::read(index, r)?;
    parse(&src).with_context(|| format!("{}: nwscript.nss", index.root.display()))
}

fn parse(src: &[u8]) -> Result<ActionTable> {
    let table = ActionTable::from_nwscript(&String::from_utf8_lossy(src));
    if table.is_empty() {
        bail!("no engine function prototypes found; is this the game's nwscript.nss?");
    }
    Ok(table)
}

/// Put function names on the `routine` entries of a decoded NCS.
pub fn name_routines(table: &ActionTable, ncs_json: &mut J) {
    let Some(list) = ncs_json.get_mut("instructions").and_then(J::as_array_mut) else {
        return;
    };
    for ins in list {
        let Some(id) = ins.get("routine").and_then(J::as_u64) else {
            continue;
        };
        let name = u16::try_from(id).ok().and_then(|id| table.get(id));
        if let (Some(sig), Some(obj)) = (name, ins.as_object_mut()) {
            obj.insert("name".into(), J::String(sig.name.clone()));
        }
    }
}
