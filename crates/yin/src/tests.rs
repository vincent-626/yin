//! UI tests: the production Workspace in a headless window, driven by real
//! key events, against a throwaway repository.

use std::path::Path;
use std::process::Command;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, base::Root, px, size,
};
use yin_git::Repository;

use crate::workspace::{self, Workspace, actions};
use gpui_kit::Action as _;
use gpui_kit::component::diff::{DiffConflictResolution, DiffLineRange, DiffSide};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repository with one modified tracked file and one untracked file.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Yin Test"]);
    git(path, &["config", "user.email", "yin@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join("a.txt"), "one\n").unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "initial"]);
    std::fs::write(path.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(path.join("b.txt"), "new\n").unwrap();
    dir
}

fn open(cx: &mut TestAppContext, repo: Repository) -> (WindowHandle<Root>, Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        workspace::init(cx);
        let (window, workspace) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Workspace::new(repo, window, cx)),
        )
        .expect("open test window");
        (window.downcast::<Root>().expect("Base Root"), workspace)
    })
}

#[gpui_kit::test]
fn stages_and_commits_from_the_keyboard(cx: &mut TestAppContext) {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let (window, workspace) = open(cx, repo);
    cx.run_until_parked();

    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert_eq!(workspace.selected_path().as_deref(), Some("a.txt"));
    });

    // space stages the selected file.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("space", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        git(dir.path(), &["diff", "--cached", "--name-only"]),
        "a.txt\n"
    );

    // c focuses the message; cmd-enter commits.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("c", cx);
        window.input("Add a second line", cx);
        window.press("cmd-enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(
        git(dir.path(), &["log", "-1", "--format=%s"]),
        "Add a second line\n"
    );
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "?? b.txt\n");
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert!(workspace.commit_message_text(cx).is_empty());
        assert_eq!(workspace.selected_path().as_deref(), Some("b.txt"));
    });
}

#[gpui_kit::test]
fn commit_is_refused_without_staged_changes(cx: &mut TestAppContext) {
    let dir = fixture();
    let repo = Repository::open(dir.path()).unwrap();
    let (window, _) = open(cx, repo);
    cx.run_until_parked();

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("c", cx);
        window.input("Nothing staged", cx);
        window.press("cmd-enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(git(dir.path(), &["rev-list", "--count", "HEAD"]), "1\n");
}

fn numbered(range: std::ops::RangeInclusive<usize>) -> String {
    range.map(|i| format!("line {i}\n")).collect()
}

#[gpui_kit::test]
fn stages_then_unstages_selected_lines_from_the_diff(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Yin Test"]);
    git(path, &["config", "user.email", "yin@example.com"]);
    std::fs::write(path.join("f.txt"), numbered(1..=20)).unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "initial"]);
    let changed = numbered(1..=20)
        .replace("line 2\n", "line two\n")
        .replace("line 18\n", "line eighteen\n");
    std::fs::write(path.join("f.txt"), &changed).unwrap();

    let (window, workspace) = open(cx, Repository::open(path).unwrap());
    cx.run_until_parked();
    let select = |cx: &mut TestAppContext, side, line| {
        cx.update(|cx| {
            let diff = workspace.read(cx).diff_state();
            diff.update(cx, |diff, cx| {
                diff.set_selected_lines(Some(DiffLineRange::new("f.txt", side, line, line)), cx)
            });
        });
    };
    let dispatch = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    // Stage only the added "line eighteen".
    select(cx, DiffSide::Modified, 18);
    dispatch(cx, actions::StageLines.boxed_clone());
    let staged = git(path, &["diff", "--cached"]);
    assert!(
        staged.contains("+line eighteen") && !staged.contains("line two"),
        "{staged}"
    );
    assert!(
        !staged.contains("-line 18"),
        "only the addition was selected: {staged}"
    );

    // Part of the hunk is still unstaged, so the hunk stages.
    select(cx, DiffSide::Modified, 18);
    dispatch(cx, actions::StageHunk.boxed_clone());
    let staged = git(path, &["diff", "--cached"]);
    assert!(
        staged.contains("-line 18") && staged.contains("+line eighteen"),
        "{staged}"
    );
    // All of it is staged now, so the same key unstages it.
    select(cx, DiffSide::Modified, 18);
    dispatch(cx, actions::StageHunk.boxed_clone());
    assert_eq!(git(path, &["diff", "--cached"]), "");
    // The working tree never changes.
    assert_eq!(
        std::fs::read_to_string(path.join("f.txt")).unwrap(),
        changed
    );
}

