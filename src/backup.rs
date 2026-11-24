//! Export and import of the personal data of the user: what cannot be got back from the game.
//!
//! What the game sends at login (items, characters, achievements, quests) comes again, so it is
//! not kept here.  What is kept here is what only this program knows: what was marked on the map,
//! the chests found and the ground walked over, what is done of the objects and the puzzles, the
//! goals of the planner, what was learned of the maps (the pins put by hand and the places found
//! by playing), the choices on the map, and some settings.  Not the key of the connection, not
//! the caches, not the logs.
//!
//! A backup is a zip file with a `manifest.json` and the files as they are in the data folder
//! (`profiles/markers/<uid>.json` ...).  Importing is done in two steps, because the program keeps
//! most of this in memory and saves it when it ends, which would undo an import made while it
//! runs: the file is *staged* (copied, with what was chosen), and the next start of the program
//! *applies* it before anything is read.  Either by joining what is there with what is in the
//! backup (marks the later of the two, places walked added together ...) or by replacing it.  What
//! is replaced is first copied to `import-backup/<date>`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The version of the format of the file; a file of a newer one is not read.
pub const FORMAT: u32 = 1;
const MANIFEST: &str = "manifest.json";
const SETTINGS_ENTRY: &str = "settings.json";
const PENDING_ZIP: &str = "import-pending.zip";
const PENDING_PLAN: &str = "import-pending.json";

/// A part of the personal data that is chosen as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Section {
    /// What is marked as collected on the map (per account).
    Marks,
    /// The chests seen and opened, the ground walked over, the chests of the other maps (per account).
    Chests,
    /// What is done of the objects and the puzzles (per account).
    Objects,
    /// Which stories of the quest list are done (per account).
    Quests,
    /// The goals of the planner.
    Planner,
    /// The pins put by hand on the maps of the places under the ground.
    Calibrations,
    /// What was learned by playing: places of the maps, teleports, heights, chests found.
    Learned,
    /// The choices on the map (the kinds shown, what can be marked) and of the events.
    MapChoice,
    /// Settings of the program, of the map over the game, and the marks of the achievement tracker.
    Settings,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::Marks,
        Section::Chests,
        Section::Objects,
        Section::Quests,
        Section::Planner,
        Section::Calibrations,
        Section::Learned,
        Section::MapChoice,
        Section::Settings,
    ];

    pub fn per_account(self) -> bool {
        matches!(self, Section::Marks | Section::Chests | Section::Objects | Section::Quests)
    }

    pub fn title(self, russian: bool) -> &'static str {
        match (self, russian) {
            (Section::Marks, false) => "Marks on the map",
            (Section::Marks, true) => "Отметки на карте",
            (Section::Chests, false) => "Chests and walked ground",
            (Section::Chests, true) => "Сундуки и пройденные места",
            (Section::Objects, false) => "Objects and puzzles done",
            (Section::Objects, true) => "Выполненные объекты и загадки",
            (Section::Quests, false) => "Quests done",
            (Section::Quests, true) => "Выполненные квесты",
            (Section::Planner, false) => "Planner goals",
            (Section::Planner, true) => "Цели планировщика",
            (Section::Calibrations, false) => "Pins of the maps (by hand)",
            (Section::Calibrations, true) => "Привязки карт (вручную)",
            (Section::Learned, false) => "Learned by playing",
            (Section::Learned, true) => "Выученное при игре",
            (Section::MapChoice, false) => "Choices on the map",
            (Section::MapChoice, true) => "Выбор на карте",
            (Section::Settings, false) => "Settings and achievement marks",
            (Section::Settings, true) => "Настройки и отметки достижений",
        }
    }

    pub fn about(self, russian: bool) -> &'static str {
        match (self, russian) {
            (Section::Marks, false) => "What you marked as collected (plants, oculi, enemies ...) and when.",
            (Section::Marks, true) => "Что вы отметили собранным (растения, окули, враги ...) и когда.",
            (Section::Chests, false) => "Chests seen and opened, the ground walked over (the map's \"most likely opened\"), chests of other maps.",
            (Section::Chests, true) => "Увиденные и открытые сундуки, пройденные места («вероятно открыто»), сундуки других карт.",
            (Section::Objects, false) => "What the traffic said is done: seelies, puzzles, challenges.",
            (Section::Objects, true) => "Что по трафику выполнено: сили, загадки, испытания.",
            (Section::Quests, false) => "Which stories of the quest list are done (the game also says it at login).",
            (Section::Quests, true) => "Какие истории из списка квестов выполнены (игра сообщает это и при входе).",
            (Section::Planner, false) => "The characters and weapons you want to take to a level, and the floating window of materials.",
            (Section::Planner, true) => "Персонажи и оружие, которые вы хотите прокачать, и плавающее окно материалов.",
            (Section::Calibrations, false) => "Where you pinned waypoints on the maps of places under the ground, so the player is shown right there.",
            (Section::Calibrations, true) => "Где вы привязали точки телепорта на картах подземных мест, чтобы игрок показывался верно.",
            (Section::Learned, false) => "Places of the maps, teleport points, heights, chests found and which objects are which kind: all made by playing.",
            (Section::Learned, true) => "Места на картах, точки телепорта, высоты, найденные сундуки и виды объектов: всё накоплено игрой.",
            (Section::MapChoice, false) => "Which kinds of marker are shown, which can be marked, the radar radius; the timeline of events.",
            (Section::MapChoice, true) => "Какие виды меток показаны, какие можно отмечать, радиус радара; лента событий.",
            (Section::Settings, false) => "Export settings, popups, tray, the map over the game (not where its windows are), and the tracker's marks.",
            (Section::Settings, true) => "Настройки экспорта, всплывающие окна, трей, карта поверх игры (без положения её окон) и отметки трекера.",
        }
    }

    /// The files of the section, from the data folder (`profiles/...` is in the folder of the
    /// profiles); for a section of an account, those of the account.
    fn files(self, uid: u32) -> Vec<String> {
        match self {
            Section::Marks => vec![format!("profiles/markers/{uid}.json")],
            Section::Chests => vec![
                format!("profiles/chests/{uid}.json"),
                format!("profiles/chests/{uid}.coverage"),
                format!("profiles/chest_marks/{uid}.json"),
            ],
            Section::Objects => vec![format!("profiles/object_marks/{uid}.json"), format!("profiles/puzzle_marks/{uid}.json")],
            Section::Quests => vec![format!("profiles/quest_status/{uid}.json")],
            Section::Planner => vec!["planner.json".to_owned()],
            Section::Calibrations => vec!["scene_calibrations.json".to_owned()],
            Section::Learned => ["scene_maps.json", "teleport_points.json", "floor_heights.json", "chest_locations.json", "profiles/object_gadgets.json"]
                .map(str::to_owned)
                .to_vec(),
            Section::MapChoice => vec!["map_filters.json".to_owned(), "events_prefs.json".to_owned()],
            Section::Settings => Vec::new(),
        }
    }
}

