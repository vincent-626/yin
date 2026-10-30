//! Commits, branches and stashes, parsed from git's machine-readable output.

use anyhow::{Result, bail};

/// Field separator in `--format` strings; never appears in git metadata.
pub(crate) const FIELD: char = '\x1f';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub oid: String,
    pub short_oid: String,
    pub author: String,
    /// Author time, seconds since the epoch.
    pub time: i64,
    pub subject: String,
    /// Branch and tag names pointing at this commit, as `%D` prints them.
    pub refs: Vec<String>,
}

pub(crate) const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%an%x1f%at%x1f%s%x1f%D";

pub(crate) fn parse_log(output: &str) -> Result<Vec<Commit>> {
    output
        .split('\0')
        .map(|record| record.trim_start_matches('\n'))
        .filter(|record| !record.is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.split(FIELD).collect();
            let [oid, short_oid, author, time, subject, refs] = fields[..] else {
                bail!("malformed log record: {record:?}");
            };
            Ok(Commit {
                oid: oid.to_owned(),
                short_oid: short_oid.to_owned(),
                author: author.to_owned(),
                time: time.parse().unwrap_or(0),
                subject: subject.to_owned(),
                refs: refs
                    .split(", ")
                    .filter(|r| !r.is_empty())
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRef {
    /// `main`, or `origin/feature` for a remote-tracking branch.
    pub name: String,
    pub is_remote: bool,
    pub is_head: bool,
    pub upstream: Option<String>,
    /// Committer time of the tip, seconds since the epoch.
    pub time: i64,
    pub subject: String,
}

impl BranchRef {
    /// For a remote branch, the local name `git switch` would create.
    pub fn local_name(&self) -> &str {
        if self.is_remote {
            self.name
                .split_once('/')
                .map_or(self.name.as_str(), |(_, name)| name)
        } else {
            &self.name
        }
    }
}

pub(crate) const BRANCH_FORMAT: &str = "--format=%(refname)%1f%(refname:short)%1f%(HEAD)%1f%(upstream:short)%1f%(committerdate:unix)%1f%(contents:subject)";

/// Local branches, then remote branches that have no local counterpart, each
/// most recently committed first.
pub(crate) fn parse_branches(output: &str) -> Result<Vec<BranchRef>> {
    let mut branches = Vec::new();
    for line in output.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.split(FIELD).collect();
        let [refname, name, head, upstream, time, subject] = fields[..] else {
            bail!("malformed branch record: {line:?}");
        };
        // `refs/remotes/origin/HEAD` is an alias, not a branch.
        if refname.ends_with("/HEAD") {
            continue;
        }
        branches.push(BranchRef {
            name: name.to_owned(),
            is_remote: refname.starts_with("refs/remotes/"),
            is_head: head == "*",
            upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
            time: time.parse().unwrap_or(0),
            subject: subject.to_owned(),
        });
    }
    let local: Vec<String> = branches
        .iter()
        .filter(|b| !b.is_remote)
        .map(|b| b.name.clone())
        .collect();
    branches.retain(|b| !b.is_remote || !local.iter().any(|name| name == b.local_name()));
    branches.sort_by(|a, b| a.is_remote.cmp(&b.is_remote).then(b.time.cmp(&a.time)));
    Ok(branches)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    /// Position in the stash list; `stash@{index}`.
    pub index: usize,
    pub message: String,
}

impl Stash {
    pub fn reference(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }
}

pub(crate) fn parse_stashes(output: &str) -> Vec<Stash> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(index, message)| Stash {
            index,
            message: message.to_owned(),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: std::path::PathBuf,
    /// The checked-out branch; `None` when detached or bare.
    pub branch: Option<String>,
    pub oid: Option<String>,
    pub is_bare: bool,
}

/// Parses `git worktree list --porcelain -z`.
pub(crate) fn parse_worktrees(output: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in output.split('\0') {
        if field.is_empty() {
            // A blank record ends a worktree.
            worktrees.extend(current.take());
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        match key {
            "worktree" => {
                worktrees.extend(current.take());
                current = Some(Worktree {
                    path: value.into(),
                    branch: None,
                    oid: None,
                    is_bare: false,
                });
            }
            "HEAD" => {
                if let Some(worktree) = &mut current {
                    worktree.oid = Some(value.to_owned());
                }
            }
            "branch" => {
                if let Some(worktree) = &mut current {
                    worktree.branch = Some(value.trim_start_matches("refs/heads/").to_owned());
                }
            }
            "bare" => {
                if let Some(worktree) = &mut current {
                    worktree.is_bare = true;
                }
            }
            _ => {}
        }
    }
    worktrees.extend(current);
    worktrees
}

/// One line of `git blame`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// All zeros for lines not committed yet.
    pub oid: String,
    pub author: String,
    /// Author time, seconds since the epoch.
    pub time: i64,
    pub summary: String,
    pub text: String,
}

impl BlameLine {
    pub fn is_committed(&self) -> bool {
        !self.oid.bytes().all(|byte| byte == b'0')
    }
}

/// Parses `git blame --porcelain`: a header per line, commit details the
/// first time each commit appears, then the line itself after a tab.
pub(crate) fn parse_blame(output: &str) -> Vec<BlameLine> {
    #[derive(Clone, Default)]
    struct Details {
        author: String,
        time: i64,
        summary: String,
    }
    let mut commits: std::collections::HashMap<String, Details> = Default::default();
    let mut lines = Vec::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        if let Some(text) = line.strip_prefix('\t') {
            let Some(oid) = current.take() else { continue };
            let details = commits.get(&oid).cloned().unwrap_or_default();
            lines.push(BlameLine {
                oid,
                author: details.author,
                time: details.time,
                summary: details.summary,
                text: text.to_owned(),
            });
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key.len() == 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            commits.entry(key.to_owned()).or_default();
            current = Some(key.to_owned());
            continue;
        }
        let Some(details) = current.as_ref().and_then(|oid| commits.get_mut(oid)) else {
            continue;
        };
        match key {
            "author" => details.author = value.to_owned(),
            "author-time" => details.time = value.parse().unwrap_or(0),
            "summary" => details.summary = value.to_owned(),
            _ => {}
        }
    }
    lines
}