#[gpui_kit::test]
fn resolves_a_conflict_then_marks_it_resolved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Yin Test"]);
    git(path, &["config", "user.email", "yin@example.com"]);
    std::fs::write(path.join("f.txt"), "base\n").unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "base"]);
    git(path, &["switch", "--quiet", "-c", "other"]);
    std::fs::write(path.join("f.txt"), "theirs\n").unwrap();
    git(path, &["commit", "--quiet", "-am", "theirs"]);
    git(path, &["switch", "--quiet", "main"]);
    std::fs::write(path.join("f.txt"), "ours\n").unwrap();
    git(path, &["commit", "--quiet", "-am", "ours"]);
    let merge = Command::new("git")
        .args(["merge", "other"])
        .current_dir(path)
        .output()
        .unwrap();
    assert!(!merge.status.success());

    let (window, workspace) = open(cx, Repository::open(path).unwrap());
    cx.run_until_parked();
    let press_space = |cx: &mut TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press("space", cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    // Unresolved: space refuses, and the file keeps its markers unstaged.
    press_space(cx);
    assert_eq!(
        git(path, &["diff", "--name-only", "--diff-filter=U"]),
        "f.txt\n"
    );

    // Accept the incoming side, as the conflict's button does, then mark resolved.
    cx.update(|cx| {
        let diff = workspace.read(cx).diff_state();
        diff.update(cx, |diff, cx| {
            diff.resolve_conflict("f.txt", 0, Some(DiffConflictResolution::Incoming), cx)
        });
    });
    press_space(cx);
    assert_eq!(git(path, &["diff", "--name-only", "--diff-filter=U"]), "");
    assert_eq!(
        std::fs::read_to_string(path.join("f.txt")).unwrap(),
        "theirs\n"
    );
    assert_eq!(git(path, &["show", ":f.txt"]), "theirs\n");
}

#[gpui_kit::test]
fn commit_message_accepts_brackets(cx: &mut TestAppContext) {
    let dir = fixture();
    let (window, workspace) = open(cx, Repository::open(dir.path()).unwrap());
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("c", cx);
        window.input("[ui] ", cx);
        window.press("[", cx);
        window.press("]", cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(workspace.read(cx).commit_message_text(cx), "[ui] []");
    });
}

#[gpui_kit::test]
fn opens_edits_and_saves_a_file(cx: &mut TestAppContext) {
    let dir = fixture();
    let (window, workspace) = open(cx, Repository::open(dir.path()).unwrap());
    cx.run_until_parked();
    let keys = |cx: &mut TestAppContext, f: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };

    keys(cx, &|window, cx| {
        window.press("cmd-p", cx);
    });
    keys(cx, &|window, cx| {
        window.input("a.txt", cx);
    });
    keys(cx, &|window, cx| {
        window.press("enter", cx);
    });
    // The editor has focus with the cursor at the start.
    keys(cx, &|window, cx| {
        window.input("zero\n", cx);
    });
    cx.update(|cx| assert_eq!(workspace.read(cx).unsaved_files(), ["a.txt"]));

    keys(cx, &|window, cx| {
        window.press("cmd-s", cx);
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "zero\none\ntwo\n"
    );
    cx.update(|cx| assert!(workspace.read(cx).unsaved_files().is_empty()));
}

#[gpui_kit::test]
fn one_window_per_repository(cx: &mut TestAppContext) {
    let (a, b) = (fixture(), fixture());
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shell::init(cx);
        workspace::init(cx);
    });
    let open = |cx: &mut TestAppContext, path: &Path| {
        let path = path.to_owned();
        cx.update(|cx| crate::windows::open(Some(path), cx));
        cx.run_until_parked();
    };
    let windows = |cx: &mut TestAppContext| cx.update(|cx| cx.windows().len());

    open(cx, a.path());
    assert_eq!(windows(cx), 1);
    // A subfolder of an open repository finds its window too.
    std::fs::create_dir(a.path().join("sub")).unwrap();
    open(cx, &a.path().join("sub"));
    assert_eq!(windows(cx), 1);
    open(cx, b.path());
    assert_eq!(windows(cx), 2);
    let repos = cx.update(crate::windows::open_repos);
    assert_eq!(repos.len(), 2);
}

#[gpui_kit::test]
fn switcher_and_welcome_open_repositories(cx: &mut TestAppContext) {
    let (a, b) = (fixture(), fixture());
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shell::init(cx);
        workspace::init(cx);
    });
    cx.update(|cx| crate::windows::open(Some(a.path().to_owned()), cx));
    cx.run_until_parked();
    // The switcher opens from inside its window's Shell.
    let window = cx.update(|cx| cx.windows()[0]);
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(crate::shell::SwitchRepository.boxed_clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();

    // A welcome window takes the repository picked from it.
    cx.update(|cx| crate::windows::open(None, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    let welcome = cx.update(crate::windows::frontmost).unwrap().1;
    let path = b.path().to_owned();
    cx.update_window(welcome, |_, _, cx| crate::windows::open(Some(path), cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    assert_eq!(cx.update(crate::windows::open_repos).len(), 2);
}
