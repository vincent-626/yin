//! Language servers: one per language, started the first time a file in that
//! language opens, used for go to definition, hover and find references.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::component::{
    WindowExt as _,
    input::{DefinitionProvider, EditorState, HoverProvider, Rope, RopeExt as _},
    notification::Notification,
};
use gpui_kit::{App, AppContext as _, Context, Entity, Task, WeakEntity, Window};
use yin_lsp::{LanguageServer, lsp_types};

use super::Workspace;

/// A server's lifecycle. A failed start is remembered so it isn't retried
/// for every file.
pub(super) enum ServerSlot {
    Starting,
    Running(Arc<LanguageServer>),
    Failed,
}

#[derive(Default)]
pub(super) struct Servers {
    by_name: HashMap<&'static str, ServerSlot>,
}

/// Answers the editor's definition and hover requests for one file.
struct FileProvider {
    server: Arc<LanguageServer>,
    path: PathBuf,
}

impl FileProvider {
    fn position_params(
        &self,
        text: &Rope,
        offset: usize,
    ) -> Result<lsp_types::TextDocumentPositionParams> {
        Ok(lsp_types::TextDocumentPositionParams {
            text_document: lsp_types::TextDocumentIdentifier {
                uri: yin_lsp::path_to_uri(&self.path)?,
            },
            position: text.offset_to_position(offset),
        })
    }
}

impl DefinitionProvider for FileProvider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<lsp_types::LocationLink>>> {
        let params = match self.position_params(text, offset) {
            Ok(params) => params,
            Err(error) => return Task::ready(Err(error)),
        };
        let request = self.server.request::<lsp_types::request::GotoDefinition>(
            lsp_types::GotoDefinitionParams {
                text_document_position_params: params,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        );
        // A server still indexing answers with errors; treat them as "none yet".
        cx.background_spawn(async move {
            Ok(request
                .await
                .map(yin_lsp::definition_links)
                .unwrap_or_default())
        })
    }
}

impl HoverProvider for FileProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<lsp_types::Hover>>> {
        let params = match self.position_params(text, offset) {
            Ok(params) => params,
            Err(error) => return Task::ready(Err(error)),
        };
        let request =
            self.server
                .request::<lsp_types::request::HoverRequest>(lsp_types::HoverParams {
                    text_document_position_params: params,
                    work_done_progress_params: Default::default(),
                });
        cx.background_spawn(async move { Ok(request.await.unwrap_or_default()) })
    }
}

