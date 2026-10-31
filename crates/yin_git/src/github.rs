//! Pull requests through the GitHub CLI (`gh`), which owns sign-in. yin
//! reads pull requests and their review threads, and opens new ones when
//! asked; it never posts comments.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow};
use serde::Deserialize;

use crate::{Repository, Side};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `OPEN`, `CLOSED` or `MERGED`.
    pub state: String,
    pub is_draft: bool,
    #[serde(rename = "baseRefName")]
    pub base: String,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == "OPEN"
    }
}

/// A review conversation on a line of the pull request's diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewThread {
    pub path: String,
    /// The line on `side` in the pull request's current diff; `None` once
    /// the code it was on changed.
    pub line: Option<usize>,
    /// The line when the comment was written.
    pub original_line: Option<usize>,
    /// `Old` for a comment on a deleted line.
    pub side: Side,
    pub is_resolved: bool,
    pub comments: Vec<ReviewComment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewComment {
    pub author: String,
    pub body: String,
    pub url: String,
}

const PR_FIELDS: &str = "number,title,url,state,isDraft,baseRefName";

const THREADS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 100) {
        nodes {
          isResolved path line originalLine diffSide
          comments(first: 50) { nodes { author { login } body url } }
        }
      }
    }
  }
}";

impl Repository {
    /// Runs `gh` in the working tree and returns stdout, or fails with its
    /// stderr. Never prompts.
    fn gh<I, S>(&self, gh: &Path, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let output = Command::new(gh)
            .args(args)
            .current_dir(self.workdir())
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .context("couldn't run gh")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("{}", stderr.trim()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The current branch's pull request, if it has one.
    pub fn pull_request(&self, gh: &Path) -> Result<Option<PullRequest>> {
        match self.gh(gh, ["pr", "view", "--json", PR_FIELDS]) {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(error) if is_no_pull_request(&error.to_string()) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Opens a pull request from `head` (pushed already) into `base` and
    /// returns its URL.
    pub fn create_pull_request(
        &self,
        gh: &Path,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<String> {
        let mut args = vec![
            "pr", "create", "--head", head, "--base", base, "--title", title, "--body", body,
        ];
        if draft {
            args.push("--draft");
        }
        let output = self.gh(gh, args)?;
        output
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with("https://"))
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("gh didn't print the new pull request's address"))
    }

    /// Review threads on pull request `number`.
    pub fn review_threads(&self, gh: &Path, number: u64) -> Result<Vec<ReviewThread>> {
        let json = self.gh(
            gh,
            [
                "api".to_owned(),
                "graphql".to_owned(),
                "-f".to_owned(),
                format!("query={THREADS_QUERY}"),
                "-F".to_owned(),
                "owner={owner}".to_owned(),
                "-F".to_owned(),
                "name={repo}".to_owned(),
                "-F".to_owned(),
                format!("number={number}"),
            ],
        )?;
        parse_threads(&json)
    }
}

fn is_no_pull_request(error: &str) -> bool {
    error.contains("no pull requests found")
}

fn parse_threads(json: &str) -> Result<Vec<ReviewThread>> {
    #[derive(Deserialize)]
    struct Response {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: RepositoryData,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RepositoryData {
        pull_request: PullRequestData,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PullRequestData {
        review_threads: Nodes<Thread>,
    }
    #[derive(Deserialize)]
    struct Nodes<T> {
        nodes: Vec<T>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Thread {
        is_resolved: bool,
        path: String,
        line: Option<usize>,
        original_line: Option<usize>,
        diff_side: String,
        comments: Nodes<Comment>,
    }
    #[derive(Deserialize)]
    struct Comment {
        author: Option<Author>,
        body: String,
        url: String,
    }
    #[derive(Deserialize)]
    struct Author {
        login: String,
    }
    let response: Response = serde_json::from_str(json)?;
    Ok(response
        .data
        .repository
        .pull_request
        .review_threads
        .nodes
        .into_iter()
        .map(|thread| ReviewThread {
            path: thread.path,
            line: thread.line,
            original_line: thread.original_line,
            side: if thread.diff_side == "LEFT" {
                Side::Old
            } else {
                Side::New
            },
            is_resolved: thread.is_resolved,
            comments: thread
                .comments
                .nodes
                .into_iter()
                .map(|comment| ReviewComment {
                    // Deleted accounts have no author.
                    author: comment.author.map_or_else(|| "ghost".into(), |a| a.login),
                    body: comment.body,
                    url: comment.url,
                })
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_review_threads() {
        let json = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
            {"isResolved":false,"path":"src/a.rs","line":12,"originalLine":10,"diffSide":"RIGHT",
             "comments":{"nodes":[{"author":{"login":"ana"},"body":"Why?","url":"https://x/1"},
                                  {"author":null,"body":"Because.","url":"https://x/2"}]}},
            {"isResolved":true,"path":"b.rs","line":null,"originalLine":3,"diffSide":"LEFT",
             "comments":{"nodes":[]}}]}}}}}"#;
        let threads = parse_threads(json).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].path, "src/a.rs");
        assert_eq!(threads[0].line, Some(12));
        assert_eq!(threads[0].side, Side::New);
        assert_eq!(threads[0].comments[1].author, "ghost");
        assert!(threads[1].is_resolved);
        assert_eq!(threads[1].side, Side::Old);
        assert_eq!(threads[1].line, None);
    }

    #[test]
    fn reads_a_pull_request() {
        let json = r#"{"number":7,"title":"Add x","url":"https://github.com/o/r/pull/7",
            "state":"OPEN","isDraft":false,"baseRefName":"main"}"#;
        let pr: PullRequest = serde_json::from_str(json).unwrap();
        assert_eq!(pr.number, 7);
        assert_eq!(pr.base, "main");
        assert!(pr.is_open());
        assert!(is_no_pull_request(
            "no pull requests found for branch \"feature\""
        ));
    }
}
