//! Live mention graph: catalog every ResRef, then keep only what the
//! engine can actually reach.
//!
//! Install mode seeds from hardcoded tables, talk files, scripts, starting
//! modules, and `StartingModule` in the ini — not from every folder under
//! `modules/` or `rims/`. A pair of templates that only name each other
//! stays dead. Talk-table rows count as used only when a *reachable*
//! GFF / 2DA / SSF cites them.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::Result;
use kq_format::gff::{Struct as GffStruct, Value as GffValue};
use kq_format::{gff, ncs, ssf, tlk, twoda, ResType};
use kq_index::{Index, RootKind, Source, SourceKind};

use crate::read;

/// Types the live walk does not follow (`--no-assets`). Textures and models
/// are opened from MDL/TXI; leftover still lists every ResRef+type the
/// engine *does* look up by name (scripts, dialog, 2DA, templates, VO).
pub const ASSET_EXTS: &[&str] = &[
    "tpc", "tga", "dds", "mdl", "mdx", "bmu", "mp3", "txi", "plt", "fxp",
];

/// Never loaded by the engine. Same ResRef as a compiled script does not
/// mean the source file is used.
const SOURCE_EXTS: &[&str] = &["nss"];

/// Area/module entry files. A module *folder* (`end_m01aa`) is not a ResRef;
/// the IFO is always `module.ifo` and the GIT/ARE are often `m01aa.*`.
/// Chitin `lyt`/`vis`/`pth` are reached from ARE via `add_are_layout_edges`, not as module entries.
#[allow(dead_code)]
const MODULE_ENTRY_EXTS: &[&str] = &["ifo", "are", "git", "lyt", "vis", "pth"];

/// Engine-opened talk files and the include the compiler always sees.
/// `nwscript` stays a seed name; `.nss` is not scanned so comments cannot seed the graph.
/// Always opened by the engine (talk table, journal GFF, nwscript include).
///
/// See `GetJournalQuestExperience` / journal GUI — `global.jrl` via `CResGFF`
/// (research-engine Q6 / journal handlers).
const ENGINE_ALWAYS: &[&str] = &["dialog", "dialogf", "nwscript", "global"];

/// 2DA filenames hardcoded as `C2DA` / `Load2DArrays_*` / side-loader
/// literals. See `CTwoDimArrays::Load2DArrays` @005c4c20 (K1) / @004805b0 (K2).
/// Nested tables named by those files (`classes.2da` AttackBonusTable, IPRP
/// `Name` / `TableResRef`) are reached by scanning those columns as type `2da`
/// via `CSWClass::Load*` / `CTwoDimArrays::LoadIPRPCostTables`.
///
/// Ship leftovers that look related but are **not** exe literals (do not seed):
/// `aliensound` (K2 loads `AlienVO`), `effecticons`/`dialogtokens`/`loadhints`/
/// `xpbaseconst`/`tutorial_old` (renames of seeded `effecticon`/`stringtokens`/
/// `loadscreenhints`/`xptable`/`tutorial`), `cls_atk_3` (classes only refs
/// `cls_atk_1`/`cls_atk_2`), plus NWN/editor orphans (`actions`, `metamagic`,
/// `polymorph`, `gamespyrooms`, `rrf_*`, EAX `sound*`, …) — confirmed absent
/// as token boundaries in `swkotor.exe` / `swkotor2.exe` string tables.
const ENGINE_2DAS: &[&str] = &[
    "acbonus",
    "aiscripts",
    "ambientmusic",
    "ambientsound",
    "ammunitiontypes",
    "animations",
    "appearance",
    "appearancesndset",
    "baseitems",
    "bindablekeys",
    "bodybag",
    "camerastyle",
    "categories",
    "classes",
    "classpowergain",
    "cls_spgn_jedi",
    "combatanimations",
    "comptypes",
    "creaturesize",
    "creaturespeed",
    "credits",
    "cursors",
    "damagehitvisual",
    "dialoganimations",
    "difficultyopt",
    "diffsettings",
    "disease",
    "doortypes",
    "droiddischarge",
    "effecticon",
    "encdifficulty",
    "excitedduration",
    "exptable",
    "feat",
    "featgain",
    "feedbacktext",
    "footstepsounds",
    "forceadjust",
    "forceshields",
    "formations",
    "fractionalcr",
    "gameeffects",
    "gamma",
    "gender",
    "genericdoors",
    "globalcat",
    "grenadesnd",
    "guisounds",
    "heads",
    "inventorysnds",
    "iprp_bonuscost",
    "iprp_costtable",
    "iprp_damagecost",
    "iprp_meleecost",
    "iprp_monstcost",
    "iprp_neg5cost",
    "iprp_onhit",
    "iprp_onhitdur",
    "iprp_paramtable",
    "iprp_srcost",
    "itempropdef",
    "itemprops",
    "itemvalue",
    "keymap",
    "lightcolor",
    "loadscreenhints",
    "loadscreens",
    "masterfeats",
    "modulesave",
    "movies",
    "namefilter",
    "names",
    "npc",
    "pazaakdecks",
    "phenotype",
    "placeableobjsnds",
    "placeables",
    "planetary",
    "plot",
    "poison",
    "portraits",
    "prioritygroups",
    "racialtypes",
    "ranges",
    "regeneration",
    "removefxondeath",
    "repadjust",
    "repute",
    "skills",
    "soundprovider",
    "soundset",
    "soundsettype",
    "spells",
    "statescripts",
    "stringtokens",
    "subrace",
    "surfacemat",
    "texpacks",
    "traps",
    "tutorial",
    "upcrystals",
    "upgrade",
    "vfx_persistent",
    "videoeffects",
    "visualeffects",
    "waypoint",
    "weapondischarge",
    "weaponsounds",
    "xptable",
];

/// Extra `C2DA` literals only in `swkotor2.exe` (`Load2DArrays_AlienVO` etc.).
/// Seeded only when `index.game == Game::K2`.
const ENGINE_2DAS_K2: &[&str] = &[
    "alienvo",
    "autobalance",
    "chemicalcreate",
    "emotion",
    "influence",
    "iprp_pc",
    "itemcreate",
    "loadstoryhints",
    "musictable",
    "swoopupgrade",
    "tutchemcreate",
];

/// GUI panel layouts passed to `CSWGuiPanel::StartLoadFromLayout` / `LoadFromLayout`.
/// Chargen / sub-panel names (`MAINCG`, `ABCHRGEN`, `ScriptSelect`, …) and the
/// resolution tooltip table (`tooltip6X4`…`tooltip16X12`) are K1 exe string
/// literals next to the same LoadFromLayout call sites as `classsel` /
/// `tooltip12x10`.
const ENGINE_GUIS: &[&str] = &[
    "abilities",
    "abchrgen",
    "areatransition",
    "barkbubble",
    "blackdot",
    "character",
    "classsel",
    "computer",
    "computercamera",
    "confirm",
    "container",
    "credits",
    "custpnl",
    "debug",
    "dialog",
    "equip",
    "fade",
    "ftchrgen",
    "galaxymap",
    "inventory",
    "journal",
    "leveluppnl",
    "load",
    "loadscreen",
    "maincg",
    "mainmenu",
    "map",
    "messages",
    "mipc210x7",
    "mipc212x10",
    "mipc212x9",
    "mipc216x12",
    "mipc28x6",
    "optautopause",
    "optfeedback",
    "optgameplay",
    "optgraphics",
    "optgraphicsadv",
    "optkeymapping",
    "optionsingame",
    "optionsmain",
    "optmouse",
    "optresolution",
    "optsound",
    "optsoundadv",
    "partyselection",
    "pause",
    "pazaakgame",
    "pazaaksetup",
    "pazaakwager",
    "portcust",
    "pwrlvlup",
    "qorcpnl",
    "questitem",
    "quickpnl",
    "saveload",
    "savename",
    "scriptselect",
    "skchrgen",
    "skillinfo",
    "statussummary",
    "store",
    "titlemovie",
    "tooltip",
    "tooltip10x8",
    "tooltip12x10",
    "tooltip12x9",
    "tooltip16x12",
    "tooltip6x4",
    "tooltip8x6",
    "top",
    "upgrade",
    "upgradeitems",
    "upgradesel",
];

/// Module roots from `CSWGuiMainMenu::OnNewGamePicked` @0067afb0 (`END_M01AA`)
/// and `CServerInfo::SetModuleName` @0054eaa0 (`ebo_m12aa`, `ebo_m40ad`, `tar_m02af`).
const K1_MODULES: &[&str] = &["end_m01aa", "ebo_m12aa", "ebo_m40ad", "tar_m02af"];

/// Default-script / GUI names hardcoded in K1 `swkotor.exe`.
/// See `CSWSCreature::EventHandler` @004fece0 (`k_hen_dialogue01`),
/// `CSWSDoor::EventHandler` / `CSWSTrigger::EventHandler` (`k_trg_transfail`,
/// `NW_G0_Transition`), `CAvoidCreature::PlotPathAroundCreature` (`k_def_pathfail01`),
/// `CSWParty::*`, `CSWGuiDialogComputer::UpdateSkills`, etc.
const K1_SCRIPTS: &[&str] = &[
    "k_computer_spike",
    "k_def_blocked01",
    "k_def_damage01",
    "k_def_pathfail01",
    "k_def_spellat01",
    "k_def_userdef01",
    "k_hen_attacked01",
    "k_hen_combend01",
    "k_hen_dialogue01",
    "k_hen_enter5m",
    "k_hen_exit5m",
    "k_hen_heartbt01",
    "k_hen_leadchng",
    "k_hen_percept01",
    "k_hen_retreat",
    "k_hen_spawn01",
    "k_pend_screenchg",
    "k_repair_part",
    "k_sup_galaxymap",
    "k_sup_gohawk",
    "k_sup_guiopen",
    "k_sup_solo",
    "k_trg_transfail",
    "nw_g0_transition",
];

/// UTI templates constructed in the exe.
/// See `CSWSCreature::TransferGold` @004fd730 (`g_i_credits001`),
/// `CSWSCreature::ForceEquipClothing` @00501c40 (`g_a_clothes01`),
/// `CSWSCreature::AddPazaakCard` @004efb40, cheat `givemed` / `givecomspikes` etc.
const ENGINE_TEMPLATES: &[&str] = &[
    "g_a_clothes01",
    "g_i_credits001",
    "g_i_drdrepeqp002",
    "g_i_medeqpmnt02",
    "g_i_pazsidebd001",
    "g_i_progspike01",
    "g_i_secspike01",
];

/// TSL new-game start. See `CSWGuiMainMenu::OnNewGamePicked` @0053ccd0 (`001ebo`).
const K2_MODULES: &[&str] = &["001ebo"];

/// How a decoded resource contributes StrRefs.
#[derive(Clone, Copy)]
enum StrRefMode {
    None,
    /// GFF `CExoLocString` / `CResRef` wrappers: only the `strref` key.
    Gff,
    /// 2DA columns that Holocron-style "find references" treats as talk ids.
    TwoDa,
    /// Every cell in an SSF is a StrRef.
    Ssf,
}

#[derive(Clone, Debug)]
pub struct TlkRow {
    pub strref: i64,
    pub text: String,
    pub sound: String,
}

/// Catalog + mention edges + reachability from engine (or capsule) seeds.
pub struct LiveGraph {
    pub catalog: HashSet<String>,
    pub seeds: Vec<String>,
    pub seed_ids: Vec<u32>,
    #[allow(dead_code)]
    pub reachable: HashSet<String>,
    /// Resource indices the live walk actually entered.
    pub used_ids: HashSet<u32>,
    /// First parent seen during BFS (child → parent resource id).
    pub parent: HashMap<u32, u32>,
    /// ResRef / module-root tokens each used resource mentions.
    pub edges: HashMap<u32, HashSet<String>>,
    /// ResRef-shaped tokens with no in-scope / module-root match.
    pub missing: HashMap<u32, HashSet<String>>,
    /// Module-root → scoped entry resource ids (ifo/are/git/pth).
    pub module_entries: HashMap<String, Vec<u32>>,
    /// ResRefs of `used_ids`, plus VO names on used talk-table rows.
    #[allow(dead_code)]
    pub used: HashSet<String>,
    pub used_strrefs: HashSet<i64>,
    pub tlk: Vec<TlkRow>,
    pub scanned: usize,
}

impl LiveGraph {
    pub fn leftover_strings(&self) -> Vec<&TlkRow> {
        self.tlk
            .iter()
            .filter(|row| !self.used_strrefs.contains(&row.strref))
            .collect()
    }
}

