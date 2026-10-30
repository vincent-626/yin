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

use crate::workspace::{self, Workspace};

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
