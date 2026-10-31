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
    // Typing goes to the switcher's search.
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("zz", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let shell = cx.update(crate::windows::frontmost).unwrap().0;
    assert_eq!(cx.update(|cx| shell.read(cx).switcher_query(cx)), "zz");
    cx.update_window(window, |_, window, cx| window.press("escape", cx))
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

#[gpui_kit::test]
fn diffs_a_run_of_commits_and_a_branch(cx: &mut TestAppContext) {
    let dir = fixture();
    let path = dir.path();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "second"]);
    git(path, &["switch", "--quiet", "-c", "feature"]);
    std::fs::write(path.join("c.txt"), "sea\n").unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "third"]);
    let repo = Repository::open(path).unwrap();
    let (window, workspace) = open(cx, repo);
    cx.run_until_parked();

    // The newest commit alone, then it and the one before.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-3", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(workspace.read(cx).history_files(cx), ["c.txt"]));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("shift-down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            workspace.read(cx).history_files(cx),
            ["a.txt", "b.txt", "c.txt"]
        )
    });
    // Plain down moves on to the next commit alone: the first.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("down", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(workspace.read(cx).history_files(cx), ["a.txt"]));

    // Comparing with main: only feature's commit, and its change.
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.compare_for_test("main", window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let workspace = workspace.read(cx);
        assert_eq!(workspace.history_commit_count(), 1);
        assert_eq!(workspace.history_files(cx), ["c.txt"]);
    });
}

#[gpui_kit::test]
fn writes_a_review_note_and_copies_it_as_a_prompt(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Yin Test"]);
    git(path, &["config", "user.email", "yin@example.com"]);
    std::fs::write(path.join("f.txt"), numbered(1..=10)).unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "initial"]);
    std::fs::write(
        path.join("f.txt"),
        numbered(1..=10).replace("line 5\n", "line five\n"),
    )
    .unwrap();

    let (window, workspace) = open(cx, Repository::open(path).unwrap());
    cx.run_until_parked();
    cx.update(|cx| {
        let diff = workspace.read(cx).diff_state();
        diff.update(cx, |diff, cx| {
            diff.set_selected_lines(
                Some(DiffLineRange::new("f.txt", DiffSide::Modified, 5, 5)),
                cx,
            )
        });
    });
    // n opens the note box with focus; typing there doesn't trigger diff keys.
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(actions::AddNote.boxed_clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.input("Spell numbers as digits", cx);
        window.press("cmd-enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let notes = workspace.read(cx).notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "Spell numbers as digits");
        assert_eq!(notes[0].code, ["line five"]);
        assert_eq!(notes[0].location(), "f.txt:5");
    });
    // Typing "s" and space in the note didn't stage anything.
    assert_eq!(git(path, &["diff", "--cached"]), "");

    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(actions::CopyReviewPrompt.boxed_clone(), cx);
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert!(copied.contains("1. f.txt:5\n"), "{copied}");
    assert!(copied.contains("line five") && copied.contains("Spell numbers as digits"));
}

#[gpui_kit::test]
fn notes_follow_agent_edits_and_go_outdated(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    git(path, &["config", "user.name", "Yin Test"]);
    git(path, &["config", "user.email", "yin@example.com"]);
    std::fs::write(path.join("f.txt"), numbered(1..=10)).unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "--quiet", "-m", "initial"]);
    let edited = numbered(1..=10).replace("line 5\n", "line five\n");
    std::fs::write(path.join("f.txt"), &edited).unwrap();

    let (window, workspace) = open(cx, Repository::open(path).unwrap());
    cx.run_until_parked();
    cx.update(|cx| {
        let diff = workspace.read(cx).diff_state();
        diff.update(cx, |diff, cx| {
            diff.set_selected_lines(
                Some(DiffLineRange::new("f.txt", DiffSide::Modified, 5, 5)),
                cx,
            )
        });
    });
    let act = |cx: &mut TestAppContext, action: Box<dyn gpui_kit::Action>| {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(action, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    act(cx, actions::AddNote.boxed_clone());
    cx.update_window(window.into(), |_, window, cx| {
        window.input("Use a digit", cx);
        window.press("cmd-enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    // The agent adds two lines above: the note moves down with its line.
    std::fs::write(path.join("f.txt"), format!("new a\nnew b\n{edited}")).unwrap();
    act(cx, actions::Refresh.boxed_clone());
    cx.update(|cx| {
        let note = &workspace.read(cx).notes()[0];
        assert_eq!((note.start, note.end, note.outdated), (7, 7, false));
    });

    // The agent rewrites the line: the note is outdated.
    std::fs::write(
        path.join("f.txt"),
        numbered(1..=10).replace("line 5\n", "line 5!\n"),
    )
    .unwrap();
    act(cx, actions::Refresh.boxed_clone());
    cx.update(|cx| assert!(workspace.read(cx).notes()[0].outdated));
}
