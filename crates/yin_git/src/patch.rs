//! Partial patches: stage or unstage some lines of a file's diff.
//!
//! The approach is lazygit's. To stage selected lines from an unstaged diff,
//! the patch applies to the index (the old side): unselected deletions become
//! context and unselected additions are dropped. To unstage from a staged
//! diff, the patch is applied in reverse to the index (the new side):
//! unselected additions become context and unselected deletions are dropped.

use anyhow::{Result, bail};

/// One side of a diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Old,
    New,
}

/// A line on one side, one-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinePosition {
    pub side: Side,
    pub line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Context,
    Deletion,
    Addition,
}

/// A line of a hunk, in patch order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    text: String,
    /// A following `\ No newline at end of file` marker.
    no_newline: bool,
}

#[derive(Clone, Debug)]
struct Hunk {
    old_start: usize,
    new_start: usize,
    /// Text after the second `@@`, such as the enclosing function.
    section: String,
    rows: Vec<Row>,
}

/// A single file's unified patch, as `git diff` prints it.
#[derive(Clone, Debug)]
pub struct FilePatch {
    header: Vec<String>,
    hunks: Vec<Hunk>,
}

impl FilePatch {
    pub fn parse(patch: &str) -> Result<Self> {
        let mut header = Vec::new();
        let mut hunks: Vec<Hunk> = Vec::new();
        let (mut old_line, mut new_line) = (0, 0);
        for line in patch.lines() {
            if let Some(rest) = line.strip_prefix("@@ ") {
                let Some((ranges, section)) = rest.split_once(" @@") else {
                    bail!("malformed hunk header: {line}");
                };
                let mut parts = ranges.split(' ');
                let (Some(old), Some(new)) = (parts.next(), parts.next()) else {
                    bail!("malformed hunk header: {line}");
                };
                let old_start = parse_start(old.strip_prefix('-'), line)?;
                let new_start = parse_start(new.strip_prefix('+'), line)?;
                old_line = old_start.max(1);
                new_line = new_start.max(1);
                hunks.push(Hunk {
                    old_start,
                    new_start,
                    section: section.to_owned(),
                    rows: Vec::new(),
                });
                continue;
            }
            let Some(hunk) = hunks.last_mut() else {
                if line.starts_with("diff --git") && !header.is_empty() {
                    bail!("expected a single file's patch");
                }
                header.push(line.to_owned());
                continue;
            };
            let (kind, text) = match line.split_at_checked(1) {
                Some((" ", text)) => (RowKind::Context, text),
                Some(("-", text)) => (RowKind::Deletion, text),
                Some(("+", text)) => (RowKind::Addition, text),
                Some(("\\", _)) => {
                    if let Some(row) = hunk.rows.last_mut() {
                        row.no_newline = true;
                    }
                    continue;
                }
                _ if line.starts_with("diff --git") => bail!("expected a single file's patch"),
                // An empty context line, from tools that strip trailing spaces.
                _ => (RowKind::Context, ""),
            };
            let (old, new) = match kind {
                RowKind::Context => (Some(old_line), Some(new_line)),
                RowKind::Deletion => (Some(old_line), None),
                RowKind::Addition => (None, Some(new_line)),
            };
            old_line += old.is_some() as usize;
            new_line += new.is_some() as usize;
            hunk.rows.push(Row {
                kind,
                old_line: old,
                new_line: new,
                text: text.to_owned(),
                no_newline: false,
            });
        }
        Ok(Self { header, hunks })
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.hunks.iter().flat_map(|hunk| &hunk.rows)
    }

    fn row_index(&self, position: LinePosition) -> Option<usize> {
        self.rows().position(|row| match position.side {
            Side::Old => row.old_line == Some(position.line),
            Side::New => row.new_line == Some(position.line),
        })
    }

    /// Where line `line` of side `from` sits on the other side, or `None`
    /// when this patch deletes or adds it.
    pub fn map_line(&self, from: Side, line: usize) -> Option<usize> {
        let on = |row: &Row, side: Side| match side {
            Side::Old => row.old_line,
            Side::New => row.new_line,
        };
        let to = match from {
            Side::Old => Side::New,
            Side::New => Side::Old,
        };
        let mut drift: isize = 0;
        for hunk in &self.hunks {
            let start = match from {
                Side::Old => hunk.old_start,
                Side::New => hunk.new_start,
            };
            let before = match hunk.rows.iter().find_map(|row| on(row, from)) {
                Some(first) => line < first,
                // A hunk with nothing on this side sits after line `start`.
                None => line <= start,
            };
            if before {
                return Some(line.saturating_add_signed(drift));
            }
            if let Some(row) = hunk.rows.iter().find(|row| on(row, from) == Some(line)) {
                return on(row, to);
            }
            let count = |side| {
                hunk.rows
                    .iter()
                    .filter(|row| on(row, side).is_some())
                    .count()
            };
            drift += count(to) as isize - count(from) as isize;
        }
        Some(line.saturating_add_signed(drift))
    }

