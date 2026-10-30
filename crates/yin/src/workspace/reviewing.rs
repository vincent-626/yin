//! Marking changes reviewed, and hiding what you've already seen.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::Context;
use yin_git::Status;

use super::{Section, Workspace};
use crate::review::content_hash;

/// A hash of each changed file's working-tree contents; run off the main thread.
pub(super) fn hash_changed_files(
    workdir: &std::path::Path,
    status: &Status,
) -> HashMap<String, u64> {
    status
        .entries
        .iter()
        .map(|entry| {
            let contents = std::fs::read(workdir.join(&entry.path)).ok();
            (entry.path.clone(), content_hash(contents.as_deref()))
        })
        .collect()
}

impl Workspace {
    pub(super) fn install_review_hashes(&mut self, hashes: HashMap<String, u64>) {
        self.reviews.prune(&hashes);
        self.reviewed = Arc::new(self.reviews.reviewed(&hashes));
        self.review_hashes = hashes;
    }

    /// Paths hidden from the Changes list.
    pub(super) fn hidden_paths(&self) -> HashSet<String> {
        if self.hide_reviewed {
            (*self.reviewed).clone()
        } else {
            HashSet::new()
        }
    }

    pub(super) fn toggle_reviewed(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key() else {
            return;
        };
        if matches!(key.section, Section::Stashes) {
            return;
        }
        if self.reviewed.contains(&key.path) {
            self.reviews.unmark(&key.path);
        } else if let Some(hash) = self.review_hashes.get(&key.path) {
            self.reviews.mark(&key.path, *hash);
        }
        self.reviewed = Arc::new(self.reviews.reviewed(&self.review_hashes));
        // Move on, like staging does, so reviewing is one key per file.
        let next = super::changes::step(&self.rows, self.selected, 1);
        self.rebuild_rows(cx);
        if !self.hide_reviewed {
            self.select(next, cx);
        }
    }

    pub(super) fn mark_all_reviewed(&mut self, cx: &mut Context<Self>) {
        for (path, hash) in &self.review_hashes {
            if !self.reviewed.contains(path) {
                self.reviews.mark(path, *hash);
            }
        }
        self.reviewed = Arc::new(self.reviews.reviewed(&self.review_hashes));
        self.rebuild_rows(cx);
    }

    pub(super) fn toggle_hide_reviewed(&mut self, cx: &mut Context<Self>) {
        self.hide_reviewed = !self.hide_reviewed;
        self.rebuild_rows(cx);
    }
}
