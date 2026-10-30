use std::fs;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use super::*;

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .status()
        .unwrap();
    for (path, contents) in [
        (".gitignore", "target/\n*.log\n"),
        ("README.md", "# Demo\nfind me\n"),
        (
            "src/main.rs",
            "fn main() {\n    println!(\"Find me\");\n}\n",
        ),
        ("src/lib.rs", "pub fn find() {}\n"),
        ("src/ui/view.rs", "// nothing here\n"),
        ("target/debug/out", "find me\n"),
        ("build.log", "find me\n"),
        ("data.bin", "find\0me"),
    ] {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, contents).unwrap();
    }
    dir
}

#[test]
fn scan_respects_gitignore_and_skips_git() {
    let dir = fixture();
    let files = scan(dir.path()).unwrap();
    assert_eq!(
        files,
        [
            ".gitignore",
            "README.md",
            "data.bin",
            "src/lib.rs",
            "src/main.rs",
            "src/ui/view.rs"
        ]
    );
}

#[test]
fn tree_puts_folders_first() {
    let files: Vec<String> = ["b.txt", "A.txt", "src/z.rs", "src/ui/v.rs", "docs/x.md"]
        .map(String::from)
        .to_vec();
    let tree = build_tree(&files);
    let names: Vec<&str> = tree.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["docs", "src", "A.txt", "b.txt"]);
    let src = &tree[1];
    assert_eq!(src.path, "src");
    assert_eq!(src.children[0].path, "src/ui");
    assert_eq!(src.children[0].children[0].path, "src/ui/v.rs");
    assert_eq!(src.children[1].path, "src/z.rs");
}

#[test]
fn fuzzy_prefers_tight_filename_matches() {
    let paths: Vec<String> = [
        "crates/yin/src/workspace.rs",
        "crates/yin/src/workspace/history.rs",
        "docs/PLAN.md",
        "crates/yin_git/src/status.rs",
    ]
    .map(String::from)
    .to_vec();
    let matches = match_paths(&paths, "wsp", 10);
    assert_eq!(paths[matches[0].index], "crates/yin/src/workspace.rs");
    let matches = match_paths(&paths, "hist", 10);
    assert_eq!(matches.len(), 1);
    assert_eq!(
        paths[matches[0].index],
        "crates/yin/src/workspace/history.rs"
    );
    assert!(!matches[0].positions.is_empty());
    assert!(match_paths(&paths, "  ", 10).is_empty());
    assert!(match_paths(&paths, "zzzz", 10).is_empty());
}

#[test]
fn search_is_smart_case_literal_and_skips_binaries() {
    let dir = fixture();
    let files = scan(dir.path()).unwrap();
    let cancel = AtomicBool::new(false);

    let results = search(
        dir.path(),
        &files,
        "find me",
        SearchOptions::default(),
        100,
        &cancel,
    )
    .unwrap();
    let hits: Vec<(&str, u64)> = results
        .matches
        .iter()
        .map(|m| (m.path.as_str(), m.line))
        .collect();
    assert_eq!(hits, [("README.md", 2), ("src/main.rs", 2)]);
    let main = &results.matches[1];
    assert_eq!(&main.text[main.ranges[0].clone()], "Find me");

    // An uppercase letter makes the search case-sensitive.
    let results = search(
        dir.path(),
        &files,
        "Find",
        SearchOptions::default(),
        100,
        &cancel,
    )
    .unwrap();
    assert_eq!(results.matches.len(), 1);

    // Literal by default: the dot is not a wildcard.
    let results = search(
        dir.path(),
        &files,
        "fn.main",
        SearchOptions::default(),
        100,
        &cancel,
    )
    .unwrap();
    assert!(results.matches.is_empty());
    let regex = SearchOptions {
        regex: true,
        ..Default::default()
    };
    let results = search(dir.path(), &files, "fn.main", regex, 100, &cancel).unwrap();
    assert_eq!(results.matches.len(), 1);
    assert!(search(dir.path(), &files, "(", regex, 100, &cancel).is_err());
}

#[test]
fn search_stops_at_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<String> = (0..50).map(|i| format!("f{i:02}.txt")).collect();
    for file in &files {
        fs::write(dir.path().join(file), "hit\nhit\n").unwrap();
    }
    let cancel = AtomicBool::new(false);
    let results = search(
        dir.path(),
        &files,
        "hit",
        SearchOptions::default(),
        10,
        &cancel,
    )
    .unwrap();
    assert_eq!(results.matches.len(), 10);
    assert!(results.truncated);
}

/// `YIN_BENCH_REPO=path cargo test -p yin_fs --release -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_large_repo() {
    let Ok(root) = std::env::var("YIN_BENCH_REPO") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let start = std::time::Instant::now();
    let files = scan(&root).unwrap();
    println!("scan: {} files in {:?}", files.len(), start.elapsed());
    let start = std::time::Instant::now();
    let tree = build_tree(&files);
    println!("tree: {} roots in {:?}", tree.len(), start.elapsed());
    for query in ["wsp", "gpui/src/window", "editor.rs"] {
        let start = std::time::Instant::now();
        let matches = match_paths(&files, query, 50);
        println!(
            "fuzzy {query:?}: {} in {:?}",
            matches.len(),
            start.elapsed()
        );
    }
    let cancel = AtomicBool::new(false);
    let start = std::time::Instant::now();
    let results = search(
        &root,
        &files,
        "fn render",
        SearchOptions::default(),
        5000,
        &cancel,
    )
    .unwrap();
    println!(
        "search: {} matches in {:?}",
        results.matches.len(),
        start.elapsed()
    );
}
