//! The working tree as files: scanning, the folder tree, fuzzy path matching
//! and project search. No UI dependencies.

mod fuzzy;
mod search;
mod tree;

use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use ignore::{WalkBuilder, WalkState};

pub use fuzzy::{PathMatch, match_paths};
pub use search::{SearchMatch, SearchOptions, SearchResults, search};
pub use tree::{Node, build_tree};

/// Every file under `root` that git would not ignore, as sorted `/`-separated
/// relative paths. Dotfiles are included; `.git` is not.
pub fn scan(root: &Path) -> Result<Vec<String>> {
    let files = Mutex::new(Vec::new());
    WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build_parallel()
        .run(|| {
            let files = &files;
            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if entry.file_type().is_some_and(|kind| !kind.is_dir())
                    && let Ok(relative) = entry.path().strip_prefix(root)
                    && let Some(relative) = relative.to_str()
                {
                    files
                        .lock()
                        .expect("scan lock")
                        .push(relative.replace('\\', "/"));
                }
                WalkState::Continue
            })
        });
    let mut files = files.into_inner().expect("scan lock");
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests;
