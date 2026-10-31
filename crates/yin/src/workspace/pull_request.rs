//! The branch's pull request, through the GitHub CLI: its number in the git
//! bar, its review threads in the diff, and a dialog that opens a new one.
//! Without `gh` (or signed out) all of this quietly stays hidden.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    notification::Notification,
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement as _, ParentElement as _,
    Styled as _, Window, div, px,
};
use yin_git::Repository;

use super::Workspace;

/// How long a pull request lookup stays fresh; refreshing more often would
/// call GitHub on every file save.
const PR_FRESH_FOR: Duration = Duration::from_secs(120);

/// What was last asked of GitHub.
pub(super) struct PrCheck {
    branch: Option<String>,
    at: Instant,
}

/// The new pull request dialog's fields.
struct PrForm {
    title: Entity<InputState>,
    base: Entity<InputState>,
    body: Entity<TextareaState>,
    draft: Entity<bool>,
}

fn gh_path() -> Option<PathBuf> {
    yin_lsp::find_executable("gh")
}

impl Workspace {
    /// Looks up the branch's pull request when the branch changed or the
    /// last lookup is stale.
    pub(super) fn check_pull_request(&mut self, force: bool, cx: &mut Context<Self>) {
        // Tests never talk to GitHub.
        if cfg!(test) {
            return;
        }
        let branch = self.status.branch.head.clone();
        let fresh = self
            .pr_check
            .as_ref()
            .is_some_and(|check| check.branch == branch && check.at.elapsed() < PR_FRESH_FOR);
        if fresh && !force {
            return;
        }
        self.pr_check = Some(PrCheck {
            branch: branch.clone(),
            at: Instant::now(),
        });
        let Some(branch) = branch else {
            self.pull_request = None;
            self.review_threads.clear();
            return;
        };
        let repo = self.repo.clone();
        self._pr_task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let gh = gh_path().ok_or_else(|| anyhow::anyhow!("gh isn't installed"))?;
                    let pr = repo.pull_request(&gh)?;
                    let threads = match &pr {
                        Some(pr) if pr.is_open() => repo.review_threads(&gh, pr.number)?,
                        _ => Vec::new(),
                    };
                    anyhow::Ok((pr, threads))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.status.branch.head.as_ref() != Some(&branch) {
                    return;
                }
                match result {
                    Ok((pr, threads)) => {
                        this.pull_request = pr;
                        this.review_threads = threads;
                    }
                    // No gh, signed out or offline: show nothing rather than nag.
                    Err(error) => {
                        eprintln!("yin: pull request lookup: {error}");
                        this.pull_request = None;
                        this.review_threads.clear();
                    }
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Whether the History tab shows the pull request's diff, where its
    /// review threads line up.
    pub(super) fn shows_pull_request(&self) -> bool {
        let (Some(pr), Some(compare)) = (&self.pull_request, &self.history_compare) else {
            return false;
        };
        compare.head == "HEAD"
            && (compare.base == pr.base || compare.base == format!("origin/{}", pr.base))
    }

    /// Compares HEAD with the pull request's base, threads included.
    pub(super) fn review_pull_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pr) = self.pull_request.clone() else {
            window.push_notification("This branch has no pull request.", cx);
            return;
        };
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let base = cx
                .background_spawn(async move { base_ref(&repo, &pr.base) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.compare(base, "HEAD".into(), window, cx);
                this.check_pull_request(true, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_pull_request_in_browser(&self, cx: &mut Context<Self>) {
        if let Some(pr) = &self.pull_request {
            cx.open_url(&pr.url);
        }
    }

    /// Opens the new pull request dialog, prefilled from the branch's commits.
    pub(super) fn open_create_pull_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = self.status.branch.head.clone() else {
            window.push_notification("Switch to a branch to open a pull request.", cx);
            return;
        };
        if let Some(pr) = self.pull_request.as_ref().filter(|pr| pr.is_open()) {
            window.push_notification(format!("This branch already has #{}.", pr.number), cx);
            return;
        }
        if gh_path().is_none() {
            window.push_notification(
                Notification::error("Opening pull requests needs the GitHub CLI: brew install gh"),
                cx,
            );
            return;
        }
        let repo = self.repo.clone();
        // A stacked branch merges into the branch below it.
        let stacked_on = self
            .stack
            .as_ref()
            .and_then(|stack| stack.head())
            .map(|layer| layer.base.clone());
        cx.spawn_in(window, async move |this, cx| {
            let (base, commits) = cx
                .background_spawn(async move {
                    let base = stacked_on
                        .or_else(|| repo.trunk())
                        .map(|base| base.trim_start_matches("origin/").to_owned())
                        .unwrap_or_else(|| "main".into());
                    let from = base_ref(&repo, &base);
                    let commits = repo.log_range(&from, "HEAD", 50).unwrap_or_default();
                    (base, commits)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let (title, body) = prefill(&branch, &commits);
                this.show_create_pull_request(branch, base, title, body, window, cx)
            })
            .ok();
        })
        .detach();
    }

    fn show_create_pull_request(
        &mut self,
        branch: String,
        base: String,
        title: String,
        body: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = PrForm {
            title: cx.new(|cx| InputState::new(window, cx).default_value(title)),
            base: cx.new(|cx| InputState::new(window, cx).default_value(base)),
            body: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("Description")
                    .auto_grow(4, 14)
                    .default_value(body)
            }),
            draft: cx.new(|_| false),
        };
        let this = cx.entity().downgrade();
        let focus_on_open = std::rc::Rc::new(std::cell::Cell::new(true));
        window.open_alert_dialog(cx, move |alert, window, cx| {
            if focus_on_open.replace(false) {
                let title = form.title.clone();
                window.defer(cx, move |window, cx| {
                    title.update(cx, |input, cx| input.focus(window, cx));
                });
            }
            let draft = *form.draft.read(cx);
            let draft_state = form.draft.clone();
            let muted = cx.theme().muted_foreground;
            let field = |label: &'static str, input: AnyElement| {
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(muted).child(label))
                    .child(input)
            };
            let (this, form_title, form_base, form_body, form_draft) = (
                this.clone(),
                form.title.clone(),
                form.base.clone(),
                form.body.clone(),
                form.draft.clone(),
            );
            let branch = branch.clone();
            alert
                .w(px(560.))
                .title(format!("Open a pull request for {branch}"))
                .child(
                    v_flex()
                        .gap_3()
                        .child(field("Title", Input::new(&form.title).into_any_element()))
                        .child(field(
                            "Merge into",
                            Input::new(&form.base).into_any_element(),
                        ))
                        .child(field(
                            "Description",
                            Textarea::new(&form.body).small().into_any_element(),
                        ))
                        .child(
                            Checkbox::new("pr-draft")
                                .small()
                                .label("Draft")
                                .checked(draft)
                                .on_click(move |checked, _, cx| {
                                    draft_state.update(cx, |draft, _| *draft = *checked)
                                }),
                        )
                        .child(
                            div().text_xs().text_color(muted).child(
                                "yin pushes the branch first if GitHub doesn't have it yet.",
                            ),
                        ),
                )
                .confirm()
                .ok_text("Open pull request")
                .on_ok(move |_, window, cx| {
                    let title = form_title.read(cx).value().trim().to_owned();
                    let base = form_base.read(cx).value().trim().to_owned();
                    let body = form_body.read(cx).value().to_string();
                    let draft = *form_draft.read(cx);
                    if title.is_empty() || base.is_empty() {
                        return false;
                    }
                    let branch = branch.clone();
                    this.update(cx, |this, cx| {
                        this.create_pull_request(branch, base, title, body, draft, window, cx)
                    })
                    .ok();
                    true
                })
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn create_pull_request(
        &mut self,
        branch: String,
        base: String,
        title: String,
        body: String,
        draft: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let status_branch = self.status.branch.clone();
        let repo = self.repo.clone();
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let gh = gh_path().ok_or_else(|| anyhow::anyhow!("gh isn't installed"))?;
                    push_if_needed(&repo, &status_branch)?;
                    repo.create_pull_request(&gh, &branch, &base, &title, &body, draft)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(url) => {
                        window.push_notification(format!("Opened {url}"), cx);
                        cx.open_url(&url);
                        this.check_pull_request(true, cx);
                    }
                    Err(error) => {
                        window.push_notification(Notification::error(error.to_string()), cx)
                    }
                }
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The pull request's number in the git bar, when there is one.
    pub(super) fn render_pr_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pr = self.pull_request.as_ref()?;
        let open_threads = self
            .review_threads
            .iter()
            .filter(|t| !t.is_resolved)
            .count();
        let state = if pr.is_draft && pr.is_open() {
            "draft".to_owned()
        } else {
            pr.state.to_lowercase()
        };
        let mut tooltip = format!("#{} {} ({state})", pr.number, pr.title);
        if open_threads > 0 {
            tooltip.push_str(&format!(
                " · {}",
                super::history::count(open_threads, "open thread")
            ));
        }
        Some(
            Button::new("git-bar-pr")
                .ghost()
                .xsmall()
                .icon(Icon::empty().path("icons/git-pull-request.svg"))
                .label(if open_threads > 0 {
                    format!("#{} · {open_threads}", pr.number)
                } else {
                    format!("#{}", pr.number)
                })
                .text_color(if pr.is_open() {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .tooltip(tooltip)
                .on_click(cx.listener(|this, _, window, cx| this.review_pull_request(window, cx)))
                .into_any_element(),
        )
    }
}

/// `origin/<base>` when it exists, else the local branch.
fn base_ref(repo: &Repository, base: &str) -> String {
    let remote = format!("origin/{base}");
    if repo.resolves(&remote) {
        remote
    } else {
        base.to_owned()
    }
}

/// Pushes the branch when the remote lacks it or is behind.
fn push_if_needed(repo: &Repository, branch: &yin_git::Branch) -> anyhow::Result<()> {
    if branch.upstream.is_none() || branch.ahead > 0 {
        repo.push(branch)?;
    }
    Ok(())
}

/// A title and description from the commits going into the pull request:
/// one commit gives its own message; several are listed.
fn prefill(branch: &str, commits: &[yin_git::Commit]) -> (String, String) {
    match commits {
        [commit] => (commit.subject.clone(), String::new()),
        [] => (humanize(branch), String::new()),
        commits => (
            humanize(branch),
            commits
                .iter()
                .rev()
                .map(|commit| format!("- {}\n", commit.subject))
                .collect(),
        ),
    }
}

/// `feat/add-login` reads as "Add login".
fn humanize(branch: &str) -> String {
    let name = branch
        .rsplit('/')
        .next()
        .unwrap_or(branch)
        .replace(['-', '_'], " ");
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(subject: &str) -> yin_git::Commit {
        yin_git::Commit {
            oid: String::new(),
            short_oid: String::new(),
            author: String::new(),
            time: 0,
            subject: subject.into(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn prefills_from_commits() {
        assert_eq!(prefill("x", &[commit("Fix it")]).0, "Fix it");
        let (title, body) = prefill("feat/add-login", &[commit("b"), commit("a")]);
        assert_eq!(title, "Add login");
        assert_eq!(body, "- a\n- b\n");
    }
}
