//! Throwing away old files the program wrote for finding out what is wrong: the logs of what
//! the traffic of the game says (census, probe, track), the samples of what could not be
//! decrypted or unpacked, and the background logs that were kept.  They are made again as the
//! program runs and are of no use when old, but they grow without end: a day of play is a few
//! megabytes.  What is not named here (the profiles, what is collected, the settings, the
//! timelines of the capture) is never touched.
//!
//! The sample of the answer to a login has the token of the account in it, so it is kept for
//! a short time only.

use std::path::Path;
use std::time::{Duration, SystemTime};

const DAY: u64 = 24 * 3600;

/// A kind of file: how its name begins and ends, and after how many days it goes.
struct Rule {
    begins: &'static str,
    ends: &'static str,
    days: u64,
}

/// In the folder of the logs.
const LOG_RULES: &[Rule] = &[
    Rule { begins: "census-", ends: ".log", days: 14 },
    Rule { begins: "probe-", ends: ".log", days: 14 },
    Rule { begins: "track-", ends: ".log", days: 14 },
];

/// In the folder of the debug files.
const DEBUG_RULES: &[Rule] = &[
    Rule { begins: "undecrypted-", ends: ".bin", days: 14 },
    Rule { begins: "packed-login-", ends: ".bin", days: 2 },
    Rule { begins: "packed-", ends: ".bin", days: 14 },
    Rule { begins: "timeline-", ends: ".log", days: 60 },
];

/// In the folder of the background logs (the ones that were kept, with their marks).
const SESSION_RULES: &[Rule] = &[
    Rule { begins: "session-", ends: ".log", days: 30 },
    Rule { begins: "session-", ends: ".keep", days: 30 },
];

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub files: u64,
    pub bytes: u64,
}

fn clean(dir: &Path, rules: &[Rule], now: SystemTime) -> Cleaned {
    let mut done = Cleaned::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return done;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // The first rule that names the file is the one for it.
        let Some(rule) = rules.iter().find(|rule| name.starts_with(rule.begins) && name.ends_with(rule.ends)) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let age = meta.modified().ok().and_then(|at| now.duration_since(at).ok()).unwrap_or(Duration::ZERO);
        if meta.is_file() && age > Duration::from_secs(rule.days * DAY) && std::fs::remove_file(entry.path()).is_ok() {
            done.files += 1;
            done.bytes += meta.len();
        }
    }
    done
}

/// Makes the data built into the program ready in a thread of its own, while the window is
/// being made, so that the first tab that wants it does not wait (about a tenth of a second,
/// on the thread that draws).  Each is made once, and whoever wants it first makes it.
pub fn preload() {
    crate::spawn_named("preload", || {
        drop(crate::markers::data());
        drop(crate::upgrade_store::data());
        drop(crate::official_chests::all());
        drop(crate::chests::kinds());
        drop(crate::mob_watch::label_of(0));
    });
}

/// Cleans the folders of the program, in a thread of its own (it is of no hurry).
pub fn start() {
    crate::spawn_named("housekeeping", || {
        let Ok(logs) = crate::log_dir() else {
            return;
        };
        let now = SystemTime::now();
        let debug = crate::debug_timeline::debug_dir().ok();
        let mut total = clean(&logs, LOG_RULES, now);
        let sessions = clean(&logs.join("sessions"), SESSION_RULES, now);
        total.files += sessions.files;
        total.bytes += sessions.bytes;
        if let Some(debug) = debug {
            let more = clean(&debug, DEBUG_RULES, now);
            total.files += more.files;
            total.bytes += more.bytes;
        }
        if total.files > 0 {
            tracing::info!("old files of the logs were thrown away: {} files, {} MB", total.files, total.bytes / 1_000_000);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old(path: &Path, days: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).expect("open");
        file.set_modified(SystemTime::now() - Duration::from_secs(days * DAY)).expect("set");
    }

    #[test]
    fn only_old_files_of_the_named_kinds_go() {
        let dir = tempfile::tempdir().expect("a folder");
        let make = |name: &str, days: u64| {
            let path = dir.path().join(name);
            std::fs::write(&path, "12345").expect("written");
            old(&path, days);
        };
        make("census-1.log", 20);
        make("census-2.log", 3);
        make("probe-raw-1.log", 30);
        make("crash.log", 90);
        make("session-1.log", 90);
        make("packed-login-1.bin", 3);
        make("packed-login-2.bin", 1);
        make("profile.json", 400);
        let cleaned = clean(dir.path(), LOG_RULES, SystemTime::now());
        assert_eq!(cleaned, Cleaned { files: 2, bytes: 10 });
        let left = |name: &str| dir.path().join(name).exists();
        assert!(!left("census-1.log") && left("census-2.log") && !left("probe-raw-1.log"));
        // Not named here: kept whatever its age.
        assert!(left("crash.log") && left("session-1.log") && left("profile.json"));
        // The sample of a login goes after two days, in the folder it is in.
        let cleaned = clean(dir.path(), DEBUG_RULES, SystemTime::now());
        assert_eq!(cleaned.files, 1);
        assert!(!left("packed-login-1.bin") && left("packed-login-2.bin"));
    }
}

#[cfg(test)]
mod startup {
    /// How long what is built into the program takes to be made ready (for looking at).
    #[test]
    #[ignore]
    fn the_built_in_data_is_timed() {
        let time = |name: &str, work: &dyn Fn()| {
            let started = std::time::Instant::now();
            work();
            println!("{name}: {:?}", started.elapsed());
        };
        time("markers (labels, 89k points, atlas)", &|| drop(crate::markers::data()));
        time("upgrade data", &|| drop(crate::upgrade_store::data()));
        time("official chests", &|| drop(crate::official_chests::all()));
        time("chest kinds", &|| drop(crate::chests::kinds()));
        time("achievements (ru)", &|| drop(crate::achievement_data::AchievementDb::load(crate::achievement_data::Language::Ru)));
        time("monster table", &|| drop(crate::mob_watch::label_of(26310201)));
        time("markers again", &|| drop(crate::markers::data()));
    }
}
