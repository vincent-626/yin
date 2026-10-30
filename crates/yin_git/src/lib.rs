//! Git model for yin. No UI dependencies.
//!
//! Every operation shells out to the `git` CLI so hooks, signing, credential
//! helpers and config behave exactly as they do in a terminal.

mod status;

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};

pub use status::{Branch, Change, FileStatus, Status, StatusEntry};

/// A git working tree, addressed by its top-level directory.
#[derive(Clone, Debug)]
pub struct Repository {
    workdir: PathBuf,
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

    pub fn has_head(&self) -> bool {
        self.git(["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_ok()
    }
}

#[cfg(test)]
mod tests;
