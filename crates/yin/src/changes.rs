//! The flattened row model behind the Changes list.

use yin_git::{Change, DiffTarget, FileStatus, Status, StatusEntry};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Conflicts,
    Staged,
    Changes,
    Untracked,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Staged => "Staged",
            Self::Changes => "Changes",
            Self::Untracked => "Untracked",
        }
    }

    pub fn diff_target(self) -> DiffTarget {
        match self {
            Self::Staged => DiffTarget::Staged,
            Self::Conflicts | Self::Changes => DiffTarget::Unstaged,
            Self::Untracked => DiffTarget::Untracked,
        }
    }

    /// Whether this section's files are in the index.
    pub fn is_staged(self) -> bool {
        self == Self::Staged
    }
}

/// A file in one section. The same path can appear in Staged and Changes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileKey {
    pub section: Section,
    pub path: String,
}

#[derive(Clone, Debug)]
pub enum Row {
    Header {
        section: Section,
        count: usize,
    },
    File {
        section: Section,
        entry: StatusEntry,
    },
}

impl Row {
    pub fn key(&self) -> Option<FileKey> {
        match self {
            Row::File { section, entry } => Some(FileKey {
                section: *section,
                path: entry.path.clone(),
            }),
            Row::Header { .. } => None,
        }
    }
}

/// The badge shown for a file: what changed on the side this section shows.
pub fn change_for(section: Section, entry: &StatusEntry) -> Change {
    match (section, entry.status) {
        (Section::Staged, FileStatus::Tracked { index, .. }) => index,
        (Section::Changes, FileStatus::Tracked { worktree, .. }) => worktree,
        (Section::Untracked, _) => Change::Added,
        _ => Change::Modified,
    }
}

pub fn rows(status: &Status) -> Vec<Row> {
    let sections: [(Section, Vec<&StatusEntry>); 4] = [
        (Section::Conflicts, status.conflicted().collect()),
        (Section::Staged, status.staged().collect()),
        (Section::Changes, status.unstaged().collect()),
        (Section::Untracked, status.untracked().collect()),
    ];
    let mut rows = Vec::with_capacity(status.entries.len() + 4);
    for (section, entries) in sections {
        if entries.is_empty() {
            continue;
        }
        rows.push(Row::Header {
            section,
            count: entries.len(),
        });
        rows.extend(entries.into_iter().map(|entry| Row::File {
            section,
            entry: entry.clone(),
        }));
    }
    rows
}

/// After a refresh, keep the same file selected, or the row that took its place.
pub fn reselect(
    rows: &[Row],
    previous: Option<&FileKey>,
    previous_ix: Option<usize>,
) -> Option<usize> {
    if let Some(key) = previous
        && let Some(ix) = rows.iter().position(|row| row.key().as_ref() == Some(key))
    {
        return Some(ix);
    }
    let start = previous_ix.unwrap_or(0).min(rows.len().saturating_sub(1));
    // Prefer the next file down, then the nearest one above.
    (start..rows.len())
        .chain((0..start).rev())
        .find(|&ix| matches!(rows[ix], Row::File { .. }))
}

/// The next file row from `from` in `direction`, skipping headers.
pub fn step(rows: &[Row], from: Option<usize>, direction: isize) -> Option<usize> {
    let Some(from) = from else {
        return rows.iter().position(|row| matches!(row, Row::File { .. }));
    };
    let mut ix = from as isize;
    loop {
        ix += direction;
        if ix < 0 || ix as usize >= rows.len() {
            return Some(from);
        }
        if matches!(rows[ix as usize], Row::File { .. }) {
            return Some(ix as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, status: FileStatus) -> StatusEntry {
        StatusEntry {
            path: path.into(),
            orig_path: None,
            status,
        }
    }

    fn status() -> Status {
        Status {
            branch: Default::default(),
            entries: vec![
                entry(
                    "both.rs",
                    FileStatus::Tracked {
                        index: Change::Modified,
                        worktree: Change::Modified,
                    },
                ),
                entry("new.rs", FileStatus::Untracked),
                entry(
                    "staged.rs",
                    FileStatus::Tracked {
                        index: Change::Added,
                        worktree: Change::Unmodified,
                    },
                ),
            ],
        }
    }

    #[test]
    fn groups_files_by_section() {
        let rows = rows(&status());
        let shape: Vec<String> = rows
            .iter()
            .map(|row| match row {
                Row::Header { section, count } => format!("# {} {count}", section.title()),
                Row::File { entry, .. } => entry.path.clone(),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "# Staged 2",
                "both.rs",
                "staged.rs",
                "# Changes 1",
                "both.rs",
                "# Untracked 1",
                "new.rs"
            ]
        );
    }

    #[test]
    fn navigation_skips_headers_and_stops_at_ends() {
        let rows = rows(&status());
        assert_eq!(step(&rows, None, 1), Some(1));
        assert_eq!(step(&rows, Some(2), 1), Some(4));
        assert_eq!(step(&rows, Some(4), -1), Some(2));
        assert_eq!(step(&rows, Some(6), 1), Some(6));
        assert_eq!(step(&rows, Some(1), -1), Some(1));
    }

    #[test]
    fn reselects_same_file_or_neighbor() {
        let rows = rows(&status());
        let key = FileKey {
            section: Section::Changes,
            path: "both.rs".into(),
        };
        assert_eq!(reselect(&rows, Some(&key), Some(0)), Some(4));
        let gone = FileKey {
            section: Section::Changes,
            path: "gone.rs".into(),
        };
        assert_eq!(reselect(&rows, Some(&gone), Some(5)), Some(6));
        assert_eq!(reselect(&[], Some(&gone), Some(5)), None);
    }
}