pub fn build(index: &Index) -> Result<LiveGraph> {
    let catalog: HashSet<String> = index.resources.iter().map(|r| r.resref.clone()).collect();
    let module_roots: HashSet<String> = index
        .module_roots()
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let loaded_map = scoped_loaded(index);
    let loaded: Vec<u32> = {
        let mut v: Vec<u32> = loaded_map.values().copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let module_entries = module_entry_ids(index, &loaded_map);

    let mut scan_ids = loaded.clone();
    scan_ids.retain(|&i| is_scan_source(index.resources[i as usize].restype));

    crate::output::warn(format!(
        "catalog {} ResRefs; scanning {} for mentions…",
        catalog.len(),
        scan_ids.len()
    ));

    // baseitems.itemclass[BaseItem] → inventory icon `i%s_%03d` (see below).
    let itemclasses = load_baseitem_itemclasses(index, &loaded_map);

    // One file at a time, resources in offset order. Nested rayon over a USB
    // mmap random-faults the same BIF and turned a ~30s scan into minutes.
    let groups = group_scan_ids_by_file(index, &scan_ids);
    let mut hits = Vec::<(u32, HashSet<String>, HashSet<i64>, HashSet<String>)>::new();
    for (file_idx, ids) in &groups {
        let path = &index.files[*file_idx as usize];
        let Ok(map) = read::map_file(path) else {
            continue;
        };
        for &i in ids {
            let r = &index.resources[i as usize];
            let end = r.offset as usize + r.size as usize;
            if end > map.len() {
                continue;
            }
            let bytes = &map[r.offset as usize..end];
            let scope = resource_scope(index, r);
            let mut mentions = HashSet::new();
            let mut strrefs = HashSet::new();
            let mut missing = HashSet::new();
            scan_bytes(
                bytes,
                r,
                index,
                &loaded_map,
                &module_entries,
                &scope,
                &module_roots,
                &catalog,
                &itemclasses,
                &mut mentions,
                &mut strrefs,
                &mut missing,
            );
            if mentions.is_empty() && strrefs.is_empty() && missing.is_empty() {
                continue;
            }
            hits.push((i, mentions, strrefs, missing));
        }
    }

    let mut edges: HashMap<u32, HashSet<String>> = HashMap::new();
    let mut strrefs_by: HashMap<u32, HashSet<i64>> = HashMap::new();
    let mut missing_by: HashMap<u32, HashSet<String>> = HashMap::new();
    for (id, mentions, strrefs, missing) in hits {
        if !mentions.is_empty() {
            edges.insert(id, mentions);
        }
        if !strrefs.is_empty() {
            strrefs_by.insert(id, strrefs);
        }
        if !missing.is_empty() {
            missing_by.insert(id, missing);
        }
    }
    add_are_layout_edges(index, &loaded_map, &mut edges);
    add_module_load_screen_edges(index, &module_entries, &mut edges);
    add_resolution_gui_texture_edges(index, &loaded_map, &mut edges);

    let (seed_labels, seed_ids) = seed_ids(index, &catalog, &loaded_map, &module_entries);
    let (mut used_ids, parent) = bfs(index, &seed_ids, &edges, &loaded_map, &module_entries);

    let tlk = load_dialog_tlk(index)?;
    let tlk_len = tlk.len() as i64;
    let mut used_strrefs = HashSet::new();
    for &id in &used_ids {
        if let Some(refs) = strrefs_by.get(&id) {
            for &n in refs {
                if n >= 0 && n < tlk_len {
                    used_strrefs.insert(n);
                }
            }
        }
    }

    let mut used: HashSet<String> = used_ids
        .iter()
        .map(|&i| index.resources[i as usize].resref.clone())
        .collect();
    let reachable = used.clone();
    for row in &tlk {
        if used_strrefs.contains(&row.strref) && !row.sound.is_empty() {
            // See `CSWSDialog::SendDialogEntry` (stream WAV) and
            // `CSWCCreature::LipSync` → `CLIP::LoadLip` (type 3004).
            // K1 streams under STREAMWAVES; K2 under STREAMVOICE — both are
            // `.wav` + same-named `.lip` in the graph.
            for ext in ["wav", "lip"] {
                let tok = format!("{}.{ext}", row.sound);
                for id in resolve_mention(index, &loaded_map, &module_entries, &None, &tok) {
                    used_ids.insert(id);
                    used.insert(index.resources[id as usize].resref.clone());
                }
            }
        }
    }

    Ok(LiveGraph {
        catalog,
        seeds: seed_labels,
        seed_ids,
        reachable,
        used_ids,
        parent,
        edges,
        missing: missing_by,
        module_entries,
        used,
        used_strrefs,
        tlk,
        scanned: scan_ids.len(),
    })
}

/// Group scan ids by archive file, each group sorted by byte offset.
pub fn group_scan_ids_by_file(index: &Index, ids: &[u32]) -> Vec<(u32, Vec<u32>)> {
    let mut by_file: HashMap<u32, Vec<u32>> = HashMap::new();
    for &id in ids {
        by_file
            .entry(index.resources[id as usize].file)
            .or_default()
            .push(id);
    }
    let mut groups: Vec<(u32, Vec<u32>)> = by_file.into_iter().collect();
    groups.sort_by_key(|(f, _)| *f);
    for (_, ids) in &mut groups {
        ids.sort_by_key(|&i| index.resources[i as usize].offset);
    }
    groups
}

pub fn is_asset(t: ResType) -> bool {
    ASSET_EXTS.contains(&t.extension().unwrap_or(""))
}

/// Types omitted from leftover reports unless `-t` or `--assets` is given.
/// Only `.nss` — the engine loads compiled `.ncs`, not source.
pub fn is_noise(t: ResType) -> bool {
    SOURCE_EXTS.contains(&t.extension().unwrap_or(""))
}

/// `None` = global (override / chitin / etc.); `Some(module_root)` for module-local sources.
pub type Scope = Option<String>;

/// ModuleMod / ModuleRim → Some(module_root); everything else → None (global).
pub fn resource_scope(index: &Index, r: &kq_index::Resource) -> Scope {
    let source = index.source(r);
    // Prefer an explicit module_root (module capsules and lips/NAME_loc.mod —
    // engine `LIPS:NAME_loc` is registered with the module).
    source
        .module_root
        .as_ref()
        .map(|s| s.to_ascii_lowercase())
}

/// Lowest precedence id per (scope, resref, restype).
pub fn scoped_loaded(index: &Index) -> HashMap<(Scope, String, ResType), u32> {
    let mut best: HashMap<(Scope, String, ResType), (u32, u32)> = HashMap::new();
    for (i, r) in index.resources.iter().enumerate() {
        let scope = resource_scope(index, r);
        let key = (scope, r.resref.clone(), r.restype);
        let prec = index.sources[r.source as usize].precedence;
        best.entry(key)
            .and_modify(|(id, p)| {
                if prec < *p || (prec == *p && (i as u32) < *id) {
                    *id = i as u32;
                    *p = prec;
                }
            })
            .or_insert((i as u32, prec));
    }
    best.into_iter().map(|(k, (id, _))| (k, id)).collect()
}

/// Resource ids loaded in their own scope (module-local or global).
#[allow(dead_code)]
pub fn scoped_loaded_id_set(index: &Index) -> HashSet<u32> {
    scoped_loaded(index).into_values().collect()
}

/// Higher-precedence copy that hides `id`.
///
/// - Same scope: another id won [`scoped_loaded`] for `(scope, resref, type)`.
/// - Module scope: Override of the same `(resref, type)` still wins (R1).
/// - A module capsule does **not** hide a global chitin/lips-localization copy.
///   Resman only searches `MODULES:NAME` / `LIPS:NAME_loc` while that module is
///   loaded (`CSWSModule::AddModuleResources`). `k_hbas_check01` in
///   `unk_m44ac.mod` does not replace `scripts.bif` on the Ebon Hawk.
///
/// `loaded` must be the map from [`scoped_loaded`].
pub fn overshadowed_by(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    id: u32,
) -> Option<u32> {
    let r = &index.resources[id as usize];
    let scope = resource_scope(index, r);
    let loaded_id = loaded
        .get(&(scope.clone(), r.resref.clone(), r.restype))
        .copied()?;
    if loaded_id != id {
        return Some(loaded_id);
    }
        if scope.is_some() {
        if let Some(&ov) = loaded.get(&(None, r.resref.clone(), r.restype)) {
            if index.source(&index.resources[ov as usize]).kind == kq_index::SourceKind::Override {
                return Some(ov);
            }
        }
        return None;
    }
    None
}

pub fn is_overshadowed(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    id: u32,
) -> bool {
    overshadowed_by(index, loaded, id).is_some()
}

/// Every resource id that [`overshadowed_by`] would report. One pass.
pub fn overshadowed_id_set(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
) -> HashSet<u32> {
    (0..index.resources.len() as u32)
        .filter(|&id| is_overshadowed(index, loaded, id))
        .collect()
}

#[allow(dead_code)] // retained for Task 5 / global fallback tooling
fn all_loaded(index: &Index) -> Vec<u32> {
    let mut ids: Vec<u32> = (0..index.resources.len() as u32).collect();
    ids.sort_by(|&a, &b| {
        let (ra, rb) = (&index.resources[a as usize], &index.resources[b as usize]);
        ra.resref
            .cmp(&rb.resref)
            .then(ra.restype.cmp(&rb.restype))
            .then(
                index.sources[ra.source as usize]
                    .precedence
                    .cmp(&index.sources[rb.source as usize].precedence),
            )
    });
    ids.dedup_by(|&mut a, &mut b| {
        let (ra, rb) = (&index.resources[a as usize], &index.resources[b as usize]);
        ra.resref == rb.resref && ra.restype == rb.restype
    });
    ids
}

fn module_entry_ids(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
) -> HashMap<String, Vec<u32>> {
    let _ = index;
    // Every GFF in a reachable module is a scan source. The capsule is on
    // resman for the whole visit (`AddModuleResources`); CreateObject and
    // editor leftovers still carry Script*/On* on templates GIT never listed.
    // Seeding only IFO/ARE/GIT dropped those slots (R3). Packed NCS is not
    // seeded — RunScript still needs a GFF field or ExecuteScript. lyt/vis
    // stay on the ARE same-resref path.
    let mut map: HashMap<String, Vec<u32>> = HashMap::new();
    for ((scope, _resref, restype), &id) in loaded {
        let Some(root) = scope.as_deref() else {
            continue;
        };
        if !restype.is_gff() {
            continue;
        }
        map.entry(root.to_string()).or_default().push(id);
    }
    map
}

fn is_scan_source(t: ResType) -> bool {
    if t.is_gff() {
        return true;
    }
    match t.extension() {
        Some("2da" | "ncs" | "ssf" | "mdl") => true,
        Some("nss") => false,
        Some(_) if t.is_plain_text() => true,
        _ => false,
    }
}

fn strref_mode(t: ResType) -> StrRefMode {
    if t.is_gff() {
        return StrRefMode::Gff;
    }
    match t.extension() {
        Some("2da") => StrRefMode::TwoDa,
        Some("ssf") => StrRefMode::Ssf,
        _ => StrRefMode::None,
    }
}

fn seed_ids(
    index: &Index,
    known: &HashSet<String>,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
) -> (Vec<String>, Vec<u32>) {
    let mut labels = Vec::new();
    let mut ids = Vec::new();

    let push_resref = |name: &str, labels: &mut Vec<String>, ids: &mut Vec<u32>| {
        let name = name.to_ascii_lowercase();
        let list = resolve_in_scope(index, loaded, module_entries, &None, &name);
        if !list.is_empty() {
            labels.push(name);
            ids.extend(list);
        }
    };
    let push_module = |name: &str, labels: &mut Vec<String>, ids: &mut Vec<u32>| {
        let name = name.to_ascii_lowercase();
        if let Some(list) = module_entries.get(&name) {
            labels.push(name.clone());
            ids.extend(list.iter().copied());
            return;
        }
        let mut any = false;
        for (i, r) in index.resources.iter().enumerate() {
            if index
                .source(r)
                .module_root
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case(&name))
            {
                ids.push(i as u32);
                any = true;
            }
        }
        if any {
            labels.push(name);
        } else {
            crate::output::warn(format!("seed module `{name}` unresolved (no resources)"));
        }
    };

    match index.kind {
        RootKind::Install => {
            for name in ENGINE_ALWAYS
                .iter()
                .chain(ENGINE_2DAS)
                .chain(ENGINE_GUIS)
            {
                push_resref(name, &mut labels, &mut ids);
            }
            if matches!(index.game, kq_index::Game::K2) {
                for name in ENGINE_2DAS_K2 {
                    push_resref(name, &mut labels, &mut ids);
                }
            }
            for name in K1_SCRIPTS {
                push_resref(name, &mut labels, &mut ids);
            }
            for name in ENGINE_TEMPLATES {
                push_resref(name, &mut labels, &mut ids);
            }
            let extra: &[&str] = match index.game {
                kq_index::Game::K1 => K1_MODULES,
                kq_index::Game::K2 => K2_MODULES,
            };
            for name in extra {
                push_module(name, &mut labels, &mut ids);
            }
            for name in ini_starting_modules(&index.root) {
                push_module(&name, &mut labels, &mut ids);
                push_resref(&name, &mut labels, &mut ids);
            }
            // Warn once per missing hardcoded resref seed.
            let expected: Vec<&str> = ENGINE_ALWAYS
                .iter()
                .chain(ENGINE_2DAS.iter())
                .chain(ENGINE_GUIS.iter())
                .chain(K1_SCRIPTS.iter())
                .chain(ENGINE_TEMPLATES.iter())
                .copied()
                .collect();
            for name in expected {
                let key = name.to_ascii_lowercase();
                if !labels.iter().any(|l| l == &key) {
                    crate::output::warn(format!("seed `{key}` is not in this install"));
                }
            }
        }
        RootKind::Capsule | RootKind::Folder | RootKind::File => {
            for r in &index.resources {
                if matches!(r.restype.extension(), Some("ifo" | "are" | "git")) {
                    let list = resolve_in_scope(index, loaded, module_entries, &None, &r.resref);
                    if !list.is_empty() {
                        labels.push(r.resref.clone());
                        ids.extend(list);
                    }
                }
            }
            if let Some(root) = index
                .resources
                .first()
                .and_then(|r| index.source(r).module_root.as_deref())
            {
                push_module(root, &mut labels, &mut ids);
            }
            if ids.is_empty() {
                for name in known {
                    push_resref(name, &mut labels, &mut ids);
                }
            }
        }
    }

    labels.sort();
    labels.dedup();
    ids.sort_unstable();
    ids.dedup();
    (labels, ids)
}

fn ini_starting_modules(_root: &Path) -> Vec<String> {
    // See research-engine Q2: there is **no** `StartingModule` INI key in either
    // exe (`CSWGuiMainMenu::OnNewGamePicked` hardcodes END_M01AA / 001ebo).
    // Kept as a no-op so old INI fiction cannot seed the graph.
    Vec::new()
}

fn load_dialog_tlk(index: &Index) -> Result<Vec<TlkRow>> {
    let Some(tlk_ty) = ResType::from_extension("tlk") else {
        return Ok(Vec::new());
    };
    let Some(r) = index.resolve("dialog", Some(tlk_ty)) else {
        return Ok(Vec::new());
    };
    let bytes = read::read(index, r)?;
    let table = tlk::read(&bytes, Path::new("dialog.tlk"))?;
    Ok(table
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| TlkRow {
            strref: i as i64,
            text: e.text.clone(),
            sound: e.sound.to_ascii_lowercase(),
        })
        .collect())
}

