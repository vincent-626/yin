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
