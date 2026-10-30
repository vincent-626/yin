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

#[test]
fn git_dirs_of_main_and_linked_worktree() {
    let fx = Fixture::new();
    fx.write("a.txt", "one\n");
    fx.repo.stage_all().unwrap();
    fx.repo.commit("initial", false).unwrap();
    let (git_dir, common_dir) = fx.repo.git_dirs().unwrap();
    assert_eq!(git_dir, common_dir);
    assert!(git_dir.ends_with(".git"));

    let linked = tempfile::tempdir().unwrap();
    let linked_path = linked.path().join("wt");
    fx.repo
        .git([
            "worktree".as_ref(),
            "add".as_ref(),
            "--quiet".as_ref(),
            linked_path.as_os_str(),
        ])
        .unwrap();
    let worktree = Repository::open(&linked_path).unwrap();
    let (wt_git_dir, wt_common_dir) = worktree.git_dirs().unwrap();
    assert_ne!(wt_git_dir, wt_common_dir);
    assert_eq!(wt_common_dir, common_dir.canonicalize().unwrap());
}

fn lines(n: std::ops::RangeInclusive<usize>) -> String {
    n.map(|i| format!("line {i}\n")).collect()
}

fn committed(contents: &str) -> Fixture {
    let fx = Fixture::new();
    fx.write("f.txt", contents);
    fx.repo.stage_all().unwrap();
    fx.repo.commit("initial", false).unwrap();
    fx
}

fn new_pos(line: usize) -> LinePosition {
    LinePosition {
        side: Side::New,
        line,
    }
}

fn old_pos(line: usize) -> LinePosition {
    LinePosition {
        side: Side::Old,
        line,
    }
}

#[test]
fn stages_one_hunk_of_two() {
    let fx = committed(&lines(1..=20));
    // Change line 2 and line 18: two hunks.
    let text = lines(1..=20)
        .replace("line 2\n", "line two\n")
        .replace("line 18\n", "line eighteen\n");
    fx.write("f.txt", &text);
    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap()).unwrap();
    let rows = patch.hunk_rows(new_pos(18));
    fx.repo
        .apply_to_index(&patch.select(&rows, false).unwrap(), false)
        .unwrap();

    let staged = fx.repo.diff("f.txt", DiffTarget::Staged).unwrap();
    assert!(
        staged.contains("+line eighteen") && !staged.contains("+line two"),
        "{staged}"
    );
    let unstaged = fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap();
    assert!(
        unstaged.contains("+line two") && !unstaged.contains("+line eighteen"),
        "{unstaged}"
    );
}

#[test]
fn stages_single_lines_within_a_hunk() {
    let fx = committed(&lines(1..=5));
    // Replace line 3 and add two lines after it, all in one hunk.
    let text = lines(1..=5).replace("line 3\n", "line three\nextra a\nextra b\n");
    fx.write("f.txt", &text);
    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap()).unwrap();
    // Select only "extra a" (new line 4).
    let rows = patch.rows_between(new_pos(4), new_pos(4));
    fx.repo
        .apply_to_index(&patch.select(&rows, false).unwrap(), false)
        .unwrap();

    let index = fx.repo.git(["show", ":f.txt"]).unwrap();
    assert_eq!(index, "line 1\nline 2\nline 3\nextra a\nline 4\nline 5\n");

    // Then the deletion of line 3 alone, selected on the old side.
    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap()).unwrap();
    let rows = patch.rows_between(old_pos(3), old_pos(3));
    fx.repo
        .apply_to_index(&patch.select(&rows, false).unwrap(), false)
        .unwrap();
    let index = fx.repo.git(["show", ":f.txt"]).unwrap();
    assert_eq!(index, "line 1\nline 2\nextra a\nline 4\nline 5\n");
}

#[test]
fn unstages_selected_lines() {
    let fx = committed(&lines(1..=20));
    let text = lines(1..=20)
        .replace("line 2\n", "line two\n")
        .replace("line 18\n", "line eighteen\n");
    fx.write("f.txt", &text);
    fx.repo.stage(&["f.txt"]).unwrap();

    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Staged).unwrap()).unwrap();
    // Unstage the first hunk, selected from its deletion to its addition.
    let rows = patch.rows_between(old_pos(2), new_pos(2));
    assert!(patch.selects_changes(&rows));
    fx.repo
        .apply_to_index(&patch.select(&rows, true).unwrap(), true)
        .unwrap();

    let staged = fx.repo.diff("f.txt", DiffTarget::Staged).unwrap();
    assert!(
        staged.contains("+line eighteen") && !staged.contains("line two"),
        "{staged}"
    );
    let unstaged = fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap();
    assert!(unstaged.contains("+line two"), "{unstaged}");
    // The working tree is untouched.
    assert_eq!(
        fs::read_to_string(fx.repo.workdir().join("f.txt")).unwrap(),
        text
    );
}

