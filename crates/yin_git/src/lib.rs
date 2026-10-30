//! Git model for yin. No UI dependencies.
//!
//! Every operation shells out to the `git` CLI so hooks, signing, credential
//! helpers and config behave exactly as they do in a terminal.

mod history;
mod patch;
mod status;

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};

pub use history::{BranchRef, Commit, Stash, Worktree};
pub use patch::{FilePatch, LinePosition, RowKind, Side};
pub use status::{Branch, Change, FileStatus, Status, StatusEntry};

/// A git working tree, addressed by its top-level directory.
#[derive(Clone, Debug)]
pub struct Repository {
    workdir: PathBuf,
}

/// An operation git stopped in the middle of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InProgress {
    Merge,
    Rebase,
}

/// Whether a failed pull means the branches diverged.
pub fn is_diverged_error(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("not possible to fast-forward") || error.contains("diverg")
}

/// Whether a failed push was rejected because the remote moved.
pub fn is_rejected_push(error: &str) -> bool {
    let error = error.to_lowercase();
    error.contains("rejected")
        && (error.contains("non-fast-forward") || error.contains("fetch first"))
}

/// Which side of the index a diff compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffTarget {
    /// HEAD against the index (`git diff --cached`).
    Staged,
    /// The index against the working tree (`git diff`).
    Unstaged,
    /// An untracked file, shown as fully added.
    Untracked,
}

