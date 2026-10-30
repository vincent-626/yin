//! Fuzzy path matching for the file finder, with nucleo (Helix's matcher).

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathMatch {
    /// Index into the searched paths.
    pub index: usize,
    pub score: u32,
    /// Character positions in the path that matched, for highlighting.
    pub positions: Vec<u32>,
}

/// The best `limit` matches for `query`, best first. An empty query matches
/// nothing; callers show recent files instead.
pub fn match_paths(paths: &[String], query: &str, limit: usize) -> Vec<PathMatch> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buffer = Vec::new();
    let mut scored: Vec<(u32, usize)> = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let haystack = Utf32Str::new(path, &mut buffer);
            pattern
                .score(haystack, &mut matcher)
                .map(|score| (score, index))
        })
        .collect();
    // Higher score first; among equals, shorter paths are usually what you meant.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(paths[a.1].len().cmp(&paths[b.1].len()))
            .then(paths[a.1].cmp(&paths[b.1]))
    });
    scored.truncate(limit);
    scored
        .into_iter()
        .map(|(score, index)| {
            let mut positions = Vec::new();
            let haystack = Utf32Str::new(&paths[index], &mut buffer);
            pattern.indices(haystack, &mut matcher, &mut positions);
            positions.sort_unstable();
            positions.dedup();
            PathMatch {
                index,
                score,
                positions,
            }
        })
        .collect()
}
