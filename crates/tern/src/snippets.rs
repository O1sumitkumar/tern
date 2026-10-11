//! Saved commands in `snippets.json`: a name to find them by and the text to type.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "snippets.json";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    /// Written to the terminal as is. It runs only when it ends with a newline.
    pub command: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    snippets: Vec<Snippet>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("snippets.json could not be read ({0}); fix or move it, tern will not overwrite it")]
    Unreadable(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// A missing file is an empty list; one that does not parse is an error so it is not
/// overwritten.
///
/// # Errors
/// [`StoreError::Unreadable`] for a bad file, [`StoreError::Io`] when it cannot be read.
pub fn load(dir: &Path) -> Result<Vec<Snippet>, StoreError> {
    let text = match std::fs::read_to_string(dir.join(FILE_NAME)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let file: File =
        serde_json::from_str(&text).map_err(|e| StoreError::Unreadable(e.to_string()))?;
    if file.version != VERSION {
        return Err(StoreError::Unreadable(format!("version {}", file.version)));
    }
    Ok(file.snippets)
}

/// Temp file + rename.
///
/// # Errors
/// [`StoreError::Io`] when the directory or file cannot be written.
pub fn save(dir: &Path, snippets: &[Snippet]) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(FILE_NAME);
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(&File {
        version: VERSION,
        snippets: snippets.to_vec(),
    })
    .map_err(io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Checks the form's text. The command keeps a trailing newline (that is how a snippet asks
/// to run at once) but not stray spaces around it.
///
/// # Errors
/// A message for the field that is wrong, worded for the form.
pub fn validate(name: &str, command: &str, others: &[&str]) -> Result<Snippet, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name is required.".into());
    }
    if others.contains(&name) {
        return Err(format!("A snippet named \"{name}\" already exists."));
    }
    let runs = command.trim_end_matches([' ', '\t']).ends_with('\n');
    let command = command.trim();
    if command.is_empty() {
        return Err("Command is required.".into());
    }
    Ok(Snippet {
        name: name.to_owned(),
        command: if runs {
            format!("{command}\n")
        } else {
            command.to_owned()
        },
    })
}
/// How a snippet reads in lists: the command on one line, with "↵" when picking it runs it
/// (it ends with a newline), so the trailing newline never shows as an escape.
pub fn preview(command: &str) -> String {
    let runs = command.ends_with('\n');
    let body = command.trim_end_matches('\n').replace('\n', " ⏎ ");
    if runs { format!("{body}  ↵") } else { body }
}

/// What the form shows for a stored command: a trailing newline is written `\n`, since a
/// single-line field cannot hold one.
pub fn to_field(command: &str) -> String {
    match command.strip_suffix('\n') {
        Some(rest) => format!("{rest}\\n"),
        None => command.to_owned(),
    }
}

/// The inverse of [`to_field`]: a trailing `\n` typed in the form means "run it".
pub fn from_field(text: &str) -> String {
    match text.trim_end().strip_suffix("\\n") {
        Some(rest) => format!("{rest}\n"),
        None => text.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_newline_is_what_makes_a_snippet_run() {
        let plain = validate("logs", " journalctl -u tern -f ", &[]).unwrap();
        assert_eq!(plain.command, "journalctl -u tern -f");
        let runs = validate("df", &from_field("df -h\\n"), &[]).unwrap();
        assert_eq!(runs.command, "df -h\n");
        assert_eq!(from_field(&to_field(&runs.command)), runs.command);
        assert_eq!(to_field("ls"), "ls");
    }

    #[test]
    fn each_bad_field_is_named() {
        assert!(validate(" ", "ls", &[]).unwrap_err().contains("Name"));
        assert!(validate("a", " ", &[]).unwrap_err().contains("Command"));
        assert!(validate("a", "ls", &["a"]).unwrap_err().contains("exists"));
    }

    #[test]
    fn it_round_trips_and_a_broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_empty());
        let list = vec![
            validate("df", "df -h\n", &[]).unwrap(),
            validate("top", "top -o cpu", &[]).unwrap(),
        ];
        save(dir.path(), &list).unwrap();
        assert_eq!(load(dir.path()).unwrap(), list);
        std::fs::write(dir.path().join(FILE_NAME), "{ nope").unwrap();
        assert!(matches!(load(dir.path()), Err(StoreError::Unreadable(_))));
    }

    /// A crash between the temp write and the rename leaves a garbage `.tmp` beside the good
    /// file; `load` reads the committed file and the next `save` replaces the stray.
    #[test]
    fn a_stray_tmp_beside_a_good_file_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let first = vec![validate("df", "df -h\n", &[]).unwrap()];
        save(dir.path(), &first).unwrap();
        std::fs::write(dir.path().join("snippets.json.tmp"), "{ half writ").unwrap();
        assert_eq!(load(dir.path()).unwrap(), first);

        let second = vec![
            validate("df", "df -h\n", &[]).unwrap(),
            validate("top", "top -o cpu", &[]).unwrap(),
        ];
        save(dir.path(), &second).unwrap();
        assert_eq!(load(dir.path()).unwrap(), second);
    }

    /// The rename never happened: only the `.tmp` exists. It is not the snippets file.
    #[test]
    fn a_tmp_without_the_real_file_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = r#"{"version":1,"snippets":[{"name":"ghost","command":"ls"}]}"#;
        std::fs::write(dir.path().join("snippets.json.tmp"), tmp).unwrap();
        assert!(load(dir.path()).unwrap().is_empty());
    }

    /// A file from a newer tern is refused, and left exactly as it was.
    #[test]
    fn a_newer_version_is_refused_and_left_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let newer = format!(
            r#"{{"version":{},"snippets":[{{"name":"df","command":"df -h"}}]}}"#,
            VERSION + 1
        );
        std::fs::write(&path, &newer).unwrap();
        assert!(matches!(load(dir.path()), Err(StoreError::Unreadable(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
    }

    #[test]
    fn preview_marks_running_snippets_and_hides_the_newline() {
        assert_eq!(super::preview("df -h\n"), "df -h  ↵");
        assert_eq!(
            super::preview("sudo docker restart "),
            "sudo docker restart "
        );
        assert_eq!(super::preview("a\nb\n"), "a ⏎ b  ↵");
    }
}
