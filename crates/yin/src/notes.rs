//! Review notes: comments you leave on lines of a diff, kept per repository
//! on this machine, then handed to an agent as one prompt. Nothing is posted
//! anywhere.

use std::path::{Path, PathBuf};

use yin_git::{FilePatch, ReviewThread, Side};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    pub id: u64,
    pub path: String,
    /// On deleted lines, numbered as in the old file.
    pub old_side: bool,
    pub start: usize,
    pub end: usize,
    pub body: String,
    /// The lines the note is about, as they read when it was written.
    pub code: Vec<String>,
    /// Written on commits in History rather than on the working tree's
    /// changes, so the working tree can't make it outdated.
    #[serde(default)]
    pub on_commits: bool,
    /// Its code is no longer among the working tree's changes.
    #[serde(default)]
    pub outdated: bool,
}

impl Note {
    /// `src/lib.rs:12` or `src/lib.rs:12-15`, with `(deleted)` for old lines.
    pub fn location(&self) -> String {
        let lines = if self.start == self.end {
            format!("{}", self.end)
        } else {
            format!("{}-{}", self.start, self.end)
        };
        let deleted = if self.old_side {
            " (deleted lines)"
        } else {
            ""
        };
        format!("{}:{lines}{deleted}", self.path)
    }

    fn side(&self) -> Side {
        if self.old_side { Side::Old } else { Side::New }
    }

    /// Where the note's code is in `patch` now, as (start, end) lines: the
    /// occurrence nearest where it was. `None` when the code isn't there.
    pub fn find_in(&self, patch: &str) -> Option<(usize, usize)> {
        let file = FilePatch::of_file(patch, &self.path, self.side())?;
        let start = file
            .find_lines(self.side(), &self.code)
            .into_iter()
            .min_by_key(|start| start.abs_diff(self.start))?;
        Some((start, start + self.code.len() - 1))
    }
}

#[derive(Debug, Default)]
pub struct Notes {
    items: Vec<Note>,
    file: Option<PathBuf>,
}

impl Notes {
    pub fn load(workdir: &Path) -> Self {
        let file = crate::state::repo_file("notes", workdir, "json");
        let items = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { items, file }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let result = serde_json::to_string_pretty(&self.items)
            .map_err(std::io::Error::other)
            .and_then(|json| {
                std::fs::create_dir_all(file.parent().expect("notes dir"))?;
                if self.items.is_empty() {
                    std::fs::remove_file(file).or(Ok(()))
                } else {
                    std::fs::write(file, json)
                }
            });
        if let Err(error) = result {
            eprintln!("yin: couldn't save review notes: {error}");
        }
    }