#[test]
fn partial_staging_handles_missing_final_newline() {
    let fx = committed("a\nb\nc");
    fx.write("f.txt", "a\nB\nc\nd");
    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap()).unwrap();
    // Stage only b -> B: c stays, and keeps its missing final newline.
    let mut rows = patch.rows_between(old_pos(2), old_pos(2));
    rows.extend(patch.rows_between(new_pos(2), new_pos(2)));
    fx.repo
        .apply_to_index(&patch.select(&rows, false).unwrap(), false)
        .unwrap();
    assert_eq!(fx.repo.git(["show", ":f.txt"]).unwrap(), "a\nB\nc");
}

#[test]
fn replacement_lands_where_its_deleted_line_was() {
    let fx = committed("a\nb\nc\nz\n");
    fx.write("f.txt", "a\nB\nz\n");
    // -b -c +B: replace c alone with B, keeping b before it.
    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap()).unwrap();
    let mut rows = patch.rows_between(old_pos(3), old_pos(3));
    rows.extend(patch.rows_between(new_pos(2), new_pos(2)));
    fx.repo
        .apply_to_index(&patch.select(&rows, false).unwrap(), false)
        .unwrap();
    assert_eq!(fx.repo.git(["show", ":f.txt"]).unwrap(), "a\nb\nB\nz\n");
}

#[test]
fn refuses_partial_new_files_and_empty_selections() {
    let fx = committed("a\n");
    fx.write("f.txt", "a\nb\n");
    let patch = FilePatch::parse(&fx.repo.diff("f.txt", DiffTarget::Unstaged).unwrap()).unwrap();
    let context_only = patch.rows_between(old_pos(1), old_pos(1));
    assert!(!patch.selects_changes(&context_only));
    assert!(patch.select(&context_only, false).is_err());

    fx.write("new.txt", "x\n");
    let patch = FilePatch::parse(&fx.repo.diff("new.txt", DiffTarget::Untracked).unwrap()).unwrap();
    assert!(patch.select(&[0], false).is_err());
}

#[test]
fn log_branches_and_switching() {
    let fx = committed("a\n");
    fx.write("f.txt", "b\n");
    fx.repo.stage_all().unwrap();
    fx.repo.commit("second\n\nwith a body", false).unwrap();

    let log = fx.repo.log(10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].subject, "second");
    assert!(
        log[0].refs.iter().any(|r| r.contains("main")),
        "{:?}",
        log[0].refs
    );
    assert_eq!(
        fx.repo.commit_message(&log[0].oid).unwrap(),
        "second\n\nwith a body"
    );
    assert!(fx.repo.commit_patch(&log[0].oid).unwrap().contains("+b"));
    // The root commit has a patch too.
    assert!(fx.repo.commit_patch(&log[1].oid).unwrap().contains("+a"));

    assert!(fx.repo.create_branch("bad name").is_err());
    fx.repo.create_branch("feature").unwrap();
    let branches = fx.repo.branches().unwrap();
    let names: Vec<_> = branches
        .iter()
        .map(|b| (b.name.as_str(), b.is_head))
        .collect();
    assert!(
        names.contains(&("feature", true)) && names.contains(&("main", false)),
        "{names:?}"
    );

    let main = branches.iter().find(|b| b.name == "main").unwrap();
    fx.repo.switch(main).unwrap();
    assert_eq!(
        fx.repo.status().unwrap().branch.head.as_deref(),
        Some("main")
    );
}

#[test]
fn log_of_unborn_branch_is_empty() {
    let fx = Fixture::new();
    assert!(fx.repo.log(10).unwrap().is_empty());
}

#[test]
fn stash_push_list_pop_and_drop() {
    let fx = committed("a\n");
    fx.write("f.txt", "changed\n");
    fx.write("untracked.txt", "u\n");
    fx.repo.stash_push(Some("wip")).unwrap();
    assert!(fx.repo.status().unwrap().entries.is_empty());

    let stashes = fx.repo.stashes().unwrap();
    assert_eq!(stashes.len(), 1);
    assert!(stashes[0].message.contains("wip"), "{:?}", stashes[0]);
    let patch = fx.repo.stash_patch(&stashes[0]).unwrap();
    assert!(
        patch.contains("+changed") && patch.contains("+u"),
        "{patch}"
    );

    fx.repo.stash_pop(&stashes[0]).unwrap();
    assert_eq!(fx.repo.status().unwrap().entries.len(), 2);
    assert!(fx.repo.stashes().unwrap().is_empty());

    fx.repo.stash_push(None).unwrap();
    fx.repo.stash_drop(&fx.repo.stashes().unwrap()[0]).unwrap();
    assert!(fx.repo.stashes().unwrap().is_empty());
}