    /// Row indices from `start` through `end` in patch order, either way round.
    pub fn rows_between(&self, start: LinePosition, end: LinePosition) -> Vec<usize> {
        let (Some(a), Some(b)) = (self.row_index(start), self.row_index(end)) else {
            return Vec::new();
        };
        (a.min(b)..=a.max(b)).collect()
    }

    /// Row indices of the hunk containing `position`.
    pub fn hunk_rows(&self, position: LinePosition) -> Vec<usize> {
        let Some(target) = self.row_index(position) else {
            return Vec::new();
        };
        let mut start = 0;
        for hunk in &self.hunks {
            let end = start + hunk.rows.len();
            if (start..end).contains(&target) {
                return (start..end).collect();
            }
            start = end;
        }
        Vec::new()
    }

    /// Whether the selection contains at least one change.
    pub fn selects_changes(&self, selected: &[usize]) -> bool {
        self.rows()
            .enumerate()
            .any(|(ix, row)| row.kind != RowKind::Context && selected.contains(&ix))
    }

    /// A patch containing only the `selected` changes. With `reverse`, it is
    /// meant for `git apply --reverse` against the new side.
    pub fn select(&self, selected: &[usize], reverse: bool) -> Result<String> {
        if self
            .header
            .iter()
            .any(|line| line.starts_with("new file mode") || line.starts_with("deleted file mode"))
        {
            bail!("a new or deleted file can only be staged as a whole");
        }
        if self
            .header
            .iter()
            .any(|line| line.starts_with("Binary files"))
        {
            bail!("a binary file can only be staged as a whole");
        }
        let mut out = String::new();
        for line in &self.header {
            out.push_str(line);
            out.push('\n');
        }
        let mut row_ix = 0;
        // How far the computed side has drifted from the base side.
        let mut drift: isize = 0;
        let mut emitted = false;
        for hunk in &self.hunks {
            let mut lines = Vec::new();
            let (mut old_count, mut new_count) = (0usize, 0usize);
            let mut changed = false;
            // Selected additions land where the first selected deletion of
            // their block was. Unselected deletions kept as context after that
            // point go after the additions: in `-b -c +B`, staging only
            // b -> B must give B, c rather than c, B.
            let mut deferred: Vec<String> = Vec::new();
            let mut deleting = false;
            for row in &hunk.rows {
                let is_selected = selected.contains(&row_ix);
                row_ix += 1;
                if row.kind == RowKind::Context {
                    lines.append(&mut deferred);
                    deleting = false;
                }
                let prefix = match (row.kind, is_selected, reverse) {
                    (RowKind::Context, _, _) => ' ',
                    (RowKind::Deletion, true, _) => '-',
                    (RowKind::Addition, true, _) => '+',
                    // Unselected changes still on the base side become context.
                    (RowKind::Deletion, false, false) | (RowKind::Addition, false, true) => ' ',
                    // Unselected changes absent from the base side disappear.
                    (RowKind::Addition, false, false) | (RowKind::Deletion, false, true) => {
                        continue;
                    }
                };
                changed |= prefix != ' ';
                old_count += (prefix != '+') as usize;
                new_count += (prefix != '-') as usize;
                deleting |= row.kind == RowKind::Deletion && prefix == '-';
                let target = if row.kind == RowKind::Deletion && prefix == ' ' && deleting {
                    &mut deferred
                } else {
                    &mut lines
                };
                target.push(format!("{prefix}{}", row.text));
                if row.no_newline {
                    target.push("\\ No newline at end of file".to_owned());
                }
            }
            lines.append(&mut deferred);
            if !changed {
                continue;
            }
            emitted = true;
            let (old_start, new_start) = if reverse {
                let new_start = hunk.new_start;
                let old_start = shifted(new_start, -drift, old_count);
                (old_start, new_start)
            } else {
                let old_start = hunk.old_start;
                (old_start, shifted(old_start, drift, new_count))
            };
            drift += new_count as isize - old_count as isize;
            out.push_str(&format!(
                "@@ -{old_start},{old_count} +{new_start},{new_count} @@{}\n",
                hunk.section
            ));
            for line in lines {
                out.push_str(&line);
                out.push('\n');
            }
        }
        if !emitted {
            bail!("the selection contains no changes");
        }
        Ok(out)
    }
}

fn parse_start(range: Option<&str>, line: &str) -> Result<usize> {
    let Some(range) = range else {
        bail!("malformed hunk header: {line}");
    };
    let start = range.split(',').next().unwrap_or(range);
    Ok(start.parse()?)
}

fn shifted(start: usize, drift: isize, count: usize) -> usize {
    let start = start.saturating_add_signed(drift);
    // An empty range names the line before it.
    if count == 0 {
        start.saturating_sub(1)
    } else {
        start.max(1)
    }
}