impl Workspace {
    fn full_path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.repo.workdir().join(path)
        }
    }

    /// Connects an open file's editor to its language server, starting the
    /// server if this is the first file in its language.
    pub(super) fn attach_language_server(
        &mut self,
        path: &str,
        language: &'static str,
        editor: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((name, args)) = yin_lsp::server_for(language) else {
            return;
        };
        match self.servers.by_name.get(name) {
            Some(ServerSlot::Running(server)) => {
                let server = server.clone();
                self.connect_editor(&server, path, language, editor, window, cx);
            }
            Some(ServerSlot::Starting | ServerSlot::Failed) => {}
            None => {
                self.servers.by_name.insert(name, ServerSlot::Starting);
                let root = self
                    .repo
                    .workdir()
                    .canonicalize()
                    .unwrap_or_else(|_| self.repo.workdir().to_owned());
                cx.spawn_in(window, async move |this, cx| {
                    let started = cx
                        .background_spawn(async move {
                            let command = yin_lsp::find_executable(name).ok_or_else(|| {
                                anyhow::anyhow!("{name} isn't installed, so definitions and hover are off for {language}")
                            })?;
                            LanguageServer::start(name, &command, args, &root)
                        })
                        .await;
                    this.update_in(cx, |this, window, cx| match started {
                        Ok(server) => {
                            this.servers
                                .by_name
                                .insert(name, ServerSlot::Running(server.clone()));
                            this.connect_open_files(&server, language, window, cx);
                        }
                        Err(error) => {
                            this.servers.by_name.insert(name, ServerSlot::Failed);
                            window.push_notification(Notification::warning(error.to_string()), cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    /// Connects every open file the server's languages cover.
    fn connect_open_files(
        &mut self,
        server: &Arc<LanguageServer>,
        language: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let server_name = server.name().to_owned();
        let editors: Vec<(String, &'static str, Entity<EditorState>)> = self
            .open_files
            .iter()
            .filter_map(|file| {
                let (path, editor) = file.editor()?;
                let file_language = super::files::language_for(path);
                let same_server =
                    yin_lsp::server_for(file_language).is_some_and(|(name, _)| name == server_name);
                (same_server || file_language == language)
                    .then(|| (path.to_owned(), file_language, editor.clone()))
            })
            .collect();
        for (path, language, editor) in editors {
            self.connect_editor(server, &path, language, &editor, window, cx);
        }
    }

    fn connect_editor(
        &mut self,
        server: &Arc<LanguageServer>,
        path: &str,
        language: &str,
        editor: &Entity<EditorState>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let full = self.full_path(path);
        let text = editor.read(cx).value().to_string();
        server.sync_document(&full, language, &text).ok();
        let provider = Rc::new(FileProvider {
            server: server.clone(),
            path: full.clone(),
        });
        let workspace = cx.entity().downgrade();
        editor.update(cx, |editor, _| {
            let lsp = editor.lsp_mut();
            lsp.definition_provider = Some(provider.clone());
            lsp.hover_provider = Some(provider);
            lsp.show_document = Some(show_document_handler(workspace, full));
        });
    }

    /// Sends an edited file's text to its server.
    pub(super) fn sync_edited_file(&self, path: &str, text: &str) {
        let language = super::files::language_for(path);
        if let Some((name, _)) = yin_lsp::server_for(language)
            && let Some(ServerSlot::Running(server)) = self.servers.by_name.get(name)
        {
            server
                .sync_document(&self.full_path(path), language, text)
                .ok();
        }
    }

    pub(super) fn close_in_language_server(&self, path: &str) {
        let language = super::files::language_for(path);
        if let Some((name, _)) = yin_lsp::server_for(language)
            && let Some(ServerSlot::Running(server)) = self.servers.by_name.get(name)
        {
            server.close_document(&self.full_path(path)).ok();
        }
    }

    /// Finds references to the symbol at the active editor's cursor and lists
    /// them in the Search tab.
    pub(super) fn find_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.active_file.and_then(|ix| self.open_files.get(ix)) else {
            return;
        };
        let Some((path, editor)) = file.editor() else {
            return;
        };
        let language = super::files::language_for(path);
        let server = yin_lsp::server_for(language).and_then(|(name, _)| {
            match self.servers.by_name.get(name) {
                Some(ServerSlot::Running(server)) => Some(server.clone()),
                _ => None,
            }
        });
        let Some(server) = server else {
            window.push_notification("No language server for this file.", cx);
            return;
        };
        let editor = editor.read(cx);
        let position = editor.cursor_position();
        let symbol = editor
            .text()
            .word_range(editor.cursor())
            .map(|range| editor.text().slice(range).to_string())
            .unwrap_or_default();
        let full = self.full_path(path);
        let Ok(uri) = yin_lsp::path_to_uri(&full) else {
            return;
        };
        let request =
            server.request::<lsp_types::request::References>(lsp_types::ReferenceParams {
                text_document_position: lsp_types::TextDocumentPositionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri },
                    position,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: lsp_types::ReferenceContext {
                    include_declaration: true,
                },
            });
        let root = self
            .repo
            .workdir()
            .canonicalize()
            .unwrap_or_else(|_| self.repo.workdir().to_owned());
        cx.spawn_in(window, async move |this, cx| {
            let locations = request.await.ok().flatten().unwrap_or_default();
            let matches = cx
                .background_spawn(async move { reference_matches(&root, locations) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.show_references(symbol, matches, window, cx)
            })
            .ok();
        })
        .detach();
    }
}

/// Reference locations as search results: path relative to `root` when
/// inside it, the line's text, and the referenced range.
fn reference_matches(root: &Path, locations: Vec<lsp_types::Location>) -> Vec<yin_fs::SearchMatch> {
    let mut files: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut matches: Vec<yin_fs::SearchMatch> = locations
        .into_iter()
        .filter_map(|location| {
            let full = yin_lsp::uri_to_path(&location.uri)?;
            let lines = files.entry(full.clone()).or_insert_with(|| {
                std::fs::read_to_string(&full)
                    .map(|text| text.lines().map(str::to_owned).collect())
                    .unwrap_or_default()
            });
            let line_ix = location.range.start.line as usize;
            let text = lines.get(line_ix).cloned().unwrap_or_default();
            // LSP columns count UTF-16 units; close enough for highlighting ASCII code.
            let start = (location.range.start.character as usize).min(text.len());
            let end = if location.range.end.line == location.range.start.line {
                (location.range.end.character as usize).clamp(start, text.len())
            } else {
                text.len()
            };
            let ranges = if text.is_char_boundary(start) && text.is_char_boundary(end) {
                vec![start..end]
            } else {
                Vec::new()
            };
            let path = full
                .strip_prefix(root)
                .map(|relative| relative.to_string_lossy().into_owned())
                .unwrap_or_else(|_| full.to_string_lossy().into_owned());
            Some(yin_fs::SearchMatch {
                path,
                line: line_ix as u64 + 1,
                text,
                ranges,
            })
        })
        .collect();
    matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    matches.dedup();
    matches
}

/// Opens definitions in other files as tabs; same-file jumps stay with the
/// editor's own handling.
fn show_document_handler(
    workspace: WeakEntity<Workspace>,
    current: PathBuf,
) -> gpui_kit::component::input::ShowDocumentHandler {
    Rc::new(move |params, window, cx| {
        let Some(target) = yin_lsp::uri_to_path(&params.uri) else {
            return false;
        };
        if target == current {
            return false;
        }
        let line = params.selection.map_or(1, |range| range.start.line + 1);
        let workspace = workspace.clone();
        // The editor is mid-update; open the tab once it is done.
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |this, cx| {
                    let root = this
                        .repo
                        .workdir()
                        .canonicalize()
                        .unwrap_or_else(|_| this.repo.workdir().to_owned());
                    let path = target
                        .strip_prefix(&root)
                        .map(|relative| relative.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| target.to_string_lossy().into_owned());
                    this.open_file(&path, Some(line), window, cx);
                })
                .ok();
        });
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_become_search_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/a.rs"),
            "fn helper() {}\nfn main() { helper(); }\n",
        )
        .unwrap();
        let uri = yin_lsp::path_to_uri(&root.join("src/a.rs")).unwrap();
        let location = |line, start, end| lsp_types::Location {
            uri: uri.clone(),
            range: lsp_types::Range {
                start: lsp_types::Position {
                    line,
                    character: start,
                },
                end: lsp_types::Position {
                    line,
                    character: end,
                },
            },
        };
        let matches = reference_matches(
            &root,
            vec![location(1, 12, 18), location(0, 3, 9), location(0, 3, 9)],
        );
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, "src/a.rs");
        assert_eq!(matches[0].line, 1);
        assert_eq!(&matches[0].text[matches[0].ranges[0].clone()], "helper");
        assert_eq!(&matches[1].text[matches[1].ranges[0].clone()], "helper");
    }
}
