//! Parser for `git status --porcelain=v2 --branch -z`.

use anyhow::{Result, anyhow, bail};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: Branch,
    pub entries: Vec<StatusEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Branch {
    /// The checked-out branch; `None` when HEAD is detached.
    pub head: Option<String>,
    /// HEAD's commit; `None` before the first commit.
    pub oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    /// The source path of a rename or copy.
    pub orig_path: Option<String>,
    pub status: FileStatus,
}

/// Where a file's changes live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    /// A tracked file; either side may be unchanged.
    Tracked {
        index: Change,
        worktree: Change,
    },
    /// Unmerged: both sides need resolving.
    Conflicted,
    Untracked,
}

/// One side of a tracked file's change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
}

impl Change {
    fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            b'.' => Self::Unmodified,
            b'M' => Self::Modified,
            b'T' => Self::TypeChanged,
            b'A' => Self::Added,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            other => bail!("unknown status code {:?}", other as char),
        })
    }

    pub fn is_changed(self) -> bool {
        self != Self::Unmodified
    }

    /// One-letter code for compact display.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Unmodified => " ",
            Self::Modified => "M",
            Self::TypeChanged => "T",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
        }
    }
}

impl StatusEntry {
    pub fn is_staged(&self) -> bool {
        matches!(self.status, FileStatus::Tracked { index, .. } if index.is_changed())
    }

    pub fn is_unstaged(&self) -> bool {
        matches!(self.status, FileStatus::Tracked { worktree, .. } if worktree.is_changed())
    }
}

impl Status {
    pub fn staged(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|entry| entry.is_staged())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries.iter().filter(|entry| entry.is_unstaged())
    }

    pub fn untracked(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.status == FileStatus::Untracked)
    }

    pub fn conflicted(&self) -> impl Iterator<Item = &StatusEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.status == FileStatus::Conflicted)
    }
}

pub(crate) fn parse(output: &str) -> Result<Status> {
    let mut status = Status::default();
    let mut records = output.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let (tag, rest) = record.split_at(1);
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        match tag {
            "#" => parse_header(rest, &mut status.branch),
            "1" => {
                let fields: Vec<&str> = rest.splitn(8, ' ').collect();
                let [xy, _, _, _, _, _, _, path] = fields[..] else {
                    bail!("malformed status record: {record}");
                };
                status.entries.push(tracked(xy, path, None)?);
            }
            "2" => {
                let fields: Vec<&str> = rest.splitn(9, ' ').collect();
                let [xy, _, _, _, _, _, _, _, path] = fields[..] else {
                    bail!("malformed status record: {record}");
                };
                let orig = records
                    .next()
                    .ok_or_else(|| anyhow!("rename without source: {record}"))?;
                status.entries.push(tracked(xy, path, Some(orig))?);
            }
            "u" => {
                let fields: Vec<&str> = rest.splitn(10, ' ').collect();
                let Some(path) = fields.get(9) else {
                    bail!("malformed status record: {record}");
                };
                status.entries.push(StatusEntry {
                    path: (*path).to_owned(),
                    orig_path: None,
                    status: FileStatus::Conflicted,
                });
            }
            "?" => status.entries.push(StatusEntry {
                path: rest.to_owned(),
                orig_path: None,
                status: FileStatus::Untracked,
            }),
            "!" => {}
            _ => bail!("unknown status record: {record}"),
        }
    }
    status.entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(status)
}

fn parse_header(header: &str, branch: &mut Branch) {
    let (key, value) = header.split_once(' ').unwrap_or((header, ""));
    match key {
        "branch.oid" if value != "(initial)" => branch.oid = Some(value.to_owned()),
        "branch.head" if value != "(detached)" => branch.head = Some(value.to_owned()),
        "branch.upstream" => branch.upstream = Some(value.to_owned()),
        "branch.ab" => {
            for part in value.split(' ') {
                if let Some(n) = part.strip_prefix('+') {
                    branch.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    branch.behind = n.parse().unwrap_or(0);
                }
            }
        }
        _ => {}
    }
}

fn tracked(xy: &str, path: &str, orig: Option<&str>) -> Result<StatusEntry> {
    let &[x, y] = xy.as_bytes() else {
        bail!("malformed XY field {xy:?}");
    };
    Ok(StatusEntry {
        path: path.to_owned(),
        orig_path: orig.map(str::to_owned),
        status: FileStatus::Tracked {
            index: Change::from_code(x)?,
            worktree: Change::from_code(y)?,
        },
    })
}
