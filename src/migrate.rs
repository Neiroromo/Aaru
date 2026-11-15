//! Taking the data of the program from the folder it had when it was called Irminsul.
//!
//! The data (profiles, what is collected on the map, the goals of the planner, the settings,
//! the key of the connection ...) was kept in `%APPDATA%\Irminsul\data`.  The program is
//! called Aaru now and looks in `%APPDATA%\Aaru\data`.  When that is not there and the old one
//! is, the old one is *copied* to it (the old one stays as it was, as a copy to go back to).  It
//! is copied to a folder of another name first and renamed when it is all there, so that a copy
//! that was stopped half way is not taken for the data.

use std::path::{Path, PathBuf};

/// The name the program had.
const OLD_APP_ID: &str = "Irminsul";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// There was nothing to do.
    Nothing,
    Copied { files: u64, bytes: u64 },
    Failed(String),
}

fn copy_all(from: &Path, to: &Path, files: &mut u64, bytes: &mut u64) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_all(&entry.path(), &target, files, bytes)?;
        } else {
            *bytes += std::fs::copy(entry.path(), &target)?;
            *files += 1;
        }
    }
    Ok(())
}

/// Copies `old` to `new` if `new` is not there and `old` is.
pub fn migrate(old: &Path, new: &Path) -> Outcome {
    if new.exists() || !old.is_dir() {
        return Outcome::Nothing;
    }
    let (Some(parent), Some(name)) = (new.parent(), new.file_name()) else {
        return Outcome::Nothing;
    };
    let mut half = name.to_os_string();
    half.push(".copying");
    let half = parent.join(half);
    // What a copy that was stopped left.
    let _ = std::fs::remove_dir_all(&half);
    let (mut files, mut bytes) = (0, 0);
    let copied = copy_all(old, &half, &mut files, &mut bytes).and_then(|()| std::fs::rename(&half, new));
    match copied {
        Ok(()) => Outcome::Copied { files, bytes },
        Err(e) => {
            let _ = std::fs::remove_dir_all(&half);
            Outcome::Failed(e.to_string())
        }
    }
}

/// The data folder of a name, as the program (`eframe`) finds it.
fn folder(app_id: &str) -> Option<PathBuf> {
    eframe::storage_dir(app_id)
}

/// Run first thing when the program starts.
pub fn run() -> Outcome {
    match (folder(OLD_APP_ID), folder(crate::APP_ID)) {
        (Some(old), Some(new)) if old != new => migrate(&old, &new),
        _ => Outcome::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_folder_is_copied_and_kept() {
        let base = tempfile::tempdir().expect("a folder");
        let old = base.path().join("Irminsul").join("data");
        std::fs::create_dir_all(old.join("profiles")).expect("made");
        std::fs::write(old.join("profiles").join("1.json"), "profile").expect("written");
        std::fs::write(old.join("app.ron"), "settings").expect("written");
        let new = base.path().join("Aaru").join("data");
        std::fs::create_dir_all(new.parent().expect("parent")).expect("made");
        let outcome = migrate(&old, &new);
        assert_eq!(outcome, Outcome::Copied { files: 2, bytes: 15 });
        assert_eq!(std::fs::read_to_string(new.join("profiles").join("1.json")).expect("read"), "profile");
        // The old one is as it was, and nothing half done is left.
        assert!(old.join("app.ron").exists());
        assert!(!base.path().join("Aaru").join("data.copying").exists());
        // Not done again, and not over what is there.
        std::fs::write(new.join("app.ron"), "changed").expect("written");
        assert_eq!(migrate(&old, &new), Outcome::Nothing);
        assert_eq!(std::fs::read_to_string(new.join("app.ron")).expect("read"), "changed");
    }

    #[test]
    fn nothing_is_done_without_an_old_folder() {
        let base = tempfile::tempdir().expect("a folder");
        assert_eq!(migrate(&base.path().join("Irminsul").join("data"), &base.path().join("Aaru").join("data")), Outcome::Nothing);
    }

    #[test]
    fn a_copy_that_was_stopped_is_made_again() {
        let base = tempfile::tempdir().expect("a folder");
        let old = base.path().join("Irminsul").join("data");
        std::fs::create_dir_all(&old).expect("made");
        std::fs::write(old.join("a"), "1").expect("written");
        let new = base.path().join("Aaru").join("data");
        std::fs::create_dir_all(base.path().join("Aaru").join("data.copying")).expect("made");
        std::fs::write(base.path().join("Aaru").join("data.copying").join("old"), "x").expect("written");
        assert!(matches!(migrate(&old, &new), Outcome::Copied { files: 1, .. }));
        assert!(!new.join("old").exists());
    }
}