const DIFF_ARGS: &[&str] = &[
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--find-renames",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

impl Repository {
    /// Opens the repository that contains `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("couldn't run git; is it installed?")?;
        if !output.status.success() {
            bail!("{} is not inside a git repository", path.display());
        }
        let top = String::from_utf8(output.stdout)?.trim_end().to_owned();
        Ok(Self {
            workdir: PathBuf::from(top),
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// The directory name, for titles.
    pub fn name(&self) -> String {
        self.workdir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.workdir.display().to_string())
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        command
            .arg("--no-pager")
            .arg("-C")
            .arg(&self.workdir)
            .args(["-c", "core.quotepath=false"])
            // Reads must never take index.lock: agents run git in the same tree.
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null());
        command
    }

    /// Runs git with `args` and returns stdout. Fails with git's stderr.
    pub fn git<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run(args, None, &[0])
    }

    fn run<I, S>(&self, args: I, stdin: Option<&str>, ok_codes: &[i32]) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command();
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().context("couldn't run git")?;
        if let Some(input) = stdin {
            child
                .stdin
                .take()
                .expect("stdin is piped")
                .write_all(input.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        let code = output.status.code().unwrap_or(-1);
        if !ok_codes.contains(&code) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            };
            return Err(anyhow!("{}", message.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn status(&self) -> Result<Status> {
        let output = self.git([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        status::parse(&output)
    }

    /// A unified patch for `path` on one side of the index.
    pub fn diff(&self, path: &str, target: DiffTarget) -> Result<String> {
        match target {
            DiffTarget::Staged => self.git(
                ["diff", "--cached"]
                    .iter()
                    .chain(DIFF_ARGS)
                    .chain(&["--", path]),
            ),
            DiffTarget::Unstaged => self.git(["diff"].iter().chain(DIFF_ARGS).chain(&["--", path])),
            // `--no-index` exits 1 when the files differ, which is always.
            DiffTarget::Untracked => self.run(
                ["diff", "--no-index"]
                    .iter()
                    .chain(DIFF_ARGS)
                    .chain(&["--", "/dev/null", path]),
                None,
                &[0, 1],
            ),
        }
    }

    /// One patch with every change against HEAD: staged, unstaged and
    /// untracked, skipping `hidden` paths. Before the first commit, against
    /// the empty tree.
    pub fn diff_all(&self, status: &Status, hidden: &dyn Fn(&str) -> bool) -> Result<String> {
        let tracked: Vec<&str> = status
            .entries
            .iter()
            .filter(|entry| entry.status != FileStatus::Untracked && !hidden(&entry.path))
            .map(|entry| entry.path.as_str())
            .collect();
        let mut patch = String::new();
        if !tracked.is_empty() {
            // The empty tree's id: diffing against it shows everything as added.
            let base = if self.has_head() {
                "HEAD"
            } else {
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            };
            patch = self.git(
                ["diff", base]
                    .iter()
                    .chain(DIFF_ARGS)
                    .chain(&["--"])
                    .chain(tracked.iter()),
            )?;
        }
        for entry in status.untracked() {
            if hidden(&entry.path) {
                continue;
            }
            patch.push_str(&self.diff(&entry.path, DiffTarget::Untracked)?);
        }
        Ok(patch)
    }

    /// Stages `paths`, including deletions.
    pub fn stage(&self, paths: &[&str]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.git(["add", "-A", "--"].iter().chain(paths)).map(drop)
    }

    /// Stages everything in the working tree.
    pub fn stage_all(&self) -> Result<()> {
        self.git(["add", "-A"]).map(drop)
    }

    /// Removes `paths` from the index, keeping working tree changes.
    pub fn unstage(&self, paths: &[&str]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        if self.has_head() {
            self.git(["restore", "--staged", "--"].iter().chain(paths))
                .map(drop)
        } else {
            // No commit to restore from yet.
            self.git(
                ["rm", "--cached", "-r", "--quiet", "--"]
                    .iter()
                    .chain(paths),
            )
            .map(drop)
        }
    }

    pub fn unstage_all(&self) -> Result<()> {
        if self.has_head() {
            self.git(["reset", "--quiet"]).map(drop)
        } else {
            self.git(["rm", "--cached", "-r", "--quiet", "."]).map(drop)
        }
    }

    /// Throws away unstaged changes to tracked `paths`. Destructive.
    pub fn discard(&self, paths: &[&str]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        self.git(["restore", "--worktree", "--"].iter().chain(paths))
            .map(drop)
    }

    /// Deletes untracked `paths` from disk. Destructive.
    pub fn remove_untracked(&self, paths: &[&str]) -> Result<()> {
        for path in paths {
            let full = self.workdir.join(path);
            std::fs::remove_file(&full)
                .with_context(|| format!("couldn't remove {}", full.display()))?;
        }
        Ok(())
    }

    /// Commits the index with `message`. Hooks run; their output is the error.
    pub fn commit(&self, message: &str, amend: bool) -> Result<()> {
        let mut args = vec!["commit", "--file=-", "--cleanup=strip"];
        if amend {
            args.push("--amend");
        }
        self.run(args, Some(message), &[0]).map(drop)
    }

    /// The full message of HEAD, for prefilling an amend.
    pub fn head_message(&self) -> Result<String> {
        Ok(self
            .git(["log", "-1", "--format=%B"])?
            .trim_end()
            .to_owned())
    }

    /// This worktree's git dir and the common dir shared by all worktrees.
    /// They differ for linked worktrees, and both can live outside `workdir`.
    pub fn git_dirs(&self) -> Result<(PathBuf, PathBuf)> {
        let output = self.git(["rev-parse", "--absolute-git-dir", "--git-common-dir"])?;
        let mut lines = output.lines();
        let (Some(git_dir), Some(common_dir)) = (lines.next(), lines.next()) else {
            bail!("unexpected rev-parse output: {output}");
        };
        let git_dir = PathBuf::from(git_dir);
        // `--git-common-dir` is relative to the working directory when inside it.
        let common_dir = self.workdir.join(common_dir);
        Ok((git_dir, common_dir.canonicalize().unwrap_or(common_dir)))
    }

    /// Applies a partial patch from [`FilePatch::select`] to the index:
    /// staging it, or with `reverse`, unstaging it.
    pub fn apply_to_index(&self, patch: &str, reverse: bool) -> Result<()> {
        let mut args = vec!["apply", "--cached", "--recount", "--whitespace=nowarn"];
        if reverse {
            args.push("--reverse");
        }
        args.push("-");
        self.run(args, Some(patch), &[0]).map(drop)
    }

    /// Commits reachable from HEAD, newest first.
    pub fn log(&self, limit: usize) -> Result<Vec<Commit>> {
        if !self.has_head() {
            return Ok(Vec::new());
        }
        let output = self.git([
            "log",
            "-z",
            "--no-show-signature",
            "--encoding=UTF-8",
            history::LOG_FORMAT,
            &format!("--max-count={limit}"),
            "HEAD",
            "--",
        ])?;
        history::parse_log(&output)
    }

    /// A commit's full message.
    pub fn commit_message(&self, oid: &str) -> Result<String> {
        Ok(self
            .git([
                "show",
                "--no-patch",
                "--no-show-signature",
                "--format=%B",
                oid,
                "--",
            ])?
            .trim_end()
            .to_owned())
    }

    /// The patch a commit introduces; merges against their first parent.
    pub fn commit_patch(&self, oid: &str) -> Result<String> {
        self.git(
            ["show", "--format=", "--first-parent", "--no-show-signature"]
                .iter()
                .chain(DIFF_ARGS)
                .chain(&[oid, "--"]),
        )
    }

    pub fn branches(&self) -> Result<Vec<BranchRef>> {
        let output = self.git([
            "for-each-ref",
            history::BRANCH_FORMAT,
            "refs/heads",
            "refs/remotes",
        ])?;
        history::parse_branches(&output)
    }

    /// Checks out a branch. A remote branch gets a local tracking branch.
    pub fn switch(&self, branch: &BranchRef) -> Result<()> {
        if branch.is_remote {
            self.git(["switch", "--track", &branch.name]).map(drop)
        } else {
            self.git(["switch", &branch.name]).map(drop)
        }
    }

    /// Creates a branch at HEAD and checks it out.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        self.git(["check-ref-format", "--branch", name])
            .map_err(|_| anyhow!("{name:?} is not a valid branch name"))?;
        self.git(["switch", "--create", name]).map(drop)
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        Ok(self.git(["remote"])?.lines().map(str::to_owned).collect())
    }

    pub fn fetch(&self) -> Result<()> {
        self.git(["fetch", "--all", "--prune"]).map(drop)
    }

    /// Fast-forwards to the upstream; refuses to create a merge.
    pub fn pull(&self) -> Result<()> {
        self.git(["pull", "--ff-only"]).map(drop)
    }

    /// Pushes the current branch, setting its upstream on the first push.
    pub fn push(&self, branch: &Branch) -> Result<()> {
        if branch.upstream.is_some() {
            return self.git(["push"]).map(drop);
        }
        if branch.head.is_none() {
            bail!("HEAD is detached; switch to a branch to push");
        }
        let remotes = self.remotes()?;
        let remote = remotes
            .iter()
            .find(|remote| *remote == "origin")
            .or(remotes.first())
            .ok_or_else(|| anyhow!("this repository has no remote to push to"))?;
        self.git(["push", "--set-upstream", remote, "HEAD"])
            .map(drop)
    }

    pub fn stashes(&self) -> Result<Vec<Stash>> {
        Ok(history::parse_stashes(&self.git([
            "stash",
            "list",
            "--format=%s",
        ])?))
    }

    /// Stashes all changes, untracked files included.
    pub fn stash_push(&self, message: Option<&str>) -> Result<()> {
        let mut args = vec!["stash", "push", "--include-untracked"];
        if let Some(message) = message {
            args.extend(["--message", message]);
        }
        self.git(args).map(drop)
    }

    pub fn stash_pop(&self, stash: &Stash) -> Result<()> {
        self.git(["stash", "pop", "--index", &stash.reference()])
            .map(drop)
    }

    pub fn stash_drop(&self, stash: &Stash) -> Result<()> {
        self.git(["stash", "drop", &stash.reference()]).map(drop)
    }

    pub fn stash_patch(&self, stash: &Stash) -> Result<String> {
        self.git(
            ["stash", "show", "--patch", "--include-untracked"]
                .iter()
                .chain(DIFF_ARGS)
                .chain(&[stash.reference().as_str()]),
        )
    }

    /// Marks a conflicted file resolved with `contents`.
    pub fn resolve_conflict(&self, path: &str, contents: &str) -> Result<()> {
        std::fs::write(self.workdir.join(path), contents)
            .with_context(|| format!("couldn't write {path}"))?;
        self.stage(&[path])
    }

    /// The working file of a conflicted path, markers included.
    pub fn read_file(&self, path: &str) -> Result<String> {
        std::fs::read_to_string(self.workdir.join(path))
            .with_context(|| format!("couldn't read {path}"))
    }

    /// Whether a merge is waiting for its conflicts to be resolved.
    pub fn merge_in_progress(&self) -> Result<bool> {
        Ok(self.in_progress()? == Some(InProgress::Merge))
    }

    /// A merge or rebase that stopped part way, waiting for the user.
    pub fn in_progress(&self) -> Result<Option<InProgress>> {
        let (git_dir, _) = self.git_dirs()?;
        Ok(
            if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
                Some(InProgress::Rebase)
            } else if git_dir.join("MERGE_HEAD").exists() {
                Some(InProgress::Merge)
            } else {
                None
            },
        )
    }

    /// The message git prepared for the merge commit, without comment lines.
    pub fn merge_message(&self) -> Result<Option<String>> {
        let (git_dir, _) = self.git_dirs()?;
        let Ok(text) = std::fs::read_to_string(git_dir.join("MERGE_MSG")) else {
            return Ok(None);
        };
        let message: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
        let message = message.join("\n").trim().to_owned();
        Ok((!message.is_empty()).then_some(message))
    }

    /// Rebases local commits onto the upstream, for a diverged branch.
    pub fn pull_rebase(&self) -> Result<()> {
        self.git(["pull", "--rebase"]).map(drop)
    }

    /// Pushes over a remote branch that moved, unless it moved again since
    /// the last fetch.
    pub fn push_force_with_lease(&self) -> Result<()> {
        self.git(["push", "--force-with-lease"]).map(drop)
    }

    pub fn abort_rebase(&self) -> Result<()> {
        self.git(["rebase", "--abort"]).map(drop)
    }

    /// Continues a rebase after conflicts are resolved and staged, keeping
    /// each commit's message.
    pub fn continue_rebase(&self) -> Result<()> {
        self.git(["-c", "core.editor=true", "rebase", "--continue"])
            .map(drop)
    }

    pub fn abort_merge(&self) -> Result<()> {
        self.git(["merge", "--abort"]).map(drop)
    }

    /// Every worktree of this repository, the main one first.
    pub fn worktrees(&self) -> Result<Vec<Worktree>> {
        Ok(history::parse_worktrees(&self.git([
            "worktree",
            "list",
            "--porcelain",
            "-z",
        ])?))
    }

    pub fn has_head(&self) -> bool {
        self.git(["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_ok()
    }
}

#[cfg(test)]
mod tests;