/// Where the data of the program is: its folder, and the folder of the profiles (which may be
/// elsewhere).
#[derive(Clone, Debug)]
pub struct Roots {
    pub data: PathBuf,
    pub profiles: PathBuf,
}

impl Roots {
    /// The folders as the program has them.
    pub fn current() -> Result<Self> {
        let data = eframe::storage_dir(crate::APP_ID).context("the data folder was not found")?;
        Ok(Self { profiles: crate::profiles::profiles_dir()?, data })
    }

    fn path(&self, rel: &str) -> PathBuf {
        match rel.strip_prefix("profiles/") {
            Some(rest) => self.profiles.join(rest),
            None => self.data.join(rel),
        }
    }

    /// The accounts that have anything kept for them.
    pub fn accounts(&self) -> Vec<u32> {
        let mut uids = BTreeSet::new();
        for folder in ["", "markers", "chests", "chest_marks", "object_marks", "puzzle_marks", "quest_status"] {
            let Ok(entries) = std::fs::read_dir(self.profiles.join(folder)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let known = matches!(path.extension().and_then(|e| e.to_str()), Some("json" | "coverage"));
                if let (true, Some(uid)) = (known, path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u32>().ok())) {
                    uids.insert(uid);
                }
            }
        }
        uids.into_iter().collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub program: String,
    pub version: String,
    pub created: String,
    /// The sections that have files in it.
    pub sections: BTreeSet<Section>,
    /// The accounts that have files in it, with their names where they are known.
    pub accounts: BTreeMap<u32, String>,
}

/// What is chosen for an export.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub sections: BTreeSet<Section>,
    pub accounts: BTreeSet<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub files: usize,
    pub bytes: u64,
}

