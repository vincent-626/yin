//! Several commits at once: the combined change of a run of commits, a branch
//! against its base, and the stack of branches HEAD sits on.

use anyhow::Result;

use crate::history::{self, Commit};
use crate::{DIFF_ARGS, Repository};

/// Branches stacked from trunk up to the current one and beyond: each layer
/// is a branch whose base is the layer below it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    /// The branch everything grows from, such as `main` or `origin/main`.
    pub trunk: String,
    /// Bottom (on trunk) to top.
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub branch: String,
    /// The branch below this one: trunk or another layer.
    pub base: String,
    /// Commits on this layer that its base doesn't have.
    pub commits: usize,
    pub is_head: bool,
}

impl Stack {
    /// The layer HEAD is on.
    pub fn head(&self) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.is_head)
    }
}

impl Repository {
    /// Everything commits `oldest` through `newest` changed together, as one
    /// patch. `oldest` is the earlier end of the run; a root commit is
    /// compared with the empty tree. With `path`, only that file.
    pub fn commits_patch(&self, oldest: &str, newest: &str, path: Option<&str>) -> Result<String> {
        let parent = format!("{oldest}^");
        let from = if self.resolves(&parent) {
            parent
        } else {
            EMPTY_TREE.to_owned()
        };
        self.diff_between(&from, newest, path)
    }

    /// What `head` changed since it left `base`: `git diff base...head`.
    pub fn compare_patch(&self, base: &str, head: &str, path: Option<&str>) -> Result<String> {
        let base = self.merge_base(base, head)?;
        self.diff_between(&base, head, path)
    }

    fn diff_between(&self, from: &str, to: &str, path: Option<&str>) -> Result<String> {
        let mut args: Vec<&str> = vec!["diff"];
        args.extend(DIFF_ARGS);
        args.extend([from, to, "--"]);
        args.extend(path);
        self.git(args)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        Ok(self.git(["merge-base", a, b])?.trim().to_owned())
    }

    /// Commits on `head` that `base` doesn't have, newest first.
    pub fn log_range(&self, base: &str, head: &str, limit: usize) -> Result<Vec<Commit>> {
        let output = self.git([
            "log",
            "-z",
            "--no-show-signature",
            "--encoding=UTF-8",
            history::LOG_FORMAT,
            &format!("--max-count={limit}"),
            &format!("{base}..{head}"),
            "--",
        ])?;
        history::parse_log(&output)
    }

    /// Whether `rev` names a commit.
    pub fn resolves(&self, rev: &str) -> bool {
        self.git([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .is_ok()
    }

    /// The default branch: `origin/HEAD`'s target, else `main`, `master` or
    /// `trunk`. Of a local branch and its remote counterpart, whichever is
    /// closer to HEAD, so a stale local `main` doesn't pull in commits.
    pub fn trunk(&self) -> Option<String> {
        let remote_default = self
            .git([
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD",
            ])
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        let names: Vec<String> = match &remote_default {
            Some(remote) => vec![remote.trim_start_matches("origin/").to_owned()],
            None => ["main", "master", "trunk"].map(str::to_owned).to_vec(),
        };
        let mut candidates = Vec::new();
        for name in names {
            for candidate in [name.clone(), format!("origin/{name}")] {
                if self.resolves(&candidate) {
                    candidates.push(candidate);
                }
            }
            if !candidates.is_empty() {
                break;
            }
        }
        candidates.into_iter().min_by_key(|candidate| {
            self.count(&format!("{candidate}..HEAD"))
                .unwrap_or(usize::MAX)
        })
    }

    fn count(&self, range: &str) -> Result<usize> {
        Ok(self.git(["rev-list", "--count", range])?.trim().parse()?)
    }

    /// The branches HEAD's branch is stacked on, and the ones stacked on it.
    /// `None` on trunk itself, when detached, or with no trunk to grow from.
    pub fn stack(&self) -> Result<Option<Stack>> {
        let Some(trunk) = self.trunk() else {
            return Ok(None);
        };
        let Ok(current) = self.git(["symbolic-ref", "--quiet", "--short", "HEAD"]) else {
            return Ok(None);
        };
        let current = current.trim().to_owned();
        if current == trunk || trunk.strip_prefix("origin/") == Some(&current) {
            return Ok(None);
        }
        // Down: walk HEAD's first parents to trunk; branch names met on the
        // way are the layers below.
        let output = self.git([
            "log",
            "--first-parent",
            "--decorate-refs=refs/heads/",
            "--format=%D",
            &format!("{trunk}..HEAD"),
            "--",
        ])?;
        let mut below: Vec<(String, usize)> = Vec::new();
        for (ix, decoration) in output.lines().enumerate() {
            // Two names on one commit make an empty layer; keep the first.
            if let Some(name) = local_branches(decoration)
                .into_iter()
                .find(|name| *name != current)
            {
                below.push((name, ix));
            }
        }
        let total = output.lines().count();
        // Each layer's commits run from its tip to the next tip below.
        let mut tips: Vec<(String, usize)> = vec![(current.clone(), 0)];
        tips.extend(below);
        let mut layers: Vec<Layer> = Vec::new();
        for (ix, (branch, at)) in tips.iter().enumerate() {
            let (base, end) = match tips.get(ix + 1) {
                Some((base, base_at)) => (base.clone(), *base_at),
                None => (trunk.clone(), total),
            };
            layers.push(Layer {
                branch: branch.clone(),
                base,
                commits: end - at,
                is_head: ix == 0,
            });
        }
        layers.reverse();
        // Up: branches that contain HEAD, nearest first, each on the previous.
        let containing = self.git([
            "for-each-ref",
            "--contains=HEAD",
            "--format=%(refname:short)",
            "refs/heads/",
        ])?;
        let mut above: Vec<(String, usize)> = containing
            .lines()
            .filter(|name| *name != current && !name.is_empty())
            .filter_map(|name| Some((name.to_owned(), self.count(&format!("HEAD..{name}")).ok()?)))
            .filter(|(_, ahead)| *ahead > 0)
            .collect();
        above.sort_by_key(|(_, ahead)| *ahead);
        let mut base = (current, 0);
        for (branch, ahead) in above {
            // A sibling forked off the same layer isn't stacked on the last one.
            if !self.is_ancestor(&base.0, &branch) {
                continue;
            }
            layers.push(Layer {
                branch: branch.clone(),
                base: base.0.clone(),
                commits: ahead - base.1,
                is_head: false,
            });
            base = (branch, ahead);
        }
        Ok(Some(Stack { trunk, layers }))
    }

    fn is_ancestor(&self, ancestor: &str, of: &str) -> bool {
        self.git(["merge-base", "--is-ancestor", ancestor, of])
            .is_ok()
    }
}

/// `git hash-object -t tree /dev/null`.
pub(crate) const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Local branch names in a `%D` decoration, `HEAD -> ` stripped.
fn local_branches(decoration: &str) -> Vec<String> {
    decoration
        .split(", ")
        .map(|name| name.trim_start_matches("HEAD -> ").trim())
        .filter(|name| !name.is_empty() && *name != "HEAD" && !name.starts_with("tag: "))
        .map(str::to_owned)
        .collect()
}
