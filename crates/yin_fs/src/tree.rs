//! Folders and files as a tree, built from a flat list of paths.

use std::collections::BTreeMap;

/// A file or folder. Folders come before files; names sort case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    /// Relative path from the root, `/`-separated.
    pub path: String,
    pub children: Vec<Node>,
    pub is_dir: bool,
}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    files: Vec<String>,
}

pub fn build_tree(paths: &[String]) -> Vec<Node> {
    let mut root = Dir::default();
    for path in paths {
        let mut dir = &mut root;
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                dir.files.push(part.to_owned());
            } else {
                dir = dir.dirs.entry(part.to_owned()).or_default();
            }
        }
    }
    nodes(root, "")
}

fn nodes(dir: Dir, prefix: &str) -> Vec<Node> {
    let join = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        }
    };
    let mut dirs: Vec<Node> = dir
        .dirs
        .into_iter()
        .map(|(name, child)| {
            let path = join(&name);
            Node {
                children: nodes(child, &path),
                name,
                path,
                is_dir: true,
            }
        })
        .collect();
    let mut files: Vec<Node> = dir
        .files
        .into_iter()
        .map(|name| Node {
            path: join(&name),
            name,
            children: Vec::new(),
            is_dir: false,
        })
        .collect();
    let by_name = |a: &Node, b: &Node| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.name.cmp(&b.name))
    };
    dirs.sort_by(by_name);
    files.sort_by(by_name);
    dirs.extend(files);
    dirs
}