/// The settings that go with a backup: what is in the memory of the program, not in a file of
/// its own.  What is only of this computer (where windows are, the path of the launcher, the
/// keys of the log) is left out.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PortableSettings {
    pub export_settings: Option<crate::player_data::ExportSettings>,
    pub auto_start_capture: Option<bool>,
    pub close_to_tray: Option<bool>,
    pub show_capture_popup: Option<bool>,
    pub show_changes_popup: Option<bool>,
    pub minimize_on_launch: Option<bool>,
    pub hide_launcher: Option<bool>,
    pub language: Option<crate::achievement_data::Language>,
    /// The overlay of the map (without the places and sizes of its windows).
    pub overlay: Option<crate::map_overlay::MapOverlaySettings>,
    /// The marks of the achievement tracker, of each account.
    pub achievements: BTreeMap<u32, BTreeSet<u32>>,
}

/// How a staged backup is joined to what is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// What is there stays, and what the backup has more or later is added to it.
    Merge,
    /// What is in the backup takes the place of what is there (it is kept in `import-backup`).
    Replace,
}

/// What is chosen for an import: what, how, and which account of the backup goes to which account
/// here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub mode: Mode,
    pub sections: BTreeSet<Section>,
    /// The account of the backup to the account it is put to.
    pub accounts: BTreeMap<u32, u32>,
    /// The program that put it by: the import waits for it to end, as it saves its data (over
    /// what is imported) when it does.  Set by `stage`.
    #[serde(default)]
    pub owner_pid: u32,
}

/// Waits for a process to end (at most `limit_ms`).
fn wait_for_exit(pid: u32, limit_ms: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    // SAFETY: a handle opened here, waited on and closed here.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(handle, limit_ms);
            let _ = CloseHandle(handle);
        }
    }
}

