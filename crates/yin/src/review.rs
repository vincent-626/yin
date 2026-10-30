//! "Since last review": which changed files you've already looked at.
//!
//! Marking a file reviewed records a hash of its working-tree contents. When
//! an agent touches the file again the hash no longer matches and the file
//! is unreviewed again. Marks persist per repository.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`, so saved
/// marks survive upgrades.
pub fn content_hash(contents: Option<&[u8]>) -> u64 {
    let Some(bytes) = contents else {
        // A deleted file.
        return 0x0dd_ba11;
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[derive(Debug, Default)]
pub struct Reviews {
    marks: HashMap<String, u64>,
    file: Option<PathBuf>,
}

fn reviews_file(workdir: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let key = content_hash(Some(workdir.as_os_str().as_encoded_bytes()));
    Some(PathBuf::from(home).join(format!(
        "Library/Application Support/yin/reviews/{key:016x}"
    )))
}

impl Reviews {
    pub fn load(workdir: &Path) -> Self {
        Self::load_from(reviews_file(workdir))
    }

    fn load_from(file: Option<PathBuf>) -> Self {
        let marks = file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .map(|text| {
                text.lines()
                    .filter_map(|line| {
                        let (hash, path) = line.split_once('\t')?;
                        Some((path.to_owned(), u64::from_str_radix(hash, 16).ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { marks, file }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let mut lines: Vec<String> = self
            .marks
            .iter()
            .map(|(path, hash)| format!("{hash:016x}\t{path}"))
            .collect();
        lines.sort();
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, lines.join("\n")));
        if let Err(error) = result {
            eprintln!("yin: couldn't save review marks: {error}");
        }
    }

    pub fn mark(&mut self, path: &str, hash: u64) {
        self.marks.insert(path.to_owned(), hash);
        self.save();
    }

    pub fn unmark(&mut self, path: &str) {
        if self.marks.remove(path).is_some() {
            self.save();
        }
    }

    /// Paths reviewed in their current state.
    pub fn reviewed(&self, hashes: &HashMap<String, u64>) -> HashSet<String> {
        hashes
            .iter()
            .filter(|(path, hash)| self.marks.get(*path) == Some(hash))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Forgets marks for files that are no longer changed, so a later change
    /// to them starts unreviewed and the saved file stays small.
    pub fn prune(&mut self, hashes: &HashMap<String, u64>) {
        let before = self.marks.len();
        self.marks.retain(|path, _| hashes.contains_key(path));
        if self.marks.len() != before {
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_after_review_unreview_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("reviews");
        let mut reviews = Reviews::load_from(Some(file.clone()));
        let mut hashes = HashMap::from([
            ("a.rs".to_owned(), content_hash(Some(b"one"))),
            ("b.rs".to_owned(), content_hash(None)),
        ]);
        reviews.mark("a.rs", hashes["a.rs"]);
        reviews.mark("b.rs", hashes["b.rs"]);
        assert_eq!(reviews.reviewed(&hashes).len(), 2);

        // An agent edits a.rs again.
        hashes.insert("a.rs".to_owned(), content_hash(Some(b"two")));
        assert_eq!(
            reviews.reviewed(&hashes),
            HashSet::from(["b.rs".to_owned()])
        );

        // Marks survive a restart.
        let reloaded = Reviews::load_from(Some(file.clone()));
        assert_eq!(
            reloaded.reviewed(&hashes),
            HashSet::from(["b.rs".to_owned()])
        );

        // Once b.rs is committed it drops out, and a new change starts unreviewed.
        hashes.remove("b.rs");
        reviews.prune(&hashes);
        hashes.insert("b.rs".to_owned(), content_hash(None));
        assert!(reviews.reviewed(&hashes).is_empty());
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(content_hash(Some(b"")), 0xcbf2_9ce4_8422_2325);
        assert_eq!(content_hash(Some(b"a")), 0xaf63_dc4c_8601_ec8c);
    }
}