#[test]
fn push_sets_upstream_then_fetch_and_pull_fast_forward() {
    let remote = tempfile::tempdir().unwrap();
    Command::new("git")
        .args(["init", "--quiet", "--bare", "--initial-branch=main"])
        .current_dir(remote.path())
        .status()
        .unwrap();
    let remote_url = remote.path().to_str().unwrap();

    let a = committed("a\n");
    a.repo.git(["remote", "add", "origin", remote_url]).unwrap();
    let branch = a.repo.status().unwrap().branch;
    assert!(branch.upstream.is_none());
    a.repo.push(&branch).unwrap();
    let branch = a.repo.status().unwrap().branch;
    assert_eq!(branch.upstream.as_deref(), Some("origin/main"));

    // A second clone pushes a commit; the first fetches and fast-forwards.
    let b_dir = tempfile::tempdir().unwrap();
    let b_path = b_dir.path().join("b");
    Command::new("git")
        .args(["clone", "--quiet", remote_url])
        .arg(&b_path)
        .status()
        .unwrap();
    let b = Repository::open(&b_path).unwrap();
    b.git(["config", "user.name", "B"]).unwrap();
    b.git(["config", "user.email", "b@example.com"]).unwrap();
    b.git(["config", "commit.gpgsign", "false"]).unwrap();
    fs::write(b_path.join("f.txt"), "from b\n").unwrap();
    b.stage_all().unwrap();
    b.commit("from b", false).unwrap();
    b.push(&b.status().unwrap().branch).unwrap();

    a.repo.fetch().unwrap();
    assert_eq!(a.repo.status().unwrap().branch.behind, 1);
    a.repo.pull().unwrap();
    assert_eq!(a.repo.status().unwrap().branch.behind, 0);
    assert_eq!(
        fs::read_to_string(a.repo.workdir().join("f.txt")).unwrap(),
        "from b\n"
    );

    // Remote branches without a local copy are offered for switching.
    b.create_branch("topic").unwrap();
    b.push(&b.status().unwrap().branch).unwrap();
    a.repo.fetch().unwrap();
    let topic = a
        .repo
        .branches()
        .unwrap()
        .into_iter()
        .find(|br| br.local_name() == "topic")
        .unwrap();
    assert!(topic.is_remote);
    a.repo.switch(&topic).unwrap();
    assert_eq!(
        a.repo.status().unwrap().branch.head.as_deref(),
        Some("topic")
    );
}

#[test]
fn resolves_a_merge_conflict() {
    let fx = committed("base\n");
    fx.repo.create_branch("other").unwrap();
    fx.write("f.txt", "theirs\n");
    fx.repo.stage_all().unwrap();
    fx.repo.commit("theirs", false).unwrap();
    fx.repo.git(["switch", "main"]).unwrap();
    fx.write("f.txt", "ours\n");
    fx.repo.stage_all().unwrap();
    fx.repo.commit("ours", false).unwrap();
    assert!(fx.repo.git(["merge", "other"]).is_err());
    assert_eq!(fx.repo.status().unwrap().conflicted().count(), 1);
    assert!(fx.repo.read_file("f.txt").unwrap().contains("<<<<<<<"));

    fx.repo
        .resolve_conflict("f.txt", "ours and theirs\n")
        .unwrap();
    assert_eq!(fx.repo.status().unwrap().conflicted().count(), 0);
    assert!(fx.repo.merge_in_progress().unwrap());
    fx.repo.abort_merge().unwrap();
    assert!(!fx.repo.merge_in_progress().unwrap());
    assert_eq!(fx.repo.read_file("f.txt").unwrap(), "ours\n");
}

#[test]
fn lists_worktrees() {
    let fx = committed("a\n");
    let linked = tempfile::tempdir().unwrap();
    let path = linked.path().join("feature wt");
    fx.repo
        .git([
            "worktree".as_ref(),
            "add".as_ref(),
            "-b".as_ref(),
            "feature".as_ref(),
            path.as_os_str(),
        ])
        .unwrap();
    let worktrees = fx.repo.worktrees().unwrap();
    assert_eq!(worktrees.len(), 2);
    assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    assert_eq!(worktrees[1].branch.as_deref(), Some("feature"));
    assert_eq!(
        worktrees[1].path.canonicalize().unwrap(),
        path.canonicalize().unwrap()
    );
    assert!(worktrees.iter().all(|w| w.oid.is_some() && !w.is_bare));
}