fn json_object() -> Value {
    Value::Object(serde_json::Map::new())
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut name = path.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    name.push(".tmp");
    let temp = path.with_file_name(name);
    std::fs::write(&temp, bytes).with_context(|| format!("unable to write {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("unable to replace {}", path.display()))
}

/// Makes the backup file.
pub fn export(roots: &Roots, choice: &Choice, settings: Option<&PortableSettings>, out: &Path) -> Result<Summary> {
    let mut temp_name = out.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    temp_name.push(".tmp");
    let temp = out.with_file_name(temp_name);
    let file = std::fs::File::create(&temp).with_context(|| format!("unable to make {}", temp.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let names: BTreeMap<u32, String> = crate::profiles::list_in(&roots.profiles)
        .into_iter()
        .map(|profile| (profile.uid, profile.nickname.unwrap_or_default()))
        .collect();
    let mut summary = Summary::default();
    let mut sections = BTreeSet::new();
    let mut accounts = BTreeMap::new();
    let mut add = |zip: &mut zip::ZipWriter<std::fs::File>, rel: &str, bytes: &[u8]| -> Result<()> {
        zip.start_file(rel, options)?;
        zip.write_all(bytes)?;
        summary.files += 1;
        summary.bytes += bytes.len() as u64;
        Ok(())
    };
    for section in &choice.sections {
        if *section == Section::Settings {
            if let Some(settings) = settings {
                add(&mut zip, SETTINGS_ENTRY, &serde_json::to_vec_pretty(&portable_for(settings, &choice.accounts))?)?;
                sections.insert(*section);
            }
            continue;
        }
        let uids: Vec<u32> = if section.per_account() { choice.accounts.iter().copied().collect() } else { vec![0] };
        for uid in uids {
            for rel in section.files(uid) {
                let Ok(bytes) = std::fs::read(roots.path(&rel)) else {
                    continue;
                };
                add(&mut zip, &rel, &bytes)?;
                sections.insert(*section);
                if section.per_account() {
                    accounts.insert(uid, names.get(&uid).cloned().unwrap_or_default());
                }
            }
        }
    }
    let manifest = Manifest {
        format: FORMAT,
        program: "Aaru".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        created: chrono::Local::now().to_rfc3339(),
        sections,
        accounts,
    };
    zip.start_file(MANIFEST, options)?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    zip.finish()?;
    std::fs::rename(&temp, out).with_context(|| format!("unable to make {}", out.display()))?;
    Ok(summary)
}

/// The settings with only the accounts that were chosen.
fn portable_for(settings: &PortableSettings, accounts: &BTreeSet<u32>) -> PortableSettings {
    let mut settings = settings.clone();
    settings.achievements.retain(|uid, _| accounts.contains(uid));
    if let Some(overlay) = &mut settings.overlay {
        let usual = crate::map_overlay::MapOverlaySettings::default();
        overlay.pos = None;
        overlay.nearby_pos = None;
        overlay.size = usual.size;
    }
    settings
}

/// What a backup file has.
#[derive(Clone, Debug)]
pub struct Inspected {
    pub manifest: Manifest,
    pub settings: Option<PortableSettings>,
}

/// Reads what a backup file has, and checks that it is one that this program can use.
pub fn inspect(path: &Path) -> Result<Inspected> {
    let file = std::fs::File::open(path).with_context(|| format!("unable to open {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("this is not a backup file (it is not a zip)")?;
    let manifest: Manifest = {
        let mut entry = zip.by_name(MANIFEST).context("this is not a backup of Aaru (it has no manifest)")?;
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        serde_json::from_str(&text).context("the manifest of the backup can not be read")?
    };
    if manifest.format > FORMAT {
        bail!("the backup is of a newer version of the program (format {}); update the program to read it", manifest.format);
    }
    let settings = match zip.by_name(SETTINGS_ENTRY) {
        Ok(mut entry) => {
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            serde_json::from_str(&text).ok()
        }
        Err(_) => None,
    };
    Ok(Inspected { manifest, settings })
}

/// Puts a backup file by, with what is chosen to be done of it, for the next start of the
/// program to apply.
pub fn stage(roots: &Roots, archive: &Path, plan: &Plan) -> Result<()> {
    inspect(archive)?;
    std::fs::create_dir_all(&roots.data)?;
    std::fs::copy(archive, roots.data.join(PENDING_ZIP)).context("unable to put the backup by")?;
    let plan = Plan { owner_pid: std::process::id(), ..plan.clone() };
    write_atomic(&roots.data.join(PENDING_PLAN), &serde_json::to_vec_pretty(&plan)?)
}

/// Whether a backup is staged.
pub fn pending(roots: &Roots) -> bool {
    roots.data.join(PENDING_ZIP).exists() && roots.data.join(PENDING_PLAN).exists()
}

/// Throws away a staged backup.
pub fn unstage(roots: &Roots) {
    let _ = std::fs::remove_file(roots.data.join(PENDING_ZIP));
    let _ = std::fs::remove_file(roots.data.join(PENDING_PLAN));
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub files: usize,
    /// Where what was replaced was put.
    pub backup: Option<PathBuf>,
}

/// Applies the staged backup, if there is one.  Run when the program starts, before anything of
/// the data is read.
pub fn apply_pending(roots: &Roots) -> Result<Option<Applied>> {
    if !pending(roots) {
        return Ok(None);
    }
    let result = (|| {
        let plan: Plan = serde_json::from_slice(&std::fs::read(roots.data.join(PENDING_PLAN))?)?;
        if plan.owner_pid != 0 && plan.owner_pid != std::process::id() {
            wait_for_exit(plan.owner_pid, 30_000);
        }
        apply(roots, &roots.data.join(PENDING_ZIP), &plan)
    })();
    // Done or not, it is not tried again at every start.
    unstage(roots);
    result.map(Some)
}

/// Applies a backup file with a plan.
pub fn apply(roots: &Roots, archive: &Path, plan: &Plan) -> Result<Applied> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let backup_dir = roots.data.join("import-backup").join(now_stamp());
    let mut applied = Applied::default();
    // What is taken from the archive: only what is known (the names of the entries are never used
    // as paths: the paths are made here).
    let mut jobs: Vec<(Section, String, String)> = Vec::new();
    for section in &plan.sections {
        if *section == Section::Settings {
            continue;
        }
        if section.per_account() {
            for (from, to) in &plan.accounts {
                for (source, target) in section.files(*from).into_iter().zip(section.files(*to)) {
                    jobs.push((*section, source, target));
                }
            }
        } else {
            for rel in section.files(0) {
                jobs.push((*section, rel.clone(), rel));
            }
        }
    }
    // The overlay of the map is a file of its own that the map reads at start: the places and the
    // size of its windows stay as they are here.
    if plan.sections.contains(&Section::Settings)
        && let Ok(mut entry) = zip.by_name(SETTINGS_ENTRY)
    {
        let mut text = String::new();
        entry.read_to_string(&mut text)?;
        drop(entry);
        if let Some(Value::Object(mut incoming)) = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("overlay").cloned()) {
            for own in ["pos", "size", "nearby_pos"] {
                incoming.remove(own);
            }
            let path = roots.data.join("map_overlay.json");
            let existing = std::fs::read(&path).ok();
            let mut now: Value = existing.as_deref().and_then(|b| serde_json::from_slice(b).ok()).unwrap_or_else(|| json_object());
            if let Some(object) = now.as_object_mut() {
                object.extend(incoming);
                if let Some(existing) = &existing {
                    write_atomic(&backup_dir.join("map_overlay.json"), existing)?;
                    applied.backup = Some(backup_dir.clone());
                }
                write_atomic(&path, &serde_json::to_vec(&now)?)?;
                applied.files += 1;
            }
        }
    }
    for (section, source, target) in jobs {
        let incoming = match zip.by_name(&source) {
            Ok(mut entry) => {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                bytes
            }
            Err(_) => continue,
        };
        let target_path = roots.path(&target);
        let existing = std::fs::read(&target_path).ok();
        let merged = match (plan.mode, &existing) {
            (Mode::Replace, _) | (_, None) => incoming,
            (Mode::Merge, Some(existing)) => merge_file(section, &target, existing, &incoming)
                .with_context(|| format!("unable to join {target}"))?,
        };
        if let Some(existing) = &existing {
            let backup = backup_dir.join(&target);
            write_atomic(&backup, existing)?;
            applied.backup = Some(backup_dir.clone());
        }
        write_atomic(&target_path, &merged)?;
        applied.files += 1;
    }
    Ok(applied)
}

/// How two files of one kind are joined.
fn merge_file(section: Section, rel: &str, existing: &[u8], incoming: &[u8]) -> Result<Vec<u8>> {
    // The settings of the map and of the events are not joined: the backup's are the ones.
    if section == Section::MapChoice {
        return Ok(incoming.to_vec());
    }
    if rel.ends_with(".coverage") {
        let mut walked = crate::coverage::Coverage::from_bytes(existing).ok_or_else(|| anyhow!("the walked ground can not be read"))?;
        let other = crate::coverage::Coverage::from_bytes(incoming).ok_or_else(|| anyhow!("the walked ground in the backup can not be read"))?;
        walked.merge(&other);
        return Ok(walked.to_bytes());
    }
    let mut mine: Value = serde_json::from_slice(existing)?;
    let theirs: Value = serde_json::from_slice(incoming)?;
    match rel {
        r if r.ends_with("profiles/chests") || r.starts_with("profiles/chests/") => join_chests(&mut mine, &theirs),
        "chest_locations.json" => join_chest_places(&mut mine, &theirs),
        "profiles/object_gadgets.json" => join_gadgets(&mut mine, &theirs),
        "planner.json" => join_planner(&mut mine, &theirs),
        "scene_calibrations.json" => join_pins(&mut mine, &theirs),
        "scene_maps.json" | "teleport_points.json" | "floor_heights.json" => add_missing(&mut mine, &theirs),
        // Marks, states, quests: the same number of the same thing, the larger one (the later).
        _ => larger_numbers(&mut mine, &theirs),
    }
    Ok(serde_json::to_vec(&mine)?)
}

/// Objects joined, and numbers where both have one: the larger.
fn larger_numbers(mine: &mut Value, theirs: &Value) {
    match (mine, theirs) {
        (Value::Object(mine), Value::Object(theirs)) => {
            for (key, value) in theirs {
                match mine.get_mut(key) {
                    Some(have) => larger_numbers(have, value),
                    None => {
                        mine.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (Value::Number(mine), Value::Number(theirs)) => {
            if theirs.as_f64() > mine.as_f64() {
                *mine = theirs.clone();
            }
        }
        _ => (),
    }
}

/// Objects joined; what is there stays.
fn add_missing(mine: &mut Value, theirs: &Value) {
    if let (Value::Object(mine), Value::Object(theirs)) = (mine, theirs) {
        for (key, value) in theirs {
            match mine.get_mut(key) {
                Some(have) => add_missing(have, value),
                None => {
                    mine.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

fn chest_key(chest: &Value) -> Option<(u64, u64)> {
    Some((chest.get("group")?.as_u64()?, chest.get("config")?.as_u64()?))
}

/// The earlier or later of two times kept as text (with their offsets).
fn pick_time<'a>(a: Option<&'a Value>, b: Option<&'a Value>, later: bool) -> Option<&'a Value> {
    let (Some(first), Some(second)) = (a.filter(|v| !v.is_null()), b.filter(|v| !v.is_null())) else {
        return a.filter(|v| !v.is_null()).or(b.filter(|v| !v.is_null()));
    };
    let parse = |value: &Value| value.as_str().and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
    match (parse(first), parse(second)) {
        (Some(x), Some(y)) => Some(if (y > x) == later { second } else { first }),
        _ => Some(first),
    }
}

/// The chests of an account: each seen the first time and opened the last time of the two.
fn join_chests(mine: &mut Value, theirs: &Value) {
    let (Some(have), Some(more)) = (mine.get_mut("chests").and_then(Value::as_array_mut), theirs.get("chests").and_then(Value::as_array)) else {
        return;
    };
    for chest in more {
        let Some(key) = chest_key(chest) else {
            continue;
        };
        match have.iter_mut().find(|known| chest_key(known) == Some(key)) {
            Some(known) => {
                let seen = pick_time(known.get("seen_at"), chest.get("seen_at"), false).cloned();
                let opened = pick_time(known.get("opened_at"), chest.get("opened_at"), true).cloned();
                if let Some(object) = known.as_object_mut() {
                    if let Some(seen) = seen {
                        object.insert("seen_at".to_owned(), seen);
                    }
                    if let Some(opened) = opened {
                        object.insert("opened_at".to_owned(), opened);
                    }
                }
            }
            None => have.push(chest.clone()),
        }
    }
}

/// The places of chests found: what is there stays, the others are added.
fn join_chest_places(mine: &mut Value, theirs: &Value) {
    let (Some(have), Some(more)) = (mine.get_mut("chests").and_then(Value::as_array_mut), theirs.get("chests").and_then(Value::as_array)) else {
        return;
    };
    for chest in more {
        if let Some(key) = chest_key(chest)
            && !have.iter().any(|known| chest_key(known) == Some(key))
        {
            have.push(chest.clone());
        }
    }
}

/// What kind of marker an object is: the one learned from more sightings.
fn join_gadgets(mine: &mut Value, theirs: &Value) {
    let (Some(have), Some(more)) = (mine.as_object_mut(), theirs.as_object()) else {
        return;
    };
    for (gadget, learned) in more {
        let count = |value: &Value| value.get("n").and_then(Value::as_u64).unwrap_or(0);
        match have.get(gadget) {
            Some(known) if count(known) >= count(learned) => (),
            _ => {
                have.insert(gadget.clone(), learned.clone());
            }
        }
    }
}

/// The goals of the planner: those that are not there yet are added.  The window of materials and
/// the other settings of the planner stay as they are.
fn join_planner(mine: &mut Value, theirs: &Value) {
    let (Some(have), Some(more)) = (mine.get_mut("goals").and_then(Value::as_array_mut), theirs.get("goals").and_then(Value::as_array)) else {
        return;
    };
    for goal in more {
        if !have.contains(goal) {
            have.push(goal.clone());
        }
    }
}

/// The pins of the maps: of a place that is not there, all; of one that is (on the same map), the
/// pins that are not there yet.
fn join_pins(mine: &mut Value, theirs: &Value) {
    let (Some(have), Some(more)) = (mine.as_object_mut(), theirs.as_object()) else {
        return;
    };
    for (scene, pins) in more {
        match have.get_mut(scene) {
            None => {
                have.insert(scene.clone(), pins.clone());
            }
            Some(known) if known.get("map") == pins.get("map") => {
                if let (Some(points), Some(extra)) = (known.get_mut("points").and_then(Value::as_array_mut), pins.get("points").and_then(Value::as_array)) {
                    for point in extra {
                        if !points.contains(point) {
                            points.push(point.clone());
                        }
                    }
                }
            }
            Some(_) => (),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn roots(dir: &Path) -> Roots {
        Roots { data: dir.join("data"), profiles: dir.join("data").join("profiles") }
    }

    fn write(roots: &Roots, rel: &str, value: &Value) {
        write_atomic(&roots.path(rel), &serde_json::to_vec(value).expect("json")).expect("written");
    }

    fn read(roots: &Roots, rel: &str) -> Value {
        serde_json::from_slice(&std::fs::read(roots.path(rel)).expect("there")).expect("json")
    }

    fn everything() -> Choice {
        Choice { sections: Section::ALL.into_iter().collect(), accounts: BTreeSet::from([111, 222]) }
    }

    #[test]
    fn marks_are_joined_by_the_later_time_and_nothing_is_lost() {
        let mut mine = json!({"collected": {"1": 100, "2": 500}});
        larger_numbers(&mut mine, &json!({"collected": {"1": 300, "2": 200, "3": 50}}));
        assert_eq!(mine, json!({"collected": {"1": 300, "2": 500, "3": 50}}));
    }

    #[test]
    fn chests_are_seen_the_first_time_and_opened_the_last() {
        let mut mine = json!({"chests": [
            {"group": 1, "config": 1, "seen_at": "2026-11-10T12:00:00+08:00", "opened_at": null},
            {"group": 2, "config": 2, "seen_at": "2026-11-10T12:00:00+08:00", "opened_at": "2026-11-10T13:00:00+08:00"}]});
        let theirs = json!({"chests": [
            {"group": 1, "config": 1, "seen_at": "2026-11-09T04:00:00+00:00", "opened_at": "2026-11-09T05:00:00+00:00"},
            {"group": 2, "config": 2, "seen_at": "2026-11-11T12:00:00+08:00", "opened_at": "2026-11-10T12:30:00+08:00"},
            {"group": 3, "config": 3, "seen_at": "2026-11-12T12:00:00+08:00"}]});
        join_chests(&mut mine, &theirs);
        let chests = mine["chests"].as_array().expect("array");
        assert_eq!(chests.len(), 3);
        // Seen earlier elsewhere (other offset): taken; opened there: taken.
        assert_eq!(chests[0]["seen_at"], "2026-11-09T04:00:00+00:00");
        assert_eq!(chests[0]["opened_at"], "2026-11-09T05:00:00+00:00");
        // Opened later here: stays.
        assert_eq!(chests[1]["opened_at"], "2026-11-10T13:00:00+08:00");
        assert_eq!(chests[1]["seen_at"], "2026-11-10T12:00:00+08:00");
    }

    #[test]
    fn goals_pins_and_learned_things_are_added_not_doubled() {
        let mut planner = json!({"goals": [{"a": 1}], "percent_mode": true});
        join_planner(&mut planner, &json!({"goals": [{"a": 1}, {"b": 2}], "percent_mode": false}));
        assert_eq!(planner, json!({"goals": [{"a": 1}, {"b": 2}], "percent_mode": true}));
        let mut pins = json!({"103": {"map": 37, "points": [{"m": 1}]}, "5": {"map": 7, "points": []}});
        join_pins(&mut pins, &json!({"103": {"map": 37, "points": [{"m": 1}, {"m": 2}]}, "5": {"map": 8, "points": [{"m": 9}]}, "6": {"map": 9, "points": []}}));
        assert_eq!(pins["103"]["points"], json!([{"m": 1}, {"m": 2}]));
        assert_eq!(pins["5"]["map"], 7, "another map for the place: what is there stays");
        assert!(pins.get("6").is_some());
        let mut gadgets = json!({"1": {"n": 5}, "2": {"n": 1}});
        join_gadgets(&mut gadgets, &json!({"1": {"n": 2}, "2": {"n": 9}, "3": {"n": 1}}));
        assert_eq!(gadgets, json!({"1": {"n": 5}, "2": {"n": 9}, "3": {"n": 1}}));
        let mut heights = json!({"1": {"samples": [1.0]}});
        add_missing(&mut heights, &json!({"1": {"samples": [9.0]}, "2": {"samples": [2.0]}}));
        assert_eq!(heights, json!({"1": {"samples": [1.0]}, "2": {"samples": [2.0]}}));
    }

    #[test]
    fn a_backup_is_made_inspected_and_applied_by_joining() {
        let from = tempfile::tempdir().expect("a folder");
        let source = roots(from.path());
        write(&source, "profiles/markers/111.json", &json!({"collected": {"1": 100, "2": 900}}));
        write(&source, "profiles/markers/222.json", &json!({"collected": {"7": 70}}));
        write(&source, "profiles/object_marks/111.json", &json!({"5": 0}));
        write(&source, "planner.json", &json!({"goals": [{"Weapon": 1}]}));
        write(&source, "map_filters.json", &json!({"enabled": [2, 3]}));
        // A key that must never go into a backup, and a profile (the game says it at login).
        write(&source, "session_key.json", &json!({"key": "secret"}));
        write(&source, "profiles/111.json", &json!({"uid": 111, "nickname": "Traveler"}));
        let archive = from.path().join("backup.zip");
        let settings = PortableSettings { auto_start_capture: Some(true), achievements: BTreeMap::from([(111, BTreeSet::from([80001])), (222, BTreeSet::from([2]))]), ..Default::default() };
        let summary = export(&source, &everything(), Some(&settings), &archive).expect("exported");
        assert_eq!(summary.files, 6);

        let found = inspect(&archive).expect("inspected");
        assert_eq!(found.manifest.format, FORMAT);
        assert_eq!(found.manifest.accounts.keys().copied().collect::<Vec<_>>(), vec![111, 222]);
        assert_eq!(found.manifest.accounts[&111], "Traveler");
        assert!(found.manifest.sections.contains(&Section::Settings) && !found.manifest.sections.contains(&Section::Quests));
        assert_eq!(found.settings.expect("settings").achievements.len(), 2);
        // Only what is known is in it.
        let names: Vec<String> = zip::ZipArchive::new(std::fs::File::open(&archive).expect("open")).expect("zip").file_names().map(str::to_owned).collect();
        assert!(!names.iter().any(|name| name.contains("session") || name.contains("profiles/111.json")), "{names:?}");

        // To another computer: the account 111 there is 999, and it has marks already.
        let to = tempfile::tempdir().expect("a folder");
        let target = roots(to.path());
        write(&target, "profiles/markers/999.json", &json!({"collected": {"1": 500, "3": 30}}));
        write(&target, "planner.json", &json!({"goals": [{"Weapon": 2}], "percent_mode": true}));
        let plan = Plan { mode: Mode::Merge, sections: Section::ALL.into_iter().collect(), accounts: BTreeMap::from([(111, 999)]), owner_pid: 0 };
        stage(&target, &archive, &plan).expect("staged");
        assert!(pending(&target));
        let applied = apply_pending(&target).expect("applied").expect("there was one");
        assert!(applied.files >= 4);
        assert!(!pending(&target), "not applied again");
        assert_eq!(read(&target, "profiles/markers/999.json"), json!({"collected": {"1": 500, "2": 900, "3": 30}}));
        assert!(!target.path("profiles/markers/222.json").exists(), "an account that was not given a place is not put");
        assert_eq!(read(&target, "planner.json")["goals"], json!([{"Weapon": 2}, {"Weapon": 1}]));
        assert_eq!(read(&target, "planner.json")["percent_mode"], true);
        assert_eq!(read(&target, "map_filters.json"), json!({"enabled": [2, 3]}));
        // What was there is kept to go back to.
        let backup = applied.backup.expect("a backup");
        assert!(backup.join("profiles/markers/999.json").exists());
    }

    #[test]
    fn replacing_puts_the_backup_in_place_and_keeps_the_old() {
        let from = tempfile::tempdir().expect("a folder");
        let source = roots(from.path());
        write(&source, "profiles/quest_status/5.json", &json!({"1": 1}));
        let archive = from.path().join("b.zip");
        export(&source, &Choice { sections: BTreeSet::from([Section::Quests]), accounts: BTreeSet::from([5]) }, None, &archive).expect("exported");
        let to = tempfile::tempdir().expect("a folder");
        let target = roots(to.path());
        write(&target, "profiles/quest_status/5.json", &json!({"1": 0, "2": 1}));
        let plan = Plan { mode: Mode::Replace, sections: BTreeSet::from([Section::Quests]), accounts: BTreeMap::from([(5, 5)]), owner_pid: 0 };
        let applied = apply(&target, &archive, &plan).expect("applied");
        assert_eq!(read(&target, "profiles/quest_status/5.json"), json!({"1": 1}));
        assert!(applied.backup.expect("backup").join("profiles/quest_status/5.json").exists());
    }

    #[test]
    fn what_is_not_a_backup_or_is_too_new_is_refused() {
        let dir = tempfile::tempdir().expect("a folder");
        let bad = dir.path().join("bad.zip");
        std::fs::write(&bad, "not a zip").expect("written");
        assert!(inspect(&bad).is_err());
        // A zip with no manifest.
        let plain = dir.path().join("plain.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&plain).expect("made"));
        zip.start_file("x.txt", zip::write::SimpleFileOptions::default()).expect("started");
        zip.write_all(b"x").expect("written");
        zip.finish().expect("finished");
        assert!(inspect(&plain).is_err());
        // A manifest of a newer format.
        let new = dir.path().join("new.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&new).expect("made"));
        zip.start_file(MANIFEST, zip::write::SimpleFileOptions::default()).expect("started");
        zip.write_all(serde_json::to_string(&Manifest { format: FORMAT + 1, program: "Aaru".into(), version: "9".into(), created: String::new(), sections: BTreeSet::new(), accounts: BTreeMap::new() }).expect("json").as_bytes()).expect("written");
        zip.finish().expect("finished");
        assert!(inspect(&new).expect_err("refused").to_string().contains("newer"));
    }

    #[test]
    fn the_accounts_that_have_anything_are_found() {
        let dir = tempfile::tempdir().expect("a folder");
        let roots = roots(dir.path());
        write(&roots, "profiles/markers/12.json", &json!({}));
        write(&roots, "profiles/chests/34.json", &json!({}));
        write(&roots, "profiles/56.json", &json!({}));
        write(&roots, "profiles/markers/notes.json", &json!({}));
        assert_eq!(roots.accounts(), vec![12, 34, 56]);
    }
}