    pub fn items(&self) -> &[Note] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Adds a note and returns its id.
    pub fn add(&mut self, mut note: Note) -> u64 {
        note.id = self.items.iter().map(|note| note.id + 1).max().unwrap_or(1);
        let id = note.id;
        self.items.push(note);
        self.save();
        id
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|note| note.id != id);
        self.save();
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.save();
    }

    pub fn outdated(&self) -> usize {
        self.items.iter().filter(|note| note.outdated).count()
    }

    /// Follows the working tree's changes in `patch`: a note whose code moved
    /// moves with it, and one whose code is gone is outdated. Files in
    /// `hidden` were left out of the patch, so their notes stay as they are.
    pub fn follow(&mut self, patch: &str, hidden: &dyn Fn(&str) -> bool) {
        let mut changed = false;
        for note in &mut self.items {
            if note.on_commits || note.code.is_empty() || hidden(&note.path) {
                continue;
            }
            let (start, end, outdated) = match note.find_in(patch) {
                Some((start, end)) => (start, end, false),
                None => (note.start, note.end, true),
            };
            if (start, end, outdated) != (note.start, note.end, note.outdated) {
                (note.start, note.end, note.outdated) = (start, end, outdated);
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }
}

/// One prompt asking an agent to address `notes` and the open review
/// `threads` of a pull request.
pub fn prompt(notes: &[Note], threads: &[ReviewThread]) -> String {
    let threads: Vec<&ReviewThread> = threads.iter().filter(|t| !t.is_resolved).collect();
    let mut out = String::from(
        "Please address these code review comments. Each one names a file and \
         line, quotes the code it is about, and says what to change.\n",
    );
    let mut n = 0;
    for note in notes {
        n += 1;
        let outdated = if note.outdated {
            " (outdated: this code has changed since the note was written)"
        } else {
            ""
        };
        out.push_str(&format!("\n{n}. {}{outdated}\n", note.location()));
        push_code(&mut out, &note.path, &note.code);
        push_body(&mut out, &note.body);
    }
    for thread in threads {
        n += 1;
        let line = thread
            .line
            .or(thread.original_line)
            .map_or(String::new(), |line| format!(":{line}"));
        out.push_str(&format!(
            "\n{n}. {}{line} (pull request review)\n",
            thread.path
        ));
        for comment in &thread.comments {
            push_body(&mut out, &format!("{}: {}", comment.author, comment.body));
        }
    }
    out
}

fn push_code(out: &mut String, path: &str, code: &[String]) {
    if code.is_empty() {
        return;
    }
    let language = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    out.push_str(&format!("   ```{language}\n"));
    for line in code {
        out.push_str(&format!("   {line}\n"));
    }
    out.push_str("   ```\n");
}

fn push_body(out: &mut String, body: &str) {
    for line in body.trim().lines() {
        out.push_str(&format!("   {line}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yin_git::{ReviewComment, Side};

    fn note(path: &str, start: usize, end: usize, body: &str) -> Note {
        Note {
            id: 0,
            path: path.into(),
            old_side: false,
            start,
            end,
            body: body.into(),
            code: vec!["let x = 1;".into()],
            on_commits: false,
            outdated: false,
        }
    }

    #[test]
    fn notes_follow_their_code_or_go_outdated() {
        let patch = |body: &str| {
            format!("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,3 @@\n{body}")
        };
        let mut notes = Notes::default();
        notes.add(note("a.rs", 2, 2, "moves"));
        notes.add(Note {
            code: vec!["gone".into()],
            ..note("a.rs", 3, 3, "goes")
        });
        notes.add(Note {
            on_commits: true,
            code: vec!["elsewhere".into()],
            ..note("a.rs", 1, 1, "history")
        });
        notes.add(Note {
            code: vec!["gone".into()],
            ..note("hidden.rs", 1, 1, "reviewed and hidden")
        });
        // The agent added two lines above `let x = 1;` and removed `gone`.
        notes.follow(&patch(" top\n+a\n+b\n+let x = 1;\n"), &|path| {
            path == "hidden.rs"
        });
        let items = notes.items();
        assert_eq!(
            (items[0].start, items[0].end, items[0].outdated),
            (4, 4, false)
        );
        assert!(items[1].outdated);
        assert!(!items[2].outdated, "notes on commits aren't checked");
        assert!(
            !items[3].outdated,
            "hidden files keep their notes as they were"
        );
        assert_eq!(notes.outdated(), 1);
        assert!(prompt(notes.items(), &[]).contains("3 (outdated: this code has changed"));
    }

    #[test]
    fn adds_and_removes_notes() {
        let mut notes = Notes::default();
        let first = notes.add(note("a.rs", 1, 1, "one"));
        let second = notes.add(note("b.rs", 2, 3, "two"));
        assert_ne!(first, second);
        notes.remove(first);
        assert_eq!(notes.items().len(), 1);
        assert_eq!(notes.items()[0].location(), "b.rs:2-3");
        notes.clear();
        assert!(notes.is_empty());
    }

    #[test]
    fn writes_a_prompt_from_notes_and_open_threads() {
        let notes = [note("src/a.rs", 4, 4, "Rename x.\nIt's vague.")];
        let thread = |resolved, body: &str| ReviewThread {
            path: "src/b.rs".into(),
            line: Some(9),
            original_line: Some(8),
            side: Side::New,
            is_resolved: resolved,
            comments: vec![ReviewComment {
                author: "ana".into(),
                body: body.into(),
                url: String::new(),
            }],
        };
        let prompt = prompt(
            &notes,
            &[thread(false, "Handle errors"), thread(true, "Done")],
        );
        assert!(prompt.contains(
            "1. src/a.rs:4\n   ```rs\n   let x = 1;\n   ```\n   Rename x.\n   It's vague.\n"
        ));
        assert!(prompt.contains("2. src/b.rs:9 (pull request review)\n   ana: Handle errors\n"));
        assert!(!prompt.contains("Done"));
    }
}