fn resolve_in_scope(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    tok: &str,
) -> Vec<u32> {
    let mut out = Vec::new();
    // Collect distinct restypes for this resref from the index.
    let mut types: Vec<ResType> = index
        .lookup(tok)
        .iter()
        .map(|&i| index.resources[i as usize].restype)
        .collect();
    types.sort_by_key(|t| t.0);
    types.dedup();

    for ty in &types {
        if let Some(&id) = loaded.get(&(None, tok.to_string(), *ty)) {
            let kind = index.source(&index.resources[id as usize]).kind;
            if kind == kq_index::SourceKind::Override {
                out.push(id);
                continue;
            }
        }
        if let Some(m) = scope {
            if let Some(&id) = loaded.get(&(Some(m.clone()), tok.to_string(), *ty)) {
                out.push(id);
                continue;
            }
        }
        if let Some(&id) = loaded.get(&(None, tok.to_string(), *ty)) {
            out.push(id);
        }
    }
    if let Some(ids) = module_entries.get(tok) {
        out.extend(ids.iter().copied());
    }
    // Override/chitin DLG `Script`/`Active` run through resman for the
    // *current* module. A ResRef that exists only in e.g. `ebo_m12aa.mod`
    // is still live when that module is loaded; global resolve alone misses it.
    // Override/chitin DLG `Script`/`Active` (global scope) run through
    // resman for whatever module is current. Module-scoped callers still
    // only see Override / this module / chitin — not a sibling module.
    if out.is_empty() && !types.is_empty() && scope.is_none() {
        for ty in &types {
            for ((sc, resref, rty), &id) in loaded {
                if sc.is_some() && resref == tok && rty == ty {
                    out.push(id);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    // `VO_ResRef` / TLK sound is one string; `.wav` is global stream and
    // `.lip` lives in `lips/NAME_loc.mod` (module-scoped). Finding the wav
    // must not skip the lip.
    if let Some(lip) = ResType::from_extension("lip") {
        if types.contains(&lip)
            && !out
                .iter()
                .any(|&i| index.resources[i as usize].restype == lip)
        {
            for ((_, resref, rty), &id) in loaded {
                if resref == tok && *rty == lip {
                    out.push(id);
                }
            }
        }
    }
    if let Some(wav) = ResType::from_extension("wav") {
        if types.contains(&wav)
            && !out
                .iter()
                .any(|&i| index.resources[i as usize].restype == wav)
        {
            for ((_, resref, rty), &id) in loaded {
                if resref == tok && *rty == wav {
                    out.push(id);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Split a graph token into ResRef base and optional restype.
///
/// See `CExoResMan` / `CResHelper<CRes*,TYPE>::SetResRef` — the engine always
/// looks up a name **and** a resource type id (`CreateResourceExtensionTable`
/// K1 @005e6d20). Untyped tokens are module roots only.
fn split_mention(tok: &str) -> (&str, Option<ResType>) {
    if let Some((base, ext)) = tok.rsplit_once('.') {
        if let Some(ty) = ResType::from_extension(ext) {
            return (base, Some(ty));
        }
    }
    (tok, None)
}

/// Resolve a mention the way resman would for the caller's module scope.
///
/// See `CSWSModule::AddModuleResources` @004c4150 (K1) / @00732120 (K2) for
/// which tables are registered while a module is loaded; typed tokens filter
/// by restype, bare names only match module roots (`ExecuteCommandStartNewModule`
/// @00544390 / `LinkedToModule` on doors via `CSWSDoor::LoadDoor` @0058a1f0).
fn resolve_mention(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    tok: &str,
) -> Vec<u32> {
    let (name, ty) = split_mention(tok);
    if let Some(ty) = ty {
        let mut ids = resolve_in_scope(index, loaded, module_entries, scope, name);
        ids.retain(|&i| index.resources[i as usize].restype == ty);
        ids
    } else if let Some(ids) = module_entries.get(name) {
        ids.clone()
    } else {
        Vec::new()
    }
}

/// Texture open path: TPC, TGA fallback, and same-ResRef TXI.
///
/// See Aurora model/GUI texture load; PyKotor `txi` docs — a TXI is either
/// embedded in a TPC footer or a separate same-ResRef `.txi` next to the
/// texture (`Libraries/bioware-kaitai-formats/.../txi.py`).
fn mention_texture(out: &mut HashSet<String>, name: &str) {
    mention_typed(out, name, "tpc");
    mention_typed(out, name, "tga");
    mention_typed(out, name, "txi");
}

/// Record a typed ResRef edge (`name.ext`) for later `resolve_mention`.
fn mention_typed(out: &mut HashSet<String>, name: &str, ext: &str) {
    let name = name.trim().trim_matches('"').to_ascii_lowercase();
    if name.is_empty() || name == "****" {
        return;
    }
    if name.len() > 16 {
        return;
    }
    out.insert(format!("{name}.{ext}"));
}

/// See `swkotor.exe` `load_%s` (next to `BMPResRef`) — module transition /
/// area load pulls texture `load_<moduleRoot>`.
fn mention_module_load_screen(out: &mut HashSet<String>, module: &str) {
    let module = module.trim().trim_matches('"').to_ascii_lowercase();
    if module.is_empty() || module == "****" {
        return;
    }
    let name = format!("load_{module}");
    if name.len() > 16 {
        return;
    }
    mention_texture(out, &name);
}

/// See `ExecuteCommandStartNewModule` @00544390 — arg0 is a module name
/// (`SetMoveToModuleString`), not a resman restype. Also opens `load_%s`.
fn mention_module(out: &mut HashSet<String>, name: &str, module_roots: &HashSet<String>) {
    let name = name.trim().trim_matches('"').to_ascii_lowercase();
    if name.is_empty() || name == "****" {
        return;
    }
    if module_roots.contains(&name) {
        out.insert(name.clone());
        mention_module_load_screen(out, &name);
    }
}

/// Inventory icon ResRef: exe format string `i%s_%03d` with baseitems
/// `itemclass` + UTI `ModelVariation` (e.g. `w_BlstrPstl` + 1 → `iw_blstrpstl_001`).
fn inventory_icon_resref(itemclass: &str, variation: u32) -> Option<String> {
    let itemclass = itemclass.trim().trim_matches('"');
    if itemclass.is_empty() || itemclass == "****" {
        return None;
    }
    let name = format!("i{itemclass}_{variation:03}");
    let name = name.to_ascii_lowercase();
    if name.len() > 16 {
        return None;
    }
    Some(name)
}

#[derive(Clone, Copy)]
enum NcsStrLookup {
    Skip,
    Module,
    Types(&'static [&'static str]),
}

/// Map ACTION routine id → string-arg lookups (declaration order among strings).
///
/// See `CSWVirtualMachineCommands::InitializeCommands` @0054c960 (K1) /
/// @0077b530 (K2) and the `ExecuteCommand*` handlers below. Tag-only and
/// print/speak routines use `Skip` so their CONSTS are **consumed** (and do not
/// become file edges) without wiping the string queue for a later ResRef ACTION.
/// Nested non-string ACTIONs (`GetPartyMemberByIndex` 577, …) keep an empty
/// plan so CONSTS for the outer call survive — see `k_def_calodef` → `k_h_calo`.
///
/// K2 divergences: `QueueMovie` is 806 (K2 769 = `SetFadeUntilScript`);
/// `ShowUpgradeScreen` string arg only on K2; 821/836/864 are K2-only.
fn ncs_str_lookups(routine: u16, k2: bool) -> &'static [NcsStrLookup] {
    use NcsStrLookup::{Module, Skip, Types};
    match routine {
        // Print / speak — drain debug and VO text, no file edge.
        1 | 39 | 221 => &[Skip],
        // See `ExecuteCommandExecuteScript` @00535b70 / @0078e630 → NCS.
        8 => &[Types(&["ncs"])],
        // See `ExecuteCommandCreateItemOnObject` @0052f570 → `CSWSItem::LoadFromTemplate`.
        31 | 747 | 766 => &[Types(&["uti"])],
        // See `ExecuteCommandPlaySound` @00541170 — WAV filename, not UTS.
        46 => &[Types(&["wav"])],
        // See `ExecuteCommandEffectAreaEffect` @00530ce0 — three script ResRefs.
        171 => &[Types(&["ncs"]), Types(&["ncs"]), Types(&["ncs"])],
        // Tag-only — consume string, no file edge.
        // See `ExecuteCommandGetWaypointByTag` @0053e2b0 / `GetObjectByTag` @0053c280.
        197 | 200 | 228 => &[Skip],
        // See `ExecuteCommandSetAreaTransitionBMP` @00542410 — custom TGA.
        203 => &[Types(&["tga"])],
        // See `ExecuteCommandActionStartConversation` @0052d5b0 — DLG then ignore tags.
        204 => &[
            Types(&["dlg"]),
            Skip,
            Skip,
            Skip,
            Skip,
            Skip,
            Skip,
        ],
        // See `ExecuteCommandCreateObject` @0052f820 — template by nType.
        243 => &[Types(&["utc", "uti", "utp", "utm", "utw"])],
        // See `ExecuteCommandBeginConversation` @0052e9f0 / `SpeakOneLiner` @00543f70.
        255 | 417 => &[Types(&["dlg"])],
        // See K2 `ExecuteCommandShowUpgradeScreen` @007a84a0 — optional 2DA override.
        354 if k2 => &[Types(&["2da"])],
        // See `ExecuteCommandPlayPazaak` @00540fd0 / `ShowPartySelectionGUI` @005438b0.
        364 | 712 => &[Types(&["ncs"])],
        // Journal plot Tag — not a file.
        367 | 368 | 369 | 384 | 678 => &[Skip],
        // See `ExecuteCommandStartNewModule` @00544390 — module, waypoint Tag, six BIK.
        509 => &[
            Module,
            Skip,
            Types(&["bik"]),
            Types(&["bik"]),
            Types(&["bik"]),
            Types(&["bik"]),
            Types(&["bik"]),
            Types(&["bik"]),
        ],
        // See `ExecuteCommandSWMG_SetGunBankStringInfo` @005cc7f0 / `PlayRoomAnimation` @00540dd0.
        632 | 633 | 738 => &[Types(&["mdl"])],
        // See `ExecuteCommandAddAvailableNPCByTemplate` @0052dcd0.
        697 => &[Types(&["utc"])],
        // See `ExecuteCommandPlayMovie` @00540ed0.
        733 => &[Types(&["bik"])],
        // See `ExecuteCommandQueueMovie` @00548210 (K1 id 769).
        769 if !k2 => &[Types(&["bik"])],
        // See `ExecuteCommandQueueMovie` @007afa40 (K2 id 806).
        806 if k2 => &[Types(&["bik"])],
        // See K2 `ExecuteCommandShowDemoScreen` @007b0510.
        821 if k2 => &[Types(&["tga"])],
        // See K2 `ExecuteCommandAddAvailablePUPByTemplate` @007b0750.
        836 if k2 => &[Types(&["utc"])],
        // See K2 `ExecuteCommandDisplayMessageBox` @007b1470.
        864 if k2 => &[Types(&["tga"])],
        _ => &[],
    }
}

/// Apply ResRef lookups for one ACTION's string arguments.
///
/// `strings` must be in **nwscript declaration order** (arg0 first). Callers
/// that read raw NCS CONSTS push order must run `ncs_strings_declaration_order`
/// first.
fn apply_ncs_strings(
    routine: u16,
    k2: bool,
    strings: &[String],
    mentions: &mut HashSet<String>,
    module_roots: &HashSet<String>,
) {
    let plan = ncs_str_lookups(routine, k2);
    if plan.is_empty() {
        return;
    }
    let start = strings.len().saturating_sub(plan.len());
    let slice = &strings[start..];
    for (lookup, s) in plan.iter().zip(slice.iter()) {
        match *lookup {
            NcsStrLookup::Skip => {}
            NcsStrLookup::Module => mention_module(mentions, s, module_roots),
            NcsStrLookup::Types(exts) => {
                for ext in exts {
                    mention_typed(mentions, s, ext);
                }
            }
        }
    }
}

/// Map raw CONSTS push order → nwscript declaration order for one ACTION.
///
/// NCS pushes ACTION args right-to-left, so the **last** pushed string is arg0.
/// Only the trailing `n_args` CONSTS belong to this ACTION — earlier strings may
/// be leftovers from comparisons / debug prints (see `k_sup_galaxymap` STUNT_00
/// call sites with a dozen prior CONSTS). Take the last `n_args`, then reverse.
///
/// Concrete K1 evidence: `k_pebn_exithawk` ACTION 509 leaves
/// `…, "k_dan_ebon_hawk_transition", "danm13"` — `danm13` is arg0 (module).
fn ncs_strings_declaration_order(pushed: &[String], n_args: usize) -> Vec<String> {
    if n_args == 0 || pushed.is_empty() {
        return Vec::new();
    }
    let n = n_args.min(pushed.len());
    pushed[pushed.len() - n..]
        .iter()
        .rev()
        .cloned()
        .collect()
}

/// Walk NCS bytecode; only string args of ResRef-taking ACTIONS become edges.
///
/// See `CVirtualMachineInternal::ExecuteCode` ACTION dispatch and
/// `CSWVirtualMachineCommands::ExecuteCommand` — CONSTS alone are not lookups.
///
/// Nested non-string ACTIONs (e.g. `GetPartyMemberByIndex` while evaluating
/// `ActionStartConversation`'s object arg) must **not** wipe the CONSTS queue:
/// those strings belong to the outer call. Only an ACTION with a non-empty
/// string plan consumes (and truncates) trailing CONSTS.
fn take_ncs_actions(
    instructions: &[ncs::Instruction],
    k2: bool,
    mentions: &mut HashSet<String>,
    module_roots: &HashSet<String>,
) {
    let mut recent = Vec::new();
    for ins in instructions {
        if ins.op == "CONSTS" {
            for arg in &ins.args {
                if let ncs::Arg::Str(s) = arg {
                    recent.push(s.clone());
                }
            }
            continue;
        }
        if ins.op == "ACTION" {
            if let Some(rid) = ins.routine {
                let plan_len = ncs_str_lookups(rid, k2).len();
                if plan_len > 0 {
                    let declared = ncs_strings_declaration_order(&recent, plan_len);
                    apply_ncs_strings(rid, k2, &declared, mentions, module_roots);
                    let n = plan_len.min(recent.len());
                    recent.truncate(recent.len() - n);
                }
            }
        }
    }
}

/// GFF label → resman types the engine requests for that field.
///
/// See:
/// - `CSWSCreature::ReadScriptsFromGff` @004ebf20 — `Script*` → NCS
/// - `CSWSDoor::LoadDoor` @0058a1f0 / `CSWSPlaceable::LoadPlaceable` @00585670 —
///   `On*` / `MineScript` → NCS; `Conversation` → DLG
/// - `CSWSTrigger::LoadTrigger` @0058da80 — `TrapScript` → NCS
/// - `CSWSArea::LoadAreaHeader` @00508c50 — ARE `On*` → NCS
/// - `CSWSModule::LoadModuleStart` @004c9050 — `Mod_On*` → NCS;
///   `Mod_Entry_Area` → ARE; `Mod_StartMovie` → BIK
/// - `CSWSDialog::LoadDialogBase` @0059f5f0 — `Script`/`Active` → NCS;
///   `VO_ResRef`/`Sound` → stream WAV (`CSWSDialog::SendDialogEntry` K1
///   STREAMWAVES @005a4010 / K2 STREAMVOICE @00754e50) and lip via
///   `CSWCCreature::LipSync` → `CLIP::LoadLip` (type 3004)
/// - `CSWSCreatureStats::ReadStatsFromGff` @005afce0 — `Conversation` → DLG
/// - `CSWSArea::LoadCreatures` etc. — `TemplateResRef` → templates
/// - UTC `Equip_ItemList`/`EquippedRes` and UTP `InventoryRes` → UTI via
///   `CSWSItem::LoadFromTemplate` (CreateItemOnObject @0052f570)
fn gff_field_exts(label: &str) -> Option<&'static [&'static str]> {
    let l = label.to_ascii_lowercase();
    Some(match l.as_str() {
        "conversation" => &["dlg"],
        "vo_resref" => &["wav", "lip"],
        // DLG node Sound is VO fallback (same path as VO_ResRef). UTS sound
        // lists are separate; bare label `Sound` on DLG is the VO case.
        "sound" => &["wav", "lip"],
        "templateresref" => &["utc", "uti", "utp", "utm", "utw", "utd", "ute", "uts", "utt"],
        // See `CSWSItem::LoadFromTemplate` (same path as CreateItemOnObject @0052f570).
        // UTC `Equip_ItemList` rows store the blueprint in `EquippedRes`; UTP/UTM
        // container loot uses `InventoryRes`. Either label is a UTI ResRef.
        "inventoryres" | "equippedres" => &["uti"],
        // See `CSWGuiPanel::StartLoadFromLayout` / `LoadFromLayout` (research Q4
        // GUI panel table) — control ResRefs `FILL`/`EDGE`/`CORNER`/`FONT`/`IMAGE`
        // open textures through the GUI renderer (TPC, TGA fallback, TXI info).
        "fill" | "edge" | "corner" | "font" | "image" => &["tpc", "tga", "txi"],
        // See `swkotor.exe` GFF label `IconResRef` — item / feat icon override.
        "iconresref" => &["tpc", "tga", "txi"],
        "mod_entry_area" | "area_name" => &["are"],
        "mod_startmovie" => &["bik"],
        "cameramodel" | "stuntmodel" => &["mdl"],
        // See `CSWSDialog::LoadDialog` @005a2ae0 — AmbientTrack is streammusic
        // (`HD0:STREAMMUSIC\%s`); install ships `.wav` (optional `.bmu`).
        "ambienttrack" => &["wav", "bmu"],
        "active" | "active2" | "script" | "script2" | "endconversation" | "endconverabort"
        | "trapscript" | "minescript" | "heartbeat" | "onenter" | "onexit" | "onuserdefined" => {
            &["ncs"]
        }
        "actionparamstra" | "actionparamstrb" | "paramstra" | "paramstrb" => &["ncs"],
        _ if l.starts_with("script") || l.starts_with("on") || l.starts_with("mod_on") => &["ncs"],
        "linkedtomodule" => return None,
        _ => return None,
    })
}

/// See `CSWSDoor::LoadDoor` @0058a1f0 / `CSWSTrigger::LoadTrigger` @0058da80 —
/// `LinkedToModule` is a module name for `StartNewModule`, not a restype.
fn is_module_name_field(label: &str) -> bool {
    label.eq_ignore_ascii_case("LinkedToModule")
}

fn bfs(
    index: &Index,
    seeds: &[u32],
    edges: &HashMap<u32, HashSet<String>>,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
) -> (HashSet<u32>, HashMap<u32, u32>) {
    let mut seen = HashSet::new();
    let mut parent = HashMap::new();
    let mut q = VecDeque::new();
    for &id in seeds {
        if seen.insert(id) {
            q.push_back(id);
        }
    }
    while let Some(id) = q.pop_front() {
        let scope = resource_scope(index, &index.resources[id as usize]);
        let Some(tokens) = edges.get(&id) else {
            continue;
        };
        for tok in tokens {
            for j in resolve_mention(index, loaded, module_entries, &scope, tok) {
                if seen.insert(j) {
                    parent.insert(j, id);
                    q.push_back(j);
                }
            }
        }
    }
    (seen, parent)
}

/// Insert same-resref mentions from each ARE loaded copy onto global `lyt`/`vis`/`pth`
/// when those types exist. Tokenizer still skips bare self-resref; this is the
/// supported path onto chitin layouts.
/// Strip `//` and `/* */` outside string literals so include walking does
/// not treat comments as compiler input (the game never parses `.nss`).
fn nwscript_without_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn parse_include_line(line: &str) -> Option<String> {
    let t = line.trim();
    let rest = t
        .strip_prefix("#include")
        .or_else(|| t.strip_prefix("#INCLUDE"))?;
    let rest = rest.trim_start();
    let (inner, _) = if let Some(s) = rest.strip_prefix('"') {
        s.split_once('"')?
    } else if let Some(s) = rest.strip_prefix('<') {
        s.split_once('>')?
    } else {
        return None;
    };
    let name = inner
        .trim()
        .trim_end_matches(".nss")
        .trim_end_matches(".NSS")
        .to_ascii_lowercase();
    if name.is_empty() || name.len() > 16 {
        None
    } else {
        Some(name)
    }
}

/// Body text the compiler would see, plus `#include` ResRefs (not themselves
/// RunScript targets).
fn nss_body_and_includes(src: &str) -> (String, Vec<String>) {
    let code = nwscript_without_comments(src);
    let mut body = String::new();
    let mut includes = Vec::new();
    for line in code.lines() {
        if let Some(name) = parse_include_line(line) {
            includes.push(name);
            body.push('\n');
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    (body, includes)
}

fn nss_source(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    scope: &Scope,
    resref: &str,
) -> Option<String> {
    let ty = ResType::from_extension("nss")?;
    let key = resref.to_ascii_lowercase();
    let mut id = None;
    if let Some(&i) = loaded.get(&(None, key.clone(), ty)) {
        if index.source(&index.resources[i as usize]).kind == kq_index::SourceKind::Override {
            id = Some(i);
        }
    }
    if id.is_none() {
        if let Some(m) = scope {
            id = loaded.get(&(Some(m.clone()), key.clone(), ty)).copied();
        }
    }
    if id.is_none() {
        id = loaded.get(&(None, key, ty)).copied();
    }
    let r = &index.resources[id? as usize];
    let bytes = read::read(index, r).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn take_nss_tree(
    src: &str,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    self_ref: &str,
    mentions: &mut HashSet<String>,
    missing: &mut HashSet<String>,
    seen_inc: &mut HashSet<String>,
) {
    let mut pending = VecDeque::new();
    let (body, incs) = nss_body_and_includes(src);
    take_tokens(
        &body,
        index,
        loaded,
        module_entries,
        scope,
        module_roots,
        catalog,
        self_ref,
        Some("ncs"),
        mentions,
        Some(missing),
    );
    pending.extend(incs);
    while let Some(inc) = pending.pop_front() {
        if !seen_inc.insert(inc.clone()) {
            continue;
        }
        let Some(text) = nss_source(index, loaded, scope, &inc) else {
            continue;
        };
        let (body, nested) = nss_body_and_includes(&text);
        // Include source is compiler input for this NCS, not a live `.nss`
        // edge. Tokens become mentions of the compiled script.
        take_tokens(
            &body,
            index,
            loaded,
            module_entries,
            scope,
            module_roots,
            catalog,
            self_ref,
            Some("ncs"),
            mentions,
            Some(missing),
        );
        pending.extend(nested);
    }
}

/// Same-resref companions for an ARE after the area is loaded.
///
/// See `CSWSArea::LoadRoomInfo` @005073d0 → `CLYT::LoadLayout` (`.lyt`);
/// `CSWSArea::LoadPathPoints` @00508400 (`.pth`); client
/// `Scene::LoadVisibility` @004568d0 (`.vis`) via `CSWCArea::LoadArea`.
fn add_are_layout_edges(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    edges: &mut HashMap<u32, HashSet<String>>,
) {
    let _ = index;
    let are = ResType::from_extension("are").unwrap();
    for ((scope, resref, ty), &id) in loaded {
        if *ty != are {
            continue;
        }
        let _ = scope;
        for ext in ["lyt", "vis", "pth"] {
            let Some(layout_ty) = ResType::from_extension(ext) else {
                continue;
            };
            if loaded.contains_key(&(None, resref.clone(), layout_ty)) {
                edges
                    .entry(id)
                    .or_default()
                    .insert(format!("{resref}.{ext}"));
            }
        }
    }
}

/// Module entry → `load_<module>` textures (`swkotor.exe` `load_%s`).
fn add_module_load_screen_edges(
    index: &Index,
    module_entries: &HashMap<String, Vec<u32>>,
    edges: &mut HashMap<u32, HashSet<String>>,
) {
    let _ = index;
    for (module, ids) in module_entries {
        let Some(&id) = ids.first() else {
            continue;
        };
        let mut toks = HashSet::new();
        mention_module_load_screen(&mut toks, module);
        if !toks.is_empty() {
            edges.entry(id).or_default().extend(toks);
        }
    }
}

/// Resolution × panel-suffix GUI backgrounds (`800x600`… + `back`/`comp0`/…).
///
/// See `swkotor.exe` resolution table next to `BACKGROUND` and panel suffixes
/// `comp`/`map`/`pazaak`/`store`/`back` — install filenames use `comp0`/`comp1`.
fn add_resolution_gui_texture_edges(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    edges: &mut HashMap<u32, HashSet<String>>,
) {
    let Some(gui_ty) = ResType::from_extension("gui") else {
        return;
    };
    // Host on a seeded always-on panel so BFS from ENGINE_GUIS reaches them.
    const HOSTS: &[&str] = &["mainmenu", "loadscreen", "map", "pazaakgame", "computer"];
    let mut host = None;
    for h in HOSTS {
        if let Some(&id) = loaded.get(&(None, (*h).to_string(), gui_ty)) {
            host = Some(id);
            break;
        }
    }
    let Some(id) = host else {
        let _ = index;
        return;
    };
    let mut toks = HashSet::new();
    mention_resolution_gui_textures(&mut toks);
    edges.entry(id).or_default().extend(toks);
}

fn mention_resolution_gui_textures(out: &mut HashSet<String>) {
    const RESOLUTIONS: &[&str] = &["800x600", "1024x768", "1280x960", "1600x1200"];
    const SUFFIXES: &[&str] = &["back", "comp0", "comp1", "load", "map", "pazaak", "store"];
    for res in RESOLUTIONS {
        for suf in SUFFIXES {
            mention_texture(out, &format!("{res}{suf}"));
        }
    }
}

/// `baseitems.2da` `itemclass` column by row index (= UTI `BaseItem`).
fn load_baseitem_itemclasses(
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
) -> Vec<String> {
    let Some(ty) = ResType::from_extension("2da") else {
        return Vec::new();
    };
    let Some(&id) = loaded.get(&(None, "baseitems".into(), ty)) else {
        return Vec::new();
    };
    let r = &index.resources[id as usize];
    let path = &index.files[r.file as usize];
    let Ok(map) = read::map_file(path) else {
        return Vec::new();
    };
    let end = r.offset as usize + r.size as usize;
    if end > map.len() {
        return Vec::new();
    }
    let bytes = &map[r.offset as usize..end];
    let Ok(t) = twoda::read_or_salvage(bytes, Path::new("baseitems.2da")) else {
        return Vec::new();
    };
    let Some(col) = t
        .columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case("itemclass"))
    else {
        return Vec::new();
    };
    t.rows
        .iter()
        .map(|row| {
            row.get(col)
                .map(|s| s.as_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        })
        .collect()
}

fn mention_uti_inventory_icon(
    root: &GffStruct,
    itemclasses: &[String],
    mentions: &mut HashSet<String>,
) {
    let base = match root.get("BaseItem") {
        Some(GffValue::Int(n)) if *n >= 0 => *n as usize,
        Some(GffValue::UInt(n)) => *n as usize,
        _ => return,
    };
    let variation = match root.get("ModelVariation") {
        Some(GffValue::Int(n)) if *n > 0 => *n as u32,
        Some(GffValue::UInt(n)) if *n > 0 => *n as u32,
        _ => 1,
    };
    let Some(itemclass) = itemclasses.get(base) else {
        return;
    };
    if let Some(icon) = inventory_icon_resref(itemclass, variation) {
        mention_texture(mentions, &icon);
    }
}

fn scan_bytes(
    bytes: &[u8],
    r: &kq_index::Resource,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    itemclasses: &[String],
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
    missing: &mut HashSet<String>,
) {
    let ext = r.restype.extension();
    let path = Path::new("<scan>");
    if ext == Some("mdl") {
        // Binary MDL vertex buffer is the same-ResRef MDX (see `kq_format::mdl`
        // / PyKotor MDL-MDX). Area rooms also open same-ResRef WOK walkmeshes;
        // placeables/doors open PWK/DWK (`bwm.py` walkmesh types).
        mention_typed(mentions, &r.resref, "mdx");
        mention_typed(mentions, &r.resref, "wok");
        mention_typed(mentions, &r.resref, "pwk");
        mention_typed(mentions, &r.resref, "dwk");
        // Bitmap / lightmap names → TPC/TGA + TXI (model loader + texture info).
        for name in kq_format::mdl::texture_refs(bytes) {
            mention_texture(mentions, &name);
        }
        return;
    }
    if ext == Some("tpc") || ext == Some("tga") {
        // Loose / BIF texture without going through an MDL still needs its TXI.
        mention_typed(mentions, &r.resref, "txi");
        return;
    }
    if ext == Some("ncs") {
        let Ok(n) = ncs::read(bytes, path) else {
            return;
        };
        let k2 = matches!(index.game, kq_index::Game::K2);
        take_ncs_actions(&n.instructions, k2, mentions, module_roots);
        let _ = (loaded, catalog, missing, scope, itemclasses);
        return;
    }
    if gff::sniff(bytes) {
        let Ok(g) = gff::read(bytes, path) else {
            return;
        };
        walk_gff_struct(
            &g.root,
            index,
            loaded,
            module_entries,
            scope,
            module_roots,
            catalog,
            &r.resref,
            ext,
            strref_mode(r.restype),
            mentions,
            strrefs,
            missing,
        );
        if ext == Some("uti") {
            mention_uti_inventory_icon(&g.root, itemclasses, mentions);
        }
        if ext == Some("ifo") {
            if let Some(m) = scope {
                mention_module_load_screen(mentions, m);
            }
        }
        return;
    }
    if ext == Some("2da") || twoda::sniff(bytes) {
        let Ok(t) = twoda::read_or_salvage(bytes, path) else {
            return;
        };
        // See `Load2DArrays_StateScripts` @005c27c0 — every string cell is an NCS
        // ResRef. `aiscripts.2da` is GUI-only (`CSWGuiScriptSelect` / K2
        // `Load2DArrays_AIScripts`) — NAME_STRREF / AISTATE, not RunScript.
        let scan_all_ncs = r.resref.eq_ignore_ascii_case("statescripts");
        if scan_all_ncs {
            for label in &t.labels {
                mention_typed(mentions, label, "ncs");
            }
        }
        for row in &t.rows {
            for (i, cell) in row.iter().enumerate() {
                let col = t.columns.get(i).map(|s| s.as_str()).unwrap_or("");
                if scan_all_ncs {
                    mention_typed(mentions, cell, "ncs");
                }
                for ext in twoda_cell_exts(&r.resref, col) {
                    mention_typed(mentions, cell, ext);
                }
                if is_strref_column(col) {
                    if let Some(n) = parse_strref(cell) {
                        strrefs.insert(n);
                    }
                }
            }
        }
        return;
    }
    if ext == Some("ssf") || ssf::sniff(bytes) {
        let Ok(s) = ssf::read(bytes, path) else {
            return;
        };
        for n in s.sounds {
            if n >= 0 {
                strrefs.insert(n);
            }
        }
        return;
    }
    if ext == Some("lyt") || ext == Some("vis") {
        if let Ok(text) = std::str::from_utf8(bytes) {
            take_layout_mdl_mentions(text, mentions);
        }
        return;
    }
    let _ = (index, loaded, module_entries, scope, catalog, missing);
}

fn walk_gff_struct(
    s: &GffStruct,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    self_ref: &str,
    self_ext: Option<&str>,
    mode: StrRefMode,
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
    missing: &mut HashSet<String>,
) {
    for (label, value) in &s.fields {
        walk_gff_value(
            value,
            Some(label),
            index,
            loaded,
            module_entries,
            scope,
            module_roots,
            catalog,
            self_ref,
            self_ext,
            mode,
            mentions,
            strrefs,
            missing,
        );
    }
}

/// True when `gff_field_exts` or `LinkedToModule` would emit an edge.
fn is_engine_resref_field(label: &str) -> bool {
    is_module_name_field(label) || gff_field_exts(label).is_some()
}

fn walk_gff_value(
    v: &GffValue,
    key: Option<&str>,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    self_ref: &str,
    self_ext: Option<&str>,
    mode: StrRefMode,
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
    missing: &mut HashSet<String>,
) {
    match v {
        GffValue::Str(s) => {
            let Some(k) = key else {
                return;
            };
            if is_module_name_field(k) {
                mention_module(mentions, s, module_roots);
            } else if let Some(exts) = gff_field_exts(k) {
                // See `CSWSSoundObject::Load` @005c9040 — UTS `Sounds` entries
                // are WAV only (no lip). DLG `Sound`/`VO_ResRef` still get both.
                let exts = if self_ext == Some("uts")
                    && (k.eq_ignore_ascii_case("sound") || k.eq_ignore_ascii_case("sounds"))
                {
                    &["wav"][..]
                } else {
                    exts
                };
                for ext in exts {
                    mention_typed(mentions, s, ext);
                }
            } else if k.eq_ignore_ascii_case("resref") {
                // See `CSWSArea::LoadStores` — store instances use `ResRef` (UTI/… templates).
                for ext in ["utc", "uti", "utp", "utm", "utw", "utd", "ute", "uts", "utt"] {
                    mention_typed(mentions, s, ext);
                }
            }
            let _ = (index, loaded, module_entries, scope, catalog, self_ref, self_ext, missing);
        }
        GffValue::LocString { strref, substrings: _ } => {
            // Spoken/UI text is not a ResRef source. Tokenizing it is what
            // made a K1 graph scan take hours (`the` / `because` through
            // resolve_in_scope). The talk-table index is the only live edge.
            if matches!(mode, StrRefMode::Gff) && *strref >= 0 {
                strrefs.insert(*strref);
            }
        }
        GffValue::StrRef(n) => {
            if matches!(mode, StrRefMode::Gff) {
                strrefs.insert(*n);
            }
        }
        GffValue::Struct(child) => walk_gff_struct(
            child,
            index,
            loaded,
            module_entries,
            scope,
            module_roots,
            catalog,
            self_ref,
            self_ext,
            mode,
            mentions,
            strrefs,
            missing,
        ),
        GffValue::List(items) => {
            for child in items {
                walk_gff_struct(
                    child,
                    index,
                    loaded,
                    module_entries,
                    scope,
                    module_roots,
                    catalog,
                    self_ref,
                    self_ext,
                    mode,
                    mentions,
                    strrefs,
                    missing,
                );
            }
        }
        _ => {
            let _ = key;
        }
    }
}

#[cfg(test)]
fn collect(
    decoded: &crate::render::Decoded,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    self_ref: &str,
    self_ext: Option<&str>,
    mode: StrRefMode,
    mentions: &mut HashSet<String>,
    strrefs: &mut HashSet<i64>,
    missing: &mut HashSet<String>,
) {
    match decoded {
        crate::render::Decoded::Value(v) => {
            let mut w = Walk {
                index,
                loaded,
                module_entries,
                scope,
                module_roots,
                catalog,
                self_ref,
                self_ext,
                mode,
                mentions,
                strrefs,
                missing,
            };
            // ACTION string-arg ResRefs only — see `take_ncs_actions`.
            if self_ext == Some("ncs")
                || v.get("instructions").is_some_and(serde_json::Value::is_array)
            {
                walk_ncs_actions_json(v, &mut w);
            } else {
                walk_json(v, &mut w, None);
            }
        }
        crate::render::Decoded::Text(s) => take_tokens(
            s,
            index,
            loaded,
            module_entries,
            scope,
            module_roots,
            catalog,
            self_ref,
            self_ext,
            mentions,
            Some(missing),
        ),
        crate::render::Decoded::Opaque { .. } => {}
    }
}

#[cfg(test)]
struct Walk<'a> {
    index: &'a Index,
    loaded: &'a HashMap<(Scope, String, ResType), u32>,
    module_entries: &'a HashMap<String, Vec<u32>>,
    scope: &'a Scope,
    module_roots: &'a HashSet<String>,
    catalog: &'a HashSet<String>,
    self_ref: &'a str,
    self_ext: Option<&'a str>,
    mode: StrRefMode,
    mentions: &'a mut HashSet<String>,
    strrefs: &'a mut HashSet<i64>,
    missing: &'a mut HashSet<String>,
}

#[cfg(test)]
fn walk_ncs_actions_json(v: &serde_json::Value, w: &mut Walk<'_>) {
    let Some(instructions) = v.get("instructions").and_then(serde_json::Value::as_array) else {
        return;
    };
    let k2 = matches!(w.index.game, kq_index::Game::K2);
    let mut recent: Vec<String> = Vec::new();
    for ins in instructions {
        let Some(map) = ins.as_object() else {
            continue;
        };
        let Some(op) = map.get("op").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if op == "CONSTS" {
            if let Some(args) = map.get("args").and_then(serde_json::Value::as_array) {
                for arg in args {
                    if let serde_json::Value::String(s) = arg {
                        recent.push(s.clone());
                    }
                }
            }
            continue;
        }
        if op == "ACTION" {
            if let Some(rid) = map.get("routine").and_then(serde_json::Value::as_u64) {
                let plan_len = ncs_str_lookups(rid as u16, k2).len();
                if plan_len > 0 {
                    let declared = ncs_strings_declaration_order(&recent, plan_len);
                    apply_ncs_strings(rid as u16, k2, &declared, w.mentions, w.module_roots);
                    let n = plan_len.min(recent.len());
                    recent.truncate(recent.len() - n);
                }
            }
        }
    }
}

#[cfg(test)]
fn walk_json(v: &serde_json::Value, w: &mut Walk<'_>, key: Option<&str>) {
    match v {
        serde_json::Value::String(s) => {
            take_tokens(
                s,
                w.index,
                w.loaded,
                w.module_entries,
                w.scope,
                w.module_roots,
                w.catalog,
                w.self_ref,
                w.self_ext,
                w.mentions,
                Some(w.missing),
            );
            if matches!(w.mode, StrRefMode::TwoDa) && key.is_some_and(is_strref_column) {
                if let Some(n) = parse_strref(s) {
                    w.strrefs.insert(n);
                }
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                match w.mode {
                    StrRefMode::Ssf => {
                        w.strrefs.insert(i);
                    }
                    StrRefMode::Gff if key.is_some_and(|k| k.eq_ignore_ascii_case("strref")) => {
                        w.strrefs.insert(i);
                    }
                    StrRefMode::TwoDa if key.is_some_and(is_strref_column) => {
                        w.strrefs.insert(i);
                    }
                    _ => {}
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_json(item, w, key);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, val) in map {
                // Do not tokenize object keys (column labels, NCS field names, etc.).
                walk_json(val, w, Some(k));
            }
        }
        _ => {}
    }
}

fn is_strref_column(col: &str) -> bool {
    let c = col.to_ascii_lowercase();
    if c == "_row" || c == "label" {
        return false;
    }
    c == "name"
        || c == "description"
        || c == "desc"
        || c == "text"
        || c == "title"
        || c == "feedback"
        || c == "tooltip"
        || c.contains("strref")
        || c.contains("stringref")
        || c.ends_with("string")
}

/// 2DA column whose cells are talk-table indices (for `kq cat` TLK resolve).
pub fn twoda_strref_column(col: &str) -> bool {
    is_strref_column(col)
}

/// Columns whose cells are nested 2DA filenames.
///
/// See `CSWClass::LoadAttackBonusTable` @005bcda0 … `LoadSpellKnownTable`
/// @005bdbe0; `CTwoDimArrays::LoadIPRPCostTables` @005c4730 /
/// `LoadIPRPParamTables` @005c49c0 (`Name` / `TableResRef`).
fn is_2da_table_column(col: &str) -> bool {
    matches!(
        col.to_ascii_lowercase().as_str(),
        "tableresref"
            | "subtyperesref"
            | "attackbonustable"
            | "featstable"
            | "savingthrowtable"
            | "skillstable"
            | "spellgaintable"
            | "spellknowntable"
            | "featgain"
            | "armorclasscolumn"
            | "costtableresref"
            | "param1resref"
    )
}

/// Whether a 2DA column holds a resource name (vs StrRef / label).
fn is_2da_live_column(table: &str, col: &str) -> bool {
    !twoda_cell_exts(table, col).is_empty()
}

/// Resource types a 2DA cell names. Empty = not a file lookup.
///
/// See `CSWSTrigger::LoadTrigger` / `Load2DArrays_Traps` — `TrapScript` → NCS;
/// `CSWSpellArray::Load` — `ImpactScript` → NCS;
/// `CSWSCreature::ApplyDiseasePayload` @004f6790 — disease script columns → NCS;
/// `CSoundSetList::CSoundSetList` @00678c10 — `RESREF` → SSF (type 2060), not WAV.
fn twoda_cell_exts(table: &str, col: &str) -> &'static [&'static str] {
    if is_2da_table_column(col) {
        return &["2da"];
    }
    let c = col.to_ascii_lowercase();
    let t = table.to_ascii_lowercase();
    if matches!(t.as_str(), "iprp_costtable" | "iprp_paramtable") && c == "name" {
        return &["2da"];
    }
    if t == "soundset" && c == "resref" {
        return &["ssf"];
    }
    // See `swkotor.exe` `HD0:STREAMMUSIC\%s` / `AmbientMusic` / `AmbientSound`
    // Load2DArrays — `resource` + `stinger*` cells are streammusic/streamsounds
    // ResRefs (`.wav` on disk).
    if matches!(t.as_str(), "ambientmusic" | "ambientsound")
        && (c == "resource" || c.starts_with("stinger"))
    {
        return &["wav"];
    }
    // `aiscripts` cells are GUI metadata, not script ResRefs.
    if t == "aiscripts" {
        return &[];
    }
    if c.contains("script") {
        return &["ncs"];
    }
    // Footstep / placeable / GUI sound tables name WAVs in surface or state
    // columns (`dirt0`, `opened`, …) — the column label often lacks "sound".
    if matches!(t.as_str(), "footstepsounds" | "placeableobjsnds" | "guisounds")
        && !is_strref_column(&c)
        && !matches!(c.as_str(), "label" | "rowname")
    {
        return &["wav"];
    }
    if c.contains("sound") || c.contains("wav") {
        return &["wav"];
    }
    if c.contains("model") || c.ends_with("_node") || c.ends_with("visual") {
        // visualeffects `*_node`, spells `CastHandVisual`, … → MDL.
        return &["mdl"];
    }
    // appearance `texA`… / `RaceTex`, heads `HeadTex*`, `EnvMap` — textures.
    // Prefer prefix/suffix `tex` so unrelated columns are not swept up.
    if c.starts_with("tex")
        || c.ends_with("tex")
        || c == "envmap"
        || c.starts_with("headtex")
        || c.contains("icon")
        || c.contains("texture")
        || c.contains("portrait")
    {
        return &["tga", "tpc", "txi"];
    }
    // heads.2da `Head` is the head MDL ResRef.
    if t == "heads" && c == "head" {
        return &["mdl"];
    }
    // upgrade / upcrystals `Template` (+ `*mdlvar` item blueprints) → UTI.
    if c == "template" || c.ends_with("mdlvar") {
        return &["uti"];
    }
    // See `swkotor.exe` `BMPResRef` on loadscreens.2da — load-screen textures
    // (`LOAD_CHARGEN`, …), not template ResRefs. Must beat the generic `*resref`
    // branch below.
    if c == "bmpresref" {
        return &["tga", "tpc", "txi"];
    }
    // See loadscreens `MusicResRef` / exe `MusicResRef` — streammusic cue, not a
    // template ResRef.
    if c == "musicresref" {
        return &["wav"];
    }
    if c == "filename" {
        // See `Load2DArrays_Movies` / movie list — Filename cells are BIK ResRefs.
        return &["bik"];
    }
    if c == "resref" || c.ends_with("resref") {
        // Generic *ResRef on templates / dialogs — multi-type probe.
        return &["utc", "uti", "utp", "utm", "utw", "utd", "ute", "uts", "utt", "dlg"];
    }
    &[]
}

/// Room / model names inside LYT/VIS text → MDL ResRefs (+ MDX/WOK companions).
///
/// See `CSWSArea::LoadRoomInfo` @005073d0 / `CLYT::LoadLayout` @005df140.
/// PyKotor `LYT.iter_resource_identifiers` yields MDL+MDX+WOK per room model.
fn take_layout_mdl_mentions(text: &str, mentions: &mut HashSet<String>) {
    let lower = text.to_ascii_lowercase();
    let mut start = None;
    for (i, c) in lower.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(st) = start.take() {
            let tok = &lower[st..i];
            if tok.len() > 1 && tok.len() <= 16 {
                mention_room_model(mentions, tok);
            }
        }
    }
    if let Some(st) = start {
        let tok = &lower[st..];
        if tok.len() > 1 && tok.len() <= 16 {
            mention_room_model(mentions, tok);
        }
    }
}

fn mention_room_model(out: &mut HashSet<String>, name: &str) {
    mention_typed(out, name, "mdl");
    mention_typed(out, name, "mdx");
    mention_typed(out, name, "wok");
}

fn parse_strref(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s == "****" {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    if n < 0 {
        return None;
    }
    Some(n)
}

fn take_tokens(
    s: &str,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    self_ref: &str,
    self_ext: Option<&str>,
    out: &mut HashSet<String>,
    mut missing: Option<&mut HashSet<String>>,
) {
    let lower = s.to_ascii_lowercase();
    let mut start = None;
    for (i, c) in lower.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' || c == '!' || c == '+' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(st) = start.take() {
            consider_token(
                &lower[st..i],
                index,
                loaded,
                module_entries,
                scope,
                module_roots,
                catalog,
                self_ref,
                self_ext,
                out,
                missing.as_deref_mut(),
                false,
            );
        }
    }
    if let Some(st) = start {
        consider_token(
            &lower[st..],
            index,
            loaded,
            module_entries,
            scope,
            module_roots,
            catalog,
            self_ref,
            self_ext,
            out,
            missing,
            false,
        );
    }
}

fn looks_like_resref_token(tok: &str) -> bool {
    tok.contains('_') || tok.contains('.') || tok.bytes().any(|b| b.is_ascii_digit())
}

fn consider_token(
    tok: &str,
    index: &Index,
    loaded: &HashMap<(Scope, String, ResType), u32>,
    module_entries: &HashMap<String, Vec<u32>>,
    scope: &Scope,
    module_roots: &HashSet<String>,
    catalog: &HashSet<String>,
    self_resref: &str,
    self_ext: Option<&str>,
    out: &mut HashSet<String>,
    missing: Option<&mut HashSet<String>>,
    allow_self: bool,
) {
    if tok.is_empty() || tok == "****" {
        return;
    }
    if tok.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    // Free-token scans skip the owner ResRef (avoids every GFF mentioning
    // itself). Engine ResRef *fields* pass allow_self: UTC Conversation and
    // DLG VO_ResRef often reuse the owner name for a different restype.
    if !allow_self && tok == self_resref {
        return;
    }
    if let Some(ext) = self_ext {
        if tok.len() == self_resref.len() + 1 + ext.len()
            && tok.as_bytes().get(self_resref.len()) == Some(&b'.')
            && tok.starts_with(self_resref)
            && tok.ends_with(ext)
        {
            return;
        }
    }
    let ident_len = tok
        .rsplit_once('.')
        .map(|(base, _)| base.len())
        .unwrap_or(tok.len());
    if ident_len == 0 || ident_len > 16 {
        return;
    }
    // Dialogue prose is full of English words. Those are not ResRefs. Looking
    // each one up in the install is what made `kq graph` take hours.
    if !catalog.contains(tok) && !module_roots.contains(tok) {
        if let Some((base, ext)) = tok.rsplit_once('.') {
            if ResType::from_extension(ext).is_some()
                && (catalog.contains(base) || module_roots.contains(base))
            {
                out.insert(base.to_string());
            } else if looks_like_resref_token(tok) {
                if let Some(m) = missing {
                    m.insert(tok.to_string());
                }
            }
            return;
        }
        if looks_like_resref_token(tok) {
            if let Some(m) = missing {
                if tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '!' || c == '+')
                {
                    m.insert(tok.to_string());
                }
            }
        }
        return;
    }
    if !resolve_in_scope(index, loaded, module_entries, scope, tok).is_empty()
        || module_roots.contains(tok)
    {
        out.insert(tok.to_string());
        return;
    }
    if let Some((base, ext)) = tok.rsplit_once('.') {
        if ResType::from_extension(ext).is_some()
            && (!resolve_in_scope(index, loaded, module_entries, scope, base).is_empty()
                || module_roots.contains(base))
        {
            out.insert(base.to_string());
            return;
        }
    }
    if let Some(m) = missing {
        if tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            m.insert(tok.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Decoded;

    fn catalog_of(index: &Index) -> HashSet<String> {
        index.resources.iter().map(|r| r.resref.clone()).collect()
    }

    fn global_lookup(
        resrefs: &[&str],
    ) -> (
        Index,
        HashMap<(Scope, String, ResType), u32>,
        HashMap<String, Vec<u32>>,
    ) {
        use serde_json::json;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let resources: Vec<serde_json::Value> = resrefs
            .iter()
            .map(|rr| {
                json!({
                    "resref": rr,
                    "restype": ncs,
                    "file": 0,
                    "offset": 0,
                    "size": 1,
                    "source": 0
                })
            })
            .collect();
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/data/scripts.bif"],
            "sources": [
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": resources,
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        (index, loaded, module_entries)
    }

    fn fixture_two_modules_shared_are() -> Index {
        use serde_json::json;
        let are = ResType::from_extension("are").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/ebo_m12aa.mod",
                "/game/modules/ebo_m40ad.mod",
                "/game/modules/ebo_m12aa_s.rim",
                "/game/Override/shared.ncs",
                "/game/data/scripts.bif"
            ],
            "sources": [
                {"kind":"module-mod","label":"ebo_m12aa.mod","precedence":100,"module_root":"ebo_m12aa"},
                {"kind":"module-mod","label":"ebo_m40ad.mod","precedence":100,"module_root":"ebo_m40ad"},
                {"kind":"module-rim","label":"ebo_m12aa_s.rim","precedence":200,"module_root":"ebo_m12aa"},
                {"kind":"override","label":"Override","precedence":0,"module_root":null},
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"module","restype":ifo,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"m12aa","restype":are,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m12aa","restype":are,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"local","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"local","restype":ncs,"file":2,"offset":0,"size":1,"source":2},
                {"resref":"shared","restype":ncs,"file":3,"offset":0,"size":1,"source":3},
                {"resref":"shared","restype":ncs,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn group_scan_ids_orders_offsets_within_file() {
        let mut index = fixture_two_modules_shared_are();
        // Fixture offsets are all 0; scramble file-0 so sort is observable.
        index.resources[0].offset = 300;
        index.resources[2].offset = 100;
        index.resources[4].offset = 200;
        index.resources[7].offset = 50;
        let ids: Vec<u32> = (0..index.resources.len() as u32).collect();
        let groups = group_scan_ids_by_file(&index, &ids);
        for (_, group) in &groups {
            let offsets: Vec<u64> = group
                .iter()
                .map(|&i| index.resources[i as usize].offset)
                .collect();
            let mut sorted = offsets.clone();
            sorted.sort_unstable();
            assert_eq!(offsets, sorted);
        }
    }

    #[test]
    fn ebo_m40ad_seed_survives_colliding_m12aa_are() {
        let index = fixture_two_modules_shared_are();
        let loaded_map = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded_map);
        assert!(module_entries.contains_key("ebo_m12aa"));
        assert!(module_entries.contains_key("ebo_m40ad"));

        let catalog: HashSet<String> = index.resources.iter().map(|r| r.resref.clone()).collect();
        let (labels, ids) = seed_ids(&index, &catalog, &loaded_map, &module_entries);
        assert!(labels.iter().any(|s| s == "ebo_m40ad"), "labels={labels:?}");
        assert!(!ids.is_empty());
        // At least one seed id must belong to ebo_m40ad's module source.
        assert!(ids.iter().any(|&i| {
            index
                .source(&index.resources[i as usize])
                .module_root
                .as_deref()
                == Some("ebo_m40ad")
        }));
    }

    #[test]
    fn scoped_loaded_keep_per_module_ifo_and_are() {
        let index = fixture_two_modules_shared_are();
        let loaded = scoped_loaded(&index);
        let ifo = ResType::from_extension("ifo").unwrap();
        let are = ResType::from_extension("are").unwrap();
        let a = loaded
            .get(&(Some("ebo_m12aa".into()), "module".into(), ifo))
            .copied();
        let b = loaded
            .get(&(Some("ebo_m40ad".into()), "module".into(), ifo))
            .copied();
        assert!(a.is_some() && b.is_some() && a != b);
        let are_a = loaded
            .get(&(Some("ebo_m12aa".into()), "m12aa".into(), are))
            .copied();
        let are_b = loaded
            .get(&(Some("ebo_m40ad".into()), "m12aa".into(), are))
            .copied();
        assert!(are_a.is_some() && are_b.is_some() && are_a != are_b);
    }

    #[test]
    fn scoped_loaded_mod_beats_rim_same_module() {
        let index = fixture_two_modules_shared_are();
        let loaded = scoped_loaded(&index);
        let ncs = ResType::from_extension("ncs").unwrap();
        let id = *loaded
            .get(&(Some("ebo_m12aa".into()), "local".into(), ncs))
            .unwrap();
        assert_eq!(
            index.source(&index.resources[id as usize]).label,
            "ebo_m12aa.mod"
        );
    }

    #[test]
    fn scoped_loaded_currentgame_rim_beats_mod_for_git() {
        use serde_json::json;
        let git = ResType::from_extension("git").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/end_m01aa.mod",
                "/game/modules/end_m01aa.rim"
            ],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"module-rim","label":"end_m01aa.rim","precedence":50,"module_root":"end_m01aa"}
            ],
            "resources": [
                {"resref":"m01aa","restype":git,"file":0,"offset":0,"size":100,"source":0},
                {"resref":"m01aa","restype":git,"file":1,"offset":0,"size":99,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let git_ty = ResType::from_extension("git").unwrap();
        let id = *loaded
            .get(&(Some("end_m01aa".into()), "m01aa".into(), git_ty))
            .unwrap();
        assert_eq!(
            index.source(&index.resources[id as usize]).label,
            "end_m01aa.rim",
            "CURRENTGAME NAME.rim must beat NAME.mod for IFO/ARE/GIT"
        );
        assert!(is_overshadowed(&index, &loaded, 0));
        assert!(!is_overshadowed(&index, &loaded, 1));
    }

    #[test]
    fn lips_loc_copy_overshadowed_by_module_lip() {
        // Engine LipSync loads VO_ResRef via resman (type 3004). Module .mod
        // beats lips/NAME_loc.mod (which already says so). Graph unused must
        // not list the lips/ copy as a leftover.
        use serde_json::json;
        let lip = ResType::from_extension("lip").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/ebo_m12aa.mod",
                "/game/lips/ebo_m12aa_loc.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"ebo_m12aa.mod","precedence":100,"module_root":"ebo_m12aa"},
                {"kind":"lips","label":"ebo_m12aa_loc.mod","precedence":300,"module_root":"ebo_m12aa"}
            ],
            "resources": [
                {"resref":"nm12aabast01000_","restype":lip,"file":0,"offset":0,"size":496,"source":0},
                {"resref":"nm12aabast01000_","restype":lip,"file":1,"offset":0,"size":421,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        assert!(
            !is_overshadowed(&index, &loaded, 0),
            "module lip is the copy LipSync loads"
        );
        assert!(
            is_overshadowed(&index, &loaded, 1),
            "lips/NAME_loc.mod lip must be overshadowed, not unused"
        );
        assert_eq!(overshadowed_by(&index, &loaded, 1), Some(0));
    }

    #[test]
    fn scoped_loaded_override_beats_module_in_global_and_lookup() {
        let index = fixture_two_modules_shared_are();
        let loaded = scoped_loaded(&index);
        let ncs = ResType::from_extension("ncs").unwrap();
        // Global-scope loaded copy of "shared" is Override.
        let id = *loaded.get(&(None, "shared".into(), ncs)).unwrap();
        assert_eq!(
            index.source(&index.resources[id as usize]).kind,
            kq_index::SourceKind::Override
        );
        // Module still has its own scoped loaded copy.
        assert!(loaded.contains_key(&(Some("ebo_m12aa".into()), "shared".into(), ncs)));
    }

    #[test]
    fn overshadowed_id_set_matches_per_id_lookup() {
        let index = fixture_two_modules_shared_are();
        let loaded = scoped_loaded(&index);
        let set = overshadowed_id_set(&index, &loaded);
        for i in 0..index.resources.len() as u32 {
            assert_eq!(
                set.contains(&i),
                is_overshadowed(&index, &loaded, i),
                "id {i}"
            );
        }
        assert!(!set.is_empty(), "fixture has shadowed rim/module copies");
    }

    #[test]
    fn tokens_ignore_self_and_stars() {
        let (index, loaded, entries) = global_lookup(&["n_bastila", "k_ai_master", "danm13"]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        take_tokens(
            "Tag=n_bastila Script=k_ai_master ****",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "n_bastila",
            Some("utc"),
            &mut out,
            None,
        );
        assert!(out.contains("k_ai_master"));
        assert!(!out.contains("n_bastila"));
        assert!(!out.contains("****"));
    }

    fn fixture_end_m01aa_are_lyt() -> Index {
        let lyt = ResType::from_extension("lyt").unwrap().0;
        let are = ResType::from_extension("are").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let mut index: Index = serde_json::from_value(serde_json::json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/modules/end_m01aa.mod", "/game/data/layouts.bif"],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"chitin","label":"layouts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m01aa","restype":are,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m01aa","restype":lyt,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn are_mentions_same_resref_lyt() {
        let index = fixture_end_m01aa_are_lyt();

        let loaded = scoped_loaded(&index);
        let mut edges = HashMap::new();
        add_are_layout_edges(&index, &loaded, &mut edges);
        let are_id = *loaded
            .get(&(
                Some("end_m01aa".into()),
                "m01aa".into(),
                ResType::from_extension("are").unwrap(),
            ))
            .unwrap();
        assert!(edges.get(&are_id).unwrap().contains("m01aa.lyt"));

        // Tokenize path: scanning ARE text that contains "m01aa.lyt" must keep the token.
        let roots = HashSet::new();
        let mut out = HashSet::new();
        let scope = Some("end_m01aa".into());
        take_tokens(
            "m01aa.lyt",
            &index,
            &loaded,
            &HashMap::new(),
            &scope,
            &roots,
            &catalog_of(&index),
            "m01aa",
            Some("are"),
            &mut out,
            None,
        );
        assert!(out.contains("m01aa"));
    }

    #[test]
    fn build_reaches_chitin_lyt_from_reachable_are() {
        let index = fixture_end_m01aa_are_lyt();
        let loaded = scoped_loaded(&index);
        let lyt_ty = ResType::from_extension("lyt").unwrap();
        let lyt_id = *loaded.get(&(None, "m01aa".into(), lyt_ty)).unwrap();

        let graph = build(&index).expect("build");
        assert!(
            graph.used_ids.contains(&lyt_id),
            "global m01aa.lyt must land in used_ids when end_m01aa is seeded; used_ids={:?}",
            graph.used_ids
        );
    }

    #[test]
    fn isolated_cycle_is_not_reachable() {
        use serde_json::json;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/data/scripts.bif"],
            "sources": [
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"seed","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"dead_a","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"dead_b","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"n_endsol01","restype":ncs,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let module_entries = HashMap::new();
        let mut edges = HashMap::new();
        edges.insert(1, HashSet::from(["dead_b.ncs".into()]));
        edges.insert(2, HashSet::from(["dead_a.ncs".into()]));
        edges.insert(0, HashSet::from(["n_endsol01.ncs".into()]));
        let (seen, _parent) = bfs(&index, &[0], &edges, &loaded, &module_entries);
        assert!(seen.contains(&0));
        assert!(seen.contains(&3));
        assert!(!seen.contains(&1));
        assert!(!seen.contains(&2));
    }

    #[test]
    fn module_folder_enters_git_not_shared_ifo_name() {
        use serde_json::json;
        let git = ResType::from_extension("git").unwrap().0;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let utc = ResType::from_extension("utc").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/end_m01aa.mod",
                "/game/modules/other.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"module-mod","label":"other.mod","precedence":100,"module_root":"other"}
            ],
            "resources": [
                {"resref":"m01aa","restype":git,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"end_trask","restype":utc,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"other_mod_utc","restype":utc,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"orphan","restype":utc,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let mut edges = HashMap::new();
        edges.insert(0, HashSet::from(["end_trask.utc".into()]));
        edges.insert(4, HashSet::from(["other_mod_utc.utc".into()]));
        let seeds = module_entries.get("end_m01aa").cloned().unwrap_or_default();
        let (seen, _parent) = bfs(&index, &seeds, &edges, &loaded, &module_entries);
        assert!(seen.contains(&2)); // end_trask
        assert!(!seen.contains(&3)); // other_mod_utc
        assert!(!seen.contains(&4)); // orphan
    }

    #[test]
    fn reachable_module_scans_unplaced_gff_script_slots() {
        use serde_json::json;
        let git = ResType::from_extension("git").unwrap().0;
        let utt = ResType::from_extension("utt").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/modules/unk_m44ac.mod"],
            "sources": [
                {"kind":"module-mod","label":"unk_m44ac.mod","precedence":100,"module_root":"unk_m44ac"}
            ],
            "resources": [
                {"resref":"m44ac","restype":git,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"unk44_exit","restype":utt,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_punk_exit","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_punk_party","restype":ncs,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let mut edges = HashMap::new();
        // GIT does not mention the trigger. The UTT still names the script.
        edges.insert(1, HashSet::from(["k_punk_exit.ncs".into()]));
        let seeds = module_entries.get("unk_m44ac").cloned().unwrap_or_default();
        let (seen, _parent) = bfs(&index, &seeds, &edges, &loaded, &module_entries);
        assert!(seen.contains(&1), "unplaced UTT is a module GFF entry");
        assert!(seen.contains(&2), "ScriptOnEnter on that UTT is live");
        assert!(!seen.contains(&3), "packed NCS with no GFF slot stays dead");
    }

    #[test]
    fn global_dlg_reaches_module_only_ncs() {
        use serde_json::json;
        let dlg = ResType::from_extension("dlg").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/Override/k_hcan_dialog.dlg",
                "/game/modules/ebo_m12aa.mod"
            ],
            "sources": [
                {"kind":"override","label":"Override","precedence":0,"module_root":null},
                {"kind":"module-mod","label":"ebo_m12aa.mod","precedence":100,"module_root":"ebo_m12aa"}
            ],
            "resources": [
                {"resref":"k_hcan_dialog","restype":dlg,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_swg_gizka01","restype":ncs,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let mut edges = HashMap::new();
        edges.insert(0, HashSet::from(["k_swg_gizka01.ncs".into()]));
        let (seen, _) = bfs(&index, &[0], &edges, &loaded, &module_entries);
        assert!(
            seen.contains(&1),
            "Override DLG Active/Script must reach module-only NCS"
        );
    }

    fn fixture_end_and_m12() -> Index {
        use serde_json::json;
        let ifo = ResType::from_extension("ifo").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/end_m01aa.mod",
                "/game/modules/M12ab.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"end_m01aa.mod","precedence":100,"module_root":"end_m01aa"},
                {"kind":"module-mod","label":"M12ab.mod","precedence":100,"module_root":"m12ab"}
            ],
            "resources": [
                {"resref":"module","restype":ifo,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"module","restype":ifo,"file":1,"offset":0,"size":1,"source":1},
                {"resref":"k_pend_activate","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_pend_activate","restype":ncs,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    fn fixture_tar_rndtalk() -> Index {
        use serde_json::json;
        let dlg = ResType::from_extension("dlg").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                "/game/modules/tar_m03aa.mod",
                "/game/modules/tar_m02aa.mod"
            ],
            "sources": [
                {"kind":"module-mod","label":"tar_m03aa.mod","precedence":100,"module_root":"tar_m03aa"},
                {"kind":"module-mod","label":"tar_m02aa.mod","precedence":100,"module_root":"tar_m02aa"}
            ],
            "resources": [
                {"resref":"tar03_citizen","restype":dlg,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"k_ptar_rndtalk0","restype":ncs,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        index
    }

    #[test]
    fn bfs_case1_ifo_script_in_starting_module() {
        let index = fixture_end_and_m12();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let ifo_id = *loaded
            .get(&(
                Some("end_m01aa".into()),
                "module".into(),
                ResType::from_extension("ifo").unwrap(),
            ))
            .unwrap();
        let mut edges = HashMap::new();
        edges.insert(ifo_id, HashSet::from(["k_pend_activate.ncs".into()]));
        let (seen, parent) = bfs(&index, &[ifo_id], &edges, &loaded, &module_entries);
        let ncs = ResType::from_extension("ncs").unwrap();
        let want = *loaded
            .get(&(Some("end_m01aa".into()), "k_pend_activate".into(), ncs))
            .unwrap();
        let foreign = *loaded
            .get(&(Some("m12ab".into()), "k_pend_activate".into(), ncs))
            .unwrap();
        assert!(seen.contains(&want));
        assert!(!seen.contains(&foreign));
        assert_eq!(parent.get(&want), Some(&ifo_id));
    }

    #[test]
    fn bfs_case5_seed_module_with_colliding_are() {
        let index = fixture_two_modules_shared_are();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let seeds = module_entries.get("ebo_m40ad").cloned().unwrap_or_default();
        assert!(!seeds.is_empty());
        let (seen, _) = bfs(&index, &seeds, &HashMap::new(), &loaded, &module_entries);
        assert!(seeds.iter().all(|id| seen.contains(id)));
    }

    #[test]
    fn bfs_case7_dlg_resolves_same_module_script_not_foreign_loaded() {
        let index = fixture_tar_rndtalk();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let dlg_ty = ResType::from_extension("dlg").unwrap();
        let dlg = *loaded
            .get(&(Some("tar_m03aa".into()), "tar03_citizen".into(), dlg_ty))
            .unwrap();
        let mut edges = HashMap::new();
        edges.insert(dlg, HashSet::from(["k_ptar_rndtalk0.ncs".into()]));
        let (seen, _) = bfs(&index, &[dlg], &edges, &loaded, &module_entries);
        let ncs = ResType::from_extension("ncs").unwrap();
        let local = *loaded
            .get(&(Some("tar_m03aa".into()), "k_ptar_rndtalk0".into(), ncs))
            .unwrap();
        let foreign = *loaded
            .get(&(Some("tar_m02aa".into()), "k_ptar_rndtalk0".into(), ncs))
            .unwrap();
        assert!(seen.contains(&local));
        assert!(!seen.contains(&foreign));
    }

    #[test]
    fn start_new_module_token_counts_without_resref() {
        let (index, loaded, entries) = global_lookup(&[]);
        let roots = HashSet::from(["end_m01aa".into()]);
        let mut out = HashSet::new();
        take_tokens(
            "StartNewModule(\"end_m01aa\")",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "k_sup_gohawk",
            Some("ncs"),
            &mut out,
            None,
        );
        assert!(out.contains("end_m01aa"));
    }

    #[test]
    fn filename_token_counts_as_resref() {
        let (index, loaded, entries) = global_lookup(&["n_bastila"]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        take_tokens(
            "n_bastila.utc",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "k_ai_master",
            Some("ncs"),
            &mut out,
            None,
        );
        assert!(out.contains("n_bastila"));
    }

    #[test]
    fn strref_columns_skip_labels_and_stars() {
        assert!(is_strref_column("name"));
        assert!(is_strref_column("StringRef"));
        assert!(!is_strref_column("label"));
        assert!(!is_strref_column("_row"));
        assert_eq!(parse_strref("****"), None);
        assert_eq!(parse_strref("-1"), None);
        assert_eq!(parse_strref("48012"), Some(48012));
    }

    #[test]
    fn k1_scripts_includes_engine_fallback_seeds() {
        assert!(K1_SCRIPTS.contains(&"k_hen_dialogue01"));
        assert!(K1_SCRIPTS.contains(&"k_hen_retreat"));
        assert!(K1_SCRIPTS.contains(&"k_trg_transfail"));
        assert!(K1_SCRIPTS.contains(&"k_def_pathfail01"));
    }

    #[test]
    fn engine_guis_include_mainmenu_layout() {
        assert!(ENGINE_GUIS.contains(&"mainmenu"));
        assert!(ENGINE_GUIS.contains(&"loadscreen"));
        assert!(ENGINE_GUIS.contains(&"partyselection"));
        // Chargen / sub-panels and resolution tooltips — K1 exe literals.
        assert!(ENGINE_GUIS.contains(&"maincg"));
        assert!(ENGINE_GUIS.contains(&"scriptselect"));
        assert!(ENGINE_GUIS.contains(&"tooltip8x6"));
        assert!(ENGINE_GUIS.contains(&"abchrgen"));
    }

    #[test]
    fn stream_vo_types_are_not_no_assets_skipped() {
        // VO is streamed (`CSWSDialog::SendDialogEntry`) and lip goes through
        // resman type 3004 (`CLIP::LoadLip`); leftover must still list them.
        assert!(!is_asset(ResType::from_extension("wav").unwrap()));
        assert!(!is_asset(ResType::from_extension("lip").unwrap()));
        assert!(is_asset(ResType::from_extension("tpc").unwrap()));
    }

    #[test]
    fn trap_script_field_maps_to_ncs() {
        // See `CSWSTrigger::LoadTrigger` @0058da80 — TrapScript / MineScript → NCS.
        assert_eq!(gff_field_exts("TrapScript"), Some(&["ncs"][..]));
        assert_eq!(gff_field_exts("MineScript"), Some(&["ncs"][..]));
        assert_eq!(gff_field_exts("ScriptHeartbeat"), Some(&["ncs"][..]));
        assert_eq!(gff_field_exts("EquippedRes"), Some(&["uti"][..]));
        assert_eq!(gff_field_exts("InventoryRes"), Some(&["uti"][..]));
        assert_eq!(gff_field_exts("FILL"), Some(&["tpc", "tga", "txi"][..]));
        assert_eq!(gff_field_exts("FONT"), Some(&["tpc", "tga", "txi"][..]));
        assert_eq!(gff_field_exts("IconResRef"), Some(&["tpc", "tga", "txi"][..]));
        assert!(gff_field_exts("Tag").is_none());
        assert!(is_engine_resref_field("TrapScript"));
    }

    #[test]
    fn loadscreens_bmpresref_maps_to_textures() {
        assert_eq!(
            twoda_cell_exts("loadscreens", "BMPResRef"),
            &["tga", "tpc", "txi"][..]
        );
        assert_eq!(twoda_cell_exts("loadscreens", "MusicResRef"), &["wav"][..]);
    }

    #[test]
    fn ambientmusic_resource_and_stinger_map_to_wav() {
        // See `HD0:STREAMMUSIC\%s` — ambientmusic/ambientsound resource cells.
        assert_eq!(twoda_cell_exts("ambientmusic", "Resource"), &["wav"][..]);
        assert_eq!(twoda_cell_exts("ambientmusic", "Stinger1"), &["wav"][..]);
        assert_eq!(twoda_cell_exts("ambientsound", "resource"), &["wav"][..]);
    }

    #[test]
    fn ambienttrack_field_maps_to_streammusic_wav() {
        assert_eq!(gff_field_exts("AmbientTrack"), Some(&["wav", "bmu"][..]));
    }

    #[test]
    fn inventory_icon_resref_from_itemclass_and_variation() {
        assert_eq!(
            inventory_icon_resref("w_BlstrPstl", 1).as_deref(),
            Some("iw_blstrpstl_001")
        );
        assert_eq!(
            inventory_icon_resref("i_Credits", 2).as_deref(),
            Some("ii_credits_002")
        );
        assert_eq!(inventory_icon_resref("****", 1), None);
    }

    #[test]
    fn module_load_screen_texture_name() {
        let mut out = HashSet::new();
        mention_module_load_screen(&mut out, "danm13");
        assert!(out.contains("load_danm13.tpc"));
        assert!(out.contains("load_danm13.tga"));
    }

    #[test]
    fn resolution_gui_textures_include_1024x768back() {
        let mut out = HashSet::new();
        mention_resolution_gui_textures(&mut out);
        assert!(out.contains("1024x768back.tpc"));
        assert!(out.contains("800x600pazaak.tga"));
        assert!(out.contains("1600x1200comp0.txi"));
    }

    #[test]
    fn k2_only_2da_not_seeded_on_k1() {
        // See `Load2DArrays_AlienVO` — TSL only; K1 seeds omit ENGINE_2DAS_K2.
        use serde_json::json;
        let twoda = ResType::from_extension("2da").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/data/2da.bif"],
            "sources": [
                {"kind":"chitin","label":"data/2da.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"appearance","restype":twoda,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"alienvo","restype":twoda,"file":0,"offset":1,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let catalog = catalog_of(&index);
        let loaded = scoped_loaded(&index);
        let entries = module_entry_ids(&index, &loaded);
        let (labels, _) = seed_ids(&index, &catalog, &loaded, &entries);
        assert!(labels.iter().any(|s| s == "appearance"));
        assert!(
            !labels.iter().any(|s| s == "alienvo"),
            "K2-only AlienVO must not seed on K1: {labels:?}"
        );
    }

    #[test]
    fn k2_engine_2das_include_alienvo() {
        assert!(ENGINE_2DAS_K2.contains(&"alienvo"));
    }

    #[test]
    fn nwn_leftover_2das_are_not_engine_seeds() {
        // Present in data/2da.bif on K1 but not Load2DArrays literals in either exe.
        for name in [
            "aliensound",
            "actions",
            "effecticons",
            "dialogtokens",
            "loadhints",
            "xpbaseconst",
            "tutorial_old",
            "cls_atk_3",
            "metamagic",
            "polymorph",
            "gamespyrooms",
        ] {
            assert!(
                !ENGINE_2DAS.contains(&name) && !ENGINE_2DAS_K2.contains(&name),
                "{name} must not be seeded (NWN/rename leftover)"
            );
        }
        // Live names the engine actually opens:
        assert!(ENGINE_2DAS.contains(&"effecticon"));
        assert!(ENGINE_2DAS.contains(&"stringtokens"));
        assert!(ENGINE_2DAS.contains(&"loadscreenhints"));
        assert!(ENGINE_2DAS.contains(&"xptable"));
        assert!(ENGINE_2DAS.contains(&"tutorial"));
    }

    #[test]
    fn execute_script_action_8_mentions_ncs() {
        // See `ExecuteCommandExecuteScript` @00535b70 — arg0 NCS.
        // See `ExecuteCommandGetObjectByTag` @0053c280 — Tag, not a file.
        let (index, loaded, entries) = global_lookup(&[]);
        let roots = HashSet::new();
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let mut missing = HashSet::new();
        let v = serde_json::json!({
            "instructions": [
                {"offset": 13, "op": "CONSTS", "args": ["my_script"]},
                {"offset": 20, "op": "ACTION", "routine": 8, "argc": 3},
                {"offset": 30, "op": "CONSTS", "args": ["some_tag"]},
                {"offset": 40, "op": "ACTION", "routine": 200, "argc": 2},
                {"offset": 50, "op": "RETN"}
            ]
        });
        collect(
            &Decoded::Value(v),
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "k_ai_master",
            Some("ncs"),
            StrRefMode::None,
            &mut mentions,
            &mut strrefs,
            &mut missing,
        );
        assert!(
            mentions.contains("my_script.ncs"),
            "ExecuteScript must mention typed ncs; mentions={mentions:?}"
        );
        assert!(
            !mentions.contains("some_tag") && !mentions.contains("some_tag.ncs"),
            "GetObjectByTag must not edge; mentions={mentions:?}"
        );
    }

    #[test]
    fn play_sound_action_46_mentions_wav() {
        // See `ExecuteCommandPlaySound` @00541170.
        let mut mentions = HashSet::new();
        let roots = HashSet::new();
        apply_ncs_strings(46, false, &["gui_click".into()], &mut mentions, &roots);
        assert!(mentions.contains("gui_click.wav"));
    }

    #[test]
    fn start_new_module_arg0_module_skips_waypoint_tag() {
        // See `ExecuteCommandStartNewModule` @00544390 — arg0 module, arg1 Tag.
        // `apply_ncs_strings` takes declaration order (arg0 first).
        let mut mentions = HashSet::new();
        let roots = HashSet::from(["end_m01aa".into()]);
        apply_ncs_strings(
            509,
            false,
            &[
                "end_m01aa".into(),
                "wp_start".into(),
                "legal".into(),
            ],
            &mut mentions,
            &roots,
        );
        assert!(mentions.contains("end_m01aa"));
        assert!(!mentions.contains("wp_start"));
        assert!(mentions.contains("legal.bik"));
    }

    #[test]
    fn start_new_module_consts_push_order_is_reversed() {
        // Real K1 `k_pebn_exithawk` ACTION 509 CONSTS stream (push / RTL order).
        let mut mentions = HashSet::new();
        let roots = HashSet::from(["danm13".into()]);
        let pushed = vec![
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "k_dan_ebon_hawk_transition".into(),
            "danm13".into(),
        ];
        let declared = ncs_strings_declaration_order(&pushed, 8);
        apply_ncs_strings(509, false, &declared, &mut mentions, &roots);
        assert!(
            mentions.contains("danm13"),
            "module arg0 must resolve; mentions={mentions:?}"
        );
        assert!(
            !mentions.contains("danm13.bik"),
            "module must not be typed as bik; mentions={mentions:?}"
        );
        assert!(!mentions.contains("k_dan_ebon_hawk_transition"));
        assert!(!mentions.contains("k_dan_ebon_hawk_transition.bik"));
    }

    #[test]
    fn start_new_module_ignores_leading_consts_noise() {
        // `k_sup_galaxymap` pushes many CONSTS before the eight StartNewModule args.
        let mut mentions = HashSet::new();
        let roots = HashSet::from(["stunt_00".into()]);
        let pushed = vec![
            "05_2a".into(),
            "05_4a".into(),
            "LIVE_1a".into(),
            "NULL".into(),
            "".into(),
            "".into(),
            "".into(),
            "".into(),
            "08".into(),
            "".into(),
            "STUNT_00".into(),
        ];
        let declared = ncs_strings_declaration_order(&pushed, 8);
        apply_ncs_strings(509, false, &declared, &mut mentions, &roots);
        assert!(
            mentions.contains("stunt_00"),
            "trailing module arg0 must win over leading noise; mentions={mentions:?}"
        );
        assert!(!mentions.contains("05_2a.bik") && !mentions.contains("05_2a"));
    }

    #[test]
    fn action_start_conversation_survives_nested_getpartymember() {
        // Real K1 `k_def_calodef`: CONSTS for ActionStartConversation, then
        // ACTION 577 GetPartyMemberByIndex (object arg), then ACTION 204.
        // Clearing on 577 dropped `k_h_calo.dlg`.
        let (index, loaded, entries) = global_lookup(&[]);
        let roots = HashSet::new();
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let mut missing = HashSet::new();
        let v = serde_json::json!({
            "instructions": [
                {"offset": 1, "op": "CONSTS", "args": [""]},
                {"offset": 2, "op": "CONSTS", "args": [""]},
                {"offset": 3, "op": "CONSTS", "args": [""]},
                {"offset": 4, "op": "CONSTS", "args": [""]},
                {"offset": 5, "op": "CONSTS", "args": [""]},
                {"offset": 6, "op": "CONSTS", "args": [""]},
                {"offset": 7, "op": "CONSTS", "args": ["k_h_calo"]},
                {"offset": 8, "op": "ACTION", "routine": 577, "argc": 1},
                {"offset": 9, "op": "ACTION", "routine": 204, "argc": 11},
                {"offset": 10, "op": "RETN"}
            ]
        });
        collect(
            &Decoded::Value(v),
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "k_def_calodef",
            Some("ncs"),
            StrRefMode::None,
            &mut mentions,
            &mut strrefs,
            &mut missing,
        );
        assert!(
            mentions.contains("k_h_calo.dlg"),
            "nested GetPartyMemberByIndex must not drop DLG; mentions={mentions:?}"
        );
    }

    #[test]
    fn queue_movie_routine_id_differs_k1_k2() {
        // K1 QueueMovie=769; K2 id 769 is SetFadeUntilScript, QueueMovie=806.
        let mut k1 = HashSet::new();
        let mut k2 = HashSet::new();
        let roots = HashSet::new();
        apply_ncs_strings(769, false, &["credits".into()], &mut k1, &roots);
        apply_ncs_strings(769, true, &["credits".into()], &mut k2, &roots);
        assert!(k1.contains("credits.bik"));
        assert!(k2.is_empty());
        apply_ncs_strings(806, true, &["credits".into()], &mut k2, &roots);
        assert!(k2.contains("credits.bik"));
    }

    #[test]
    fn walk_json_does_not_tokenize_object_keys() {
        let (index, loaded, entries) = global_lookup(&["name", "offset", "k_ai_master"]);
        let roots = HashSet::new();
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let mut missing = HashSet::new();
        let v = serde_json::json!({"name": "k_ai_master", "offset": 0});
        walk_json(
            &v,
            &mut Walk {
                index: &index,
                loaded: &loaded,
                module_entries: &entries,
                scope: &None,
                module_roots: &roots,
                catalog: &catalog_of(&index),
                self_ref: "row",
                self_ext: None,
                mode: StrRefMode::None,
                mentions: &mut mentions,
                strrefs: &mut strrefs,
                missing: &mut missing,
            },
            None,
        );
        assert!(!mentions.contains("name"));
        assert!(!mentions.contains("offset"));
        assert!(mentions.contains("k_ai_master"));
    }

    #[test]
    fn conversation_field_allows_self_resref_to_reach_dlg() {
        // Engine: UTC Conversation is a DLG ResRef. BioWare often sets it equal
        // to the template name. Blind self-skip dropped that edge.
        use serde_json::json;
        let utc = ResType::from_extension("utc").unwrap().0;
        let dlg = ResType::from_extension("dlg").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/modules/m.mod"],
            "sources": [
                {"kind":"module-mod","label":"m.mod","precedence":100,"module_root":"m"}
            ],
            "resources": [
                {"resref":"npc01","restype":utc,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"npc01","restype":dlg,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let entries = module_entry_ids(&index, &loaded);
        let roots = HashSet::new();
        let cat = catalog_of(&index);
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let mut missing = HashSet::new();
        walk_gff_value(
            &GffValue::Str("npc01".into()),
            Some("Conversation"),
            &index,
            &loaded,
            &entries,
            &Some("m".into()),
            &roots,
            &cat,
            "npc01",
            Some("utc"),
            StrRefMode::Gff,
            &mut mentions,
            &mut strrefs,
            &mut missing,
        );
        assert!(
            mentions.contains("npc01.dlg"),
            "Conversation must emit typed dlg; mentions={mentions:?}"
        );
    }

    #[test]
    fn vo_resref_field_edges_even_when_matching_dlg_name() {
        // LipSync loads type 3004 with the VO ResRef; rare but legal for the
        // ResRef string to equal the DLG name.
        use serde_json::json;
        let dlg = ResType::from_extension("dlg").unwrap().0;
        let lip = ResType::from_extension("lip").unwrap().0;
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": "/game",
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": ["/game/modules/m.mod", "/game/lips/m_loc.mod"],
            "sources": [
                {"kind":"module-mod","label":"m.mod","precedence":100,"module_root":"m"},
                {"kind":"lips","label":"m_loc.mod","precedence":300,"module_root":null}
            ],
            "resources": [
                {"resref":"chat","restype":dlg,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"chat","restype":lip,"file":1,"offset":0,"size":1,"source":1}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let entries = module_entry_ids(&index, &loaded);
        let roots = HashSet::new();
        let cat = catalog_of(&index);
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let mut missing = HashSet::new();
        walk_gff_value(
            &GffValue::Str("chat".into()),
            Some("VO_ResRef"),
            &index,
            &loaded,
            &entries,
            &Some("m".into()),
            &roots,
            &cat,
            "chat",
            Some("dlg"),
            StrRefMode::Gff,
            &mut mentions,
            &mut strrefs,
            &mut missing,
        );
        assert!(
            mentions.contains("chat.lip") && mentions.contains("chat.wav"),
            "VO_ResRef must emit wav+lip; mentions={mentions:?}"
        );
    }

    #[test]
    fn consider_token_skips_pure_numeric() {
        let (index, loaded, entries) = global_lookup(&["3"]);
        let roots = HashSet::new();
        let mut out = HashSet::new();
        consider_token(
            "3", &index, &loaded, &entries, &None, &roots, &catalog_of(&index), "x", None, &mut out, None, false,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn consider_token_skips_english_prose() {
        let (index, loaded, entries) = global_lookup(&["k_ai_master"]);
        let roots = HashSet::new();
        let cat = catalog_of(&index);
        let mut mentions = HashSet::new();
        let mut missing = HashSet::new();
        consider_token(
            "the",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &cat,
            "k_ai_master",
            Some("dlg"),
            &mut mentions,
            Some(&mut missing),
            false,
        );
        consider_token(
            "because",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &cat,
            "k_ai_master",
            Some("dlg"),
            &mut mentions,
            Some(&mut missing),
            false,
        );
        assert!(mentions.is_empty(), "mentions={mentions:?}");
        assert!(missing.is_empty(), "missing={missing:?}");
    }

    #[test]
    fn locstring_prose_is_not_tokenized() {
        let (index, loaded, entries) = global_lookup(&["k_ai_master"]);
        let roots = HashSet::new();
        let cat = catalog_of(&index);
        let mut mentions = HashSet::new();
        let mut strrefs = HashSet::new();
        let mut missing = HashSet::new();
        let mut substrings = std::collections::BTreeMap::new();
        substrings.insert(0, "the because k_ai_master".into());
        walk_gff_value(
            &GffValue::LocString {
                strref: 42,
                substrings,
            },
            Some("Text"),
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &cat,
            "dlg_foo",
            Some("dlg"),
            StrRefMode::Gff,
            &mut mentions,
            &mut strrefs,
            &mut missing,
        );
        assert!(mentions.is_empty(), "mentions={mentions:?}");
        assert!(missing.is_empty(), "missing={missing:?}");
        assert!(strrefs.contains(&42));
    }

    #[test]
    fn collect_records_missing_resref_shaped_tokens() {
        let (index, loaded, entries) = global_lookup(&[]);
        let roots = HashSet::new();
        let mut mentions = HashSet::new();
        let mut missing = HashSet::new();
        take_tokens(
            "StartNewModule(k_rapidtransit)",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "k_sup_gohawk",
            Some("ncs"),
            &mut mentions,
            Some(&mut missing),
        );
        assert!(mentions.is_empty());
        assert!(missing.contains("k_rapidtransit"));
    }

    #[test]
    fn foreign_module_only_resref_is_missing_not_mention() {
        let index = fixture_two_modules_shared_are();
        let loaded = scoped_loaded(&index);
        let module_entries = module_entry_ids(&index, &loaded);
        let roots = HashSet::new();
        let scope = resource_scope(&index, &index.resources[1]);
        assert_eq!(scope.as_deref(), Some("ebo_m40ad"));
        let mut mentions = HashSet::new();
        let mut missing = HashSet::new();
        // `local` exists only in ebo_m12aa. A resource in ebo_m40ad must
        // not treat the global catalog hit as an existing edge.
        consider_token(
            "local",
            &index,
            &loaded,
            &module_entries,
            &scope,
            &roots,
            &catalog_of(&index),
            "module",
            Some("ifo"),
            &mut mentions,
            Some(&mut missing),
            false,
        );
        assert!(
            mentions.is_empty(),
            "foreign-only resref must not be an existing mention; mentions={mentions:?}"
        );
        assert!(missing.contains("local"));
    }

    #[test]
    fn livegraph_missing_map_survives_build_shape() {
        let (index, loaded, entries) = global_lookup(&[]);
        let roots = HashSet::new();
        let mut mentions = HashSet::new();
        let mut missing = HashSet::new();
        consider_token(
            "nw_o0_death",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog_of(&index),
            "module",
            Some("ifo"),
            &mut mentions,
            Some(&mut missing),
            false,
        );
        let mut missing_by: HashMap<u32, HashSet<String>> = HashMap::new();
        missing_by.entry(42).or_default().extend(missing);
        assert!(mentions.is_empty());
        assert!(missing_by.get(&42).unwrap().contains("nw_o0_death"));
    }

    #[test]
    fn nss_is_not_a_scan_source() {
        let nss = ResType::from_extension("nss").unwrap();
        assert!(!is_scan_source(nss));
        let ncs = ResType::from_extension("ncs").unwrap();
        assert!(is_scan_source(ncs));
    }

    #[test]
    fn nwscript_comments_are_not_compiler_input() {
        let src = "ExecuteScript(\"live\"); // ExecuteScript(\"dead\")\n/* #include \"nope\" */\n#include \"k_inc_generic\"\n";
        let (body, incs) = nss_body_and_includes(src);
        assert!(body.contains("live"));
        assert!(!body.contains("dead"));
        assert!(!body.contains("k_inc_generic"));
        assert_eq!(incs, vec!["k_inc_generic".to_string()]);
        assert!(parse_include_line("#include \"k_inc_switch.nss\"").as_deref() == Some("k_inc_switch"));
    }

    #[test]
    fn include_nss_tree_feeds_ncs_mentions() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let foo = dir.path().join("k_inc_foo.nss");
        let bar = dir.path().join("k_inc_bar.nss");
        let dummy = dir.path().join("k_punk_exit.ncs");
        std::fs::write(&foo, "#include \"k_inc_bar\"\n").unwrap();
        std::fs::write(&bar, "ExecuteScript(\"k_punk_exit\");\n").unwrap();
        std::fs::write(&dummy, [0u8]).unwrap();
        let nss = ResType::from_extension("nss").unwrap().0;
        let ncs = ResType::from_extension("ncs").unwrap().0;
        let foo_sz = std::fs::metadata(&foo).unwrap().len();
        let bar_sz = std::fs::metadata(&bar).unwrap().len();
        let mut index: Index = serde_json::from_value(json!({
            "schema": 3,
            "root": dir.path().to_string_lossy(),
            "kind": "install",
            "game": "k1",
            "fingerprint": 0,
            "files": [
                foo.to_string_lossy(),
                bar.to_string_lossy(),
                dummy.to_string_lossy()
            ],
            "sources": [
                {"kind":"chitin","label":"scripts.bif","precedence":700,"module_root":null}
            ],
            "resources": [
                {"resref":"k_inc_foo","restype":nss,"file":0,"offset":0,"size":foo_sz,"source":0},
                {"resref":"k_inc_bar","restype":nss,"file":1,"offset":0,"size":bar_sz,"source":0},
                {"resref":"k_punk_exit","restype":ncs,"file":2,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();
        let loaded = scoped_loaded(&index);
        let entries = HashMap::new();
        let roots = HashSet::new();
        let catalog = catalog_of(&index);
        let mut mentions = HashSet::new();
        let mut missing = HashSet::new();
        let mut seen = HashSet::new();
        take_nss_tree(
            "#include \"k_inc_foo\"\n",
            &index,
            &loaded,
            &entries,
            &None,
            &roots,
            &catalog,
            "k_ai_master",
            &mut mentions,
            &mut missing,
            &mut seen,
        );
        assert!(
            mentions.contains("k_punk_exit"),
            "nested include ExecuteScript must mention the target; mentions={mentions:?}"
        );
        assert!(!mentions.contains("k_inc_foo"));
        assert!(!mentions.contains("k_inc_bar"));
    }

    #[test]
    fn nwscript_remains_engine_always_seed_name() {
        assert!(ENGINE_ALWAYS.contains(&"nwscript"));
    }

    #[test]
    fn texture_refs_names_are_collected_from_ascii_mdl() {
        let ascii = b"newmodel test\nsetsupermodel test NULL\nbeginmodelgeom test\n  node trimesh mesh\n  {\n    bitmap cm_baremetal\n    lightmap m01aa_lm\n  }\nendmodelgeom test\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mdl");
        std::fs::write(&path, ascii).unwrap();

        let mdl = ResType::from_extension("mdl").unwrap().0;
        let tpc = ResType::from_extension("tpc").unwrap().0;
        let mdx = ResType::from_extension("mdx").unwrap().0;
        let wok = ResType::from_extension("wok").unwrap().0;
        let txi = ResType::from_extension("txi").unwrap().0;
        let mut index: Index = serde_json::from_value(serde_json::json!({
            "schema": 3,
            "root": dir.path().to_string_lossy(),
            "kind": "file",
            "game": "k1",
            "fingerprint": 0,
            "files": [path.to_string_lossy()],
            "sources": [
                {"kind":"loose","label":"test.mdl","precedence":800,"module_root":null}
            ],
            "resources": [
                {"resref":"test","restype":mdl,"file":0,"offset":0,"size":ascii.len(),"source":0},
                {"resref":"test","restype":mdx,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"test","restype":wok,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"cm_baremetal","restype":tpc,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"cm_baremetal","restype":txi,"file":0,"offset":0,"size":1,"source":0},
                {"resref":"m01aa_lm","restype":tpc,"file":0,"offset":0,"size":1,"source":0}
            ],
            "warnings": []
        }))
        .unwrap();
        index.reindex();

        let graph = build(&index).unwrap();
        let mentions: HashSet<&String> = graph.edges.values().flatten().collect();
        assert!(
            mentions.iter().any(|s| s.as_str() == "cm_baremetal.tpc"),
            "edges={:?}",
            graph.edges
        );
        assert!(
            mentions.iter().any(|s| s.as_str() == "m01aa_lm.tpc"),
            "edges={:?}",
            graph.edges
        );
        assert!(
            mentions.iter().any(|s| s.as_str() == "test.mdx"),
            "MDL must edge same-ResRef MDX; edges={:?}",
            graph.edges
        );
        assert!(
            mentions.iter().any(|s| s.as_str() == "test.wok"),
            "MDL must edge same-ResRef WOK; edges={:?}",
            graph.edges
        );
        assert!(
            mentions.iter().any(|s| s.as_str() == "cm_baremetal.txi"),
            "texture open must edge TXI; edges={:?}",
            graph.edges
        );
    }
}
