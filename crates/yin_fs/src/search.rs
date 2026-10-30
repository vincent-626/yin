//! Project search with ripgrep's searcher.

use std::ops::Range;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::Result;
use grep_matcher::Matcher as _;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    /// Treat the query as a regular expression rather than literal text.
    pub regex: bool,
    /// Match case exactly. Otherwise case is smart: sensitive only when the
    /// query has an uppercase letter.
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: String,
    /// One-based.
    pub line: u64,
    /// The line, without its terminator.
    pub text: String,
    /// Byte ranges of each match within `text`.
    pub ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub matches: Vec<SearchMatch>,
    /// More matches exist than were collected.
    pub truncated: bool,
}

fn matcher(query: &str, options: SearchOptions) -> Result<RegexMatcher> {
    let mut builder = RegexMatcherBuilder::new();
    builder
        .fixed_strings(!options.regex)
        .case_insensitive(false)
        .case_smart(!options.case_sensitive);
    Ok(builder.build(query)?)
}

/// Searches `files` (relative to `root`) for `query`, stopping after `limit`
/// matches or as soon as `cancel` is set. Results are in file order.
pub fn search(
    root: &Path,
    files: &[String],
    query: &str,
    options: SearchOptions,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<SearchResults> {
    if query.is_empty() {
        return Ok(SearchResults::default());
    }
    let matcher = matcher(query, options)?;
    let found = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = files.len().div_ceil(threads).max(1);
    // Per-chunk results keep file order when stitched back together.
    let chunks: Vec<Mutex<Vec<SearchMatch>>> = files
        .chunks(chunk)
        .map(|_| Mutex::new(Vec::new()))
        .collect();
    std::thread::scope(|scope| {
        for (paths, out) in files.chunks(chunk).zip(&chunks) {
            let matcher = &matcher;
            let found = &found;
            scope.spawn(move || {
                let mut searcher = SearcherBuilder::new()
                    .binary_detection(BinaryDetection::quit(b'\0'))
                    .line_number(true)
                    .build();
                let mut local = Vec::new();
                for path in paths {
                    if cancel.load(Ordering::Relaxed) || found.load(Ordering::Relaxed) >= limit {
                        break;
                    }
                    let full = root.join(path);
                    // Unreadable files (permissions, races) are skipped.
                    let _skipped = searcher.search_path(
                        matcher,
                        &full,
                        UTF8(|line, text| {
                            let text = text.trim_end_matches(['\n', '\r']);
                            let mut ranges = Vec::new();
                            matcher.find_iter(text.as_bytes(), |m| {
                                ranges.push(m.start()..m.end());
                                true
                            })?;
                            local.push(SearchMatch {
                                path: path.clone(),
                                line,
                                text: text.to_owned(),
                                ranges,
                            });
                            let total = found.fetch_add(1, Ordering::Relaxed) + 1;
                            Ok(total < limit && !cancel.load(Ordering::Relaxed))
                        }),
                    );
                }
                *out.lock().expect("search lock") = local;
            });
        }
    });
    let mut matches: Vec<SearchMatch> = chunks
        .into_iter()
        .flat_map(|chunk| chunk.into_inner().expect("search lock"))
        .collect();
    let truncated = matches.len() >= limit;
    matches.truncate(limit);
    Ok(SearchResults { matches, truncated })
}
