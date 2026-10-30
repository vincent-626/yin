use std::fs;
use std::process::Command;

use super::*;

/// A throwaway repository with an identity configured.
struct Fixture {
    _dir: tempfile::TempDir,
    repo: Repository,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet", "--initial-branch=main"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let repo = Repository::open(dir.path()).unwrap();
        repo.git(["config", "user.name", "Yin Test"]).unwrap();
        repo.git(["config", "user.email", "yin@example.com"])
            .unwrap();
        repo.git(["config", "commit.gpgsign", "false"]).unwrap();
        Self { _dir: dir, repo }
    }

    fn write(&self, path: &str, contents: &str) {
        let full = self.repo.workdir().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, contents).unwrap();
    }
}

#[test]
fn parses_porcelain_v2() {
    let output = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0\
                  # branch.ab +2 -1\0\
                  1 M. N... 100644 100644 100644 aaa bbb src/lib.rs\0\
                  1 .M N... 100644 100644 100644 aaa bbb file with spaces.txt\0\
                  2 R. N... 100644 100644 100644 aaa bbb R100 new.rs\0old.rs\0\
                  u UU N... 100644 100644 100644 100644 h1 h2 h3 conflict.rs\0\
                  ? notes.md\0";
    let status = status::parse(output).unwrap();
    assert_eq!(status.branch.head.as_deref(), Some("main"));
    assert_eq!(status.branch.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.branch.ahead, status.branch.behind), (2, 1));
    let paths: Vec<_> = status.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "conflict.rs",
            "file with spaces.txt",
            "new.rs",
            "notes.md",
            "src/lib.rs"
        ]
    );
    let renamed = status.entries.iter().find(|e| e.path == "new.rs").unwrap();
    assert_eq!(renamed.orig_path.as_deref(), Some("old.rs"));
    assert_eq!(status.staged().count(), 2);
    assert_eq!(status.unstaged().count(), 1);
    assert_eq!(status.untracked().count(), 1);
    assert_eq!(status.conflicted().count(), 1);
}

#[test]
fn detached_and_initial_heads() {
    let status = status::parse("# branch.oid (initial)\0# branch.head (detached)\0").unwrap();
    assert_eq!(status.branch.head, None);
    assert_eq!(status.branch.oid, None);
}

#[test]
fn stage_commit_and_diff_round_trip() {
    let fx = Fixture::new();
    fx.write("a.txt", "one\n");
    fx.write("dir/b.txt", "two\n");

    let status = fx.repo.status().unwrap();
    assert_eq!(status.untracked().count(), 2);
    assert!(
        fx.repo
            .diff("a.txt", DiffTarget::Untracked)
            .unwrap()
            .contains("+one")
    );

    // Staging and unstaging work before the first commit.
    fx.repo.stage(&["a.txt"]).unwrap();
    assert_eq!(fx.repo.status().unwrap().staged().count(), 1);
    fx.repo.unstage(&["a.txt"]).unwrap();
    assert_eq!(fx.repo.status().unwrap().staged().count(), 0);

    fx.repo.stage_all().unwrap();
    fx.repo.commit("initial\n\nbody", false).unwrap();
    assert!(fx.repo.status().unwrap().entries.is_empty());
    assert_eq!(fx.repo.head_message().unwrap(), "initial\n\nbody");

    fx.write("a.txt", "one\nmore\n");
    let status = fx.repo.status().unwrap();
    assert_eq!(status.unstaged().count(), 1);
    let patch = fx.repo.diff("a.txt", DiffTarget::Unstaged).unwrap();
    assert!(patch.contains("diff --git a/a.txt b/a.txt"));
    assert!(patch.contains("+more"));

    fx.repo.stage(&["a.txt"]).unwrap();
    assert!(
        fx.repo
            .diff("a.txt", DiffTarget::Staged)
            .unwrap()
            .contains("+more")
    );
    assert!(
        fx.repo
            .diff("a.txt", DiffTarget::Unstaged)
            .unwrap()
            .is_empty()
    );

    fx.repo.commit("amended", true).unwrap();
    assert_eq!(fx.repo.head_message().unwrap(), "amended");
    assert_eq!(
        fx.repo.git(["rev-list", "--count", "HEAD"]).unwrap().trim(),
        "1"
    );
}

#[test]
fn discard_and_remove_untracked() {
    let fx = Fixture::new();
    fx.write("a.txt", "one\n");
    fx.repo.stage_all().unwrap();
    fx.repo.commit("initial", false).unwrap();

    fx.write("a.txt", "changed\n");
    fx.write("new.txt", "new\n");
    fx.repo.discard(&["a.txt"]).unwrap();
    fx.repo.remove_untracked(&["new.txt"]).unwrap();
    assert!(fx.repo.status().unwrap().entries.is_empty());
}

#[test]
fn deletions_stage_and_hook_failures_surface() {
    let fx = Fixture::new();
    fx.write("a.txt", "one\n");
    fx.repo.stage_all().unwrap();
    fx.repo.commit("initial", false).unwrap();

    fs::remove_file(fx.repo.workdir().join("a.txt")).unwrap();
    fx.repo.stage(&["a.txt"]).unwrap();
    let status = fx.repo.status().unwrap();
    assert!(matches!(
        status.entries[0].status,
        FileStatus::Tracked {
            index: Change::Deleted,
            ..
        }
    ));

    let hook = fx.repo.workdir().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    Command::new("chmod").arg("+x").arg(&hook).status().unwrap();
    let error = fx.repo.commit("delete", false).unwrap_err();
    assert!(error.to_string().contains("lint failed"), "{error}");
}
