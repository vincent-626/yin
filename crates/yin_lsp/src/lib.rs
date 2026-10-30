//! A minimal language server client: JSON-RPC over stdio, enough for
//! definitions, hover and references. No UI dependencies.

mod paths;

use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use lsp_types::notification::Notification;
use lsp_types::request::Request;
use serde_json::{Value, json};

pub use lsp_types;
pub use paths::{find_executable, path_to_uri, uri_to_path};

type Pending = Arc<Mutex<HashMap<i64, async_channel::Sender<Result<Value>>>>>;

/// A running language server for one root.
pub struct LanguageServer {
    name: String,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    next_id: AtomicI64,
    pending: Pending,
    versions: Mutex<HashMap<String, AtomicI32>>,
    capabilities: OnceLock<lsp_types::ServerCapabilities>,
}

fn write_frame(stdin: &mut impl std::io::Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()?;
    Ok(())
}

fn read_frame(reader: &mut impl std::io::BufRead) -> Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.ok_or_else(|| anyhow!("frame without Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

/// Answers requests the server sends us, so it never waits on them.
fn reply_to_server(request: &Value) -> Value {
    let result = match request["method"].as_str() {
        // One null per requested section: use the server's defaults.
        Some("workspace/configuration") => {
            let items = request["params"]["items"].as_array().map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        }
        _ => Value::Null,
    };
    json!({ "jsonrpc": "2.0", "id": request["id"], "result": result })
}

impl LanguageServer {
    /// Starts `command` for `root` and completes the initialize handshake.
    pub fn start(name: &str, command: &Path, args: &[&str], root: &Path) -> Result<Arc<Self>> {
        let mut child = Command::new(command)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("couldn't start {}", command.display()))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let pending: Pending = Arc::default();
        let (replies_tx, replies_rx) = std::sync::mpsc::channel::<Value>();

        std::thread::Builder::new()
            .name(format!("lsp-{name}"))
            .spawn({
                let pending = pending.clone();
                move || {
                    let mut reader = BufReader::new(stdout);
                    while let Ok(Some(message)) = read_frame(&mut reader) {
                        let is_response = message.get("method").is_none();
                        if is_response {
                            let Some(id) = message["id"].as_i64() else {
                                continue;
                            };
                            let Some(sender) = pending.lock().expect("pending").remove(&id) else {
                                continue;
                            };
                            let result = match message.get("error") {
                                Some(error) => Err(anyhow!(
                                    "{}",
                                    error["message"].as_str().unwrap_or("request failed")
                                )),
                                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            sender.try_send(result).ok();
                        } else if message.get("id").is_some() {
                            replies_tx.send(reply_to_server(&message)).ok();
                        }
                        // Notifications (diagnostics, progress, logs) are not used yet.
                    }
                    // The server exited: fail everything still waiting.
                    for (_, sender) in pending.lock().expect("pending").drain() {
                        sender.try_send(Err(anyhow!("language server exited"))).ok();
                    }
                }
            })?;

        let server = Arc::new(Self {
            name: name.to_owned(),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            next_id: AtomicI64::new(1),
            pending,
            versions: Mutex::default(),
            capabilities: OnceLock::new(),
        });
        // Replies to server requests go through the same writer.
        std::thread::spawn({
            let server = Arc::downgrade(&server);
            move || {
                while let Ok(reply) = replies_rx.recv() {
                    let Some(server) = server.upgrade() else {
                        break;
                    };
                    server.send(&reply).ok();
                }
            }
        });

        let root_uri = path_to_uri(root)?;
        #[allow(deprecated)]
        let params = lsp_types::InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            workspace_folders: Some(vec![lsp_types::WorkspaceFolder {
                uri: root_uri,
                name: root
                    .file_name()
                    .map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned()),
            }]),
            capabilities: client_capabilities(),
            ..Default::default()
        };
        let result = server
            .request_blocking::<lsp_types::request::Initialize>(params, Duration::from_secs(30))?;
        server.notify::<lsp_types::notification::Initialized>(lsp_types::InitializedParams {})?;
        server.capabilities.set(result.capabilities).ok();
        Ok(server)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn capabilities(&self) -> Option<&lsp_types::ServerCapabilities> {
        self.capabilities.get()
    }

    fn send(&self, message: &Value) -> Result<()> {
        write_frame(&mut *self.stdin.lock().expect("stdin"), message)
    }

    fn start_request<R: Request>(
        &self,
        params: R::Params,
    ) -> Result<async_channel::Receiver<Result<Value>>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = async_channel::bounded(1);
        self.pending.lock().expect("pending").insert(id, sender);
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": R::METHOD,
            "params": params,
        }))?;
        Ok(receiver)
    }

    /// Sends a request; the future resolves with the server's result.
    pub fn request<R: Request>(
        &self,
        params: R::Params,
    ) -> impl Future<Output = Result<R::Result>> + use<R> {
        let receiver = self.start_request::<R>(params);
        async move {
            let value = receiver?
                .recv()
                .await
                .map_err(|_| anyhow!("language server exited"))??;
            Ok(serde_json::from_value(value)?)
        }
    }

    fn request_blocking<R: Request>(
        &self,
        params: R::Params,
        timeout: Duration,
    ) -> Result<R::Result> {
        let receiver = self.start_request::<R>(params)?;
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            done_tx.send(receiver.recv_blocking()).ok();
        });
        let value = done_rx
            .recv_timeout(timeout)
            .map_err(|_| anyhow!("{} timed out", R::METHOD))?
            .map_err(|_| anyhow!("language server exited"))??;
        Ok(serde_json::from_value(value)?)
    }

    pub fn notify<N: Notification>(&self, params: N::Params) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": N::METHOD, "params": params }))
    }

    /// Tells the server about a document, or its new full text.
    pub fn sync_document(&self, path: &Path, language_id: &str, text: &str) -> Result<()> {
        let uri = path_to_uri(path)?;
        let key = uri.as_str().to_owned();
        let mut versions = self.versions.lock().expect("versions");
        match versions.get(&key) {
            Some(version) => {
                let version = version.fetch_add(1, Ordering::Relaxed) + 1;
                self.notify::<lsp_types::notification::DidChangeTextDocument>(
                    lsp_types::DidChangeTextDocumentParams {
                        text_document: lsp_types::VersionedTextDocumentIdentifier { uri, version },
                        content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                            range: None,
                            range_length: None,
                            text: text.to_owned(),
                        }],
                    },
                )
            }
            None => {
                versions.insert(key, AtomicI32::new(1));
                self.notify::<lsp_types::notification::DidOpenTextDocument>(
                    lsp_types::DidOpenTextDocumentParams {
                        text_document: lsp_types::TextDocumentItem {
                            uri,
                            language_id: language_id.to_owned(),
                            version: 1,
                            text: text.to_owned(),
                        },
                    },
                )
            }
        }
    }

    pub fn close_document(&self, path: &Path) -> Result<()> {
        let uri = path_to_uri(path)?;
        if self
            .versions
            .lock()
            .expect("versions")
            .remove(uri.as_str())
            .is_none()
        {
            return Ok(());
        }
        self.notify::<lsp_types::notification::DidCloseTextDocument>(
            lsp_types::DidCloseTextDocumentParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
            },
        )
    }
}

impl Drop for LanguageServer {
    fn drop(&mut self) {
        // Best effort: ask nicely, then make sure it is gone.
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": "shutdown" }))
            .ok();
        self.notify::<lsp_types::notification::Exit>(()).ok();
        if let Ok(mut child) = self.child.lock() {
            std::thread::sleep(Duration::from_millis(50));
            child.kill().ok();
            child.wait().ok();
        }
    }
}

fn client_capabilities() -> lsp_types::ClientCapabilities {
    use lsp_types::*;
    ClientCapabilities {
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities::default()),
            hover: Some(HoverClientCapabilities {
                content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                ..Default::default()
            }),
            definition: Some(GotoCapability {
                link_support: Some(true),
                ..Default::default()
            }),
            references: Some(DynamicRegistrationClientCapabilities::default()),
            ..Default::default()
        }),
        workspace: Some(WorkspaceClientCapabilities {
            workspace_folders: Some(true),
            configuration: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Normalizes `textDocument/definition` results to links.
pub fn definition_links(
    response: Option<lsp_types::GotoDefinitionResponse>,
) -> Vec<lsp_types::LocationLink> {
    use lsp_types::GotoDefinitionResponse::*;
    let location_link = |location: lsp_types::Location| lsp_types::LocationLink {
        origin_selection_range: None,
        target_uri: location.uri,
        target_range: location.range,
        target_selection_range: location.range,
    };
    match response {
        None => Vec::new(),
        Some(Scalar(location)) => vec![location_link(location)],
        Some(Array(locations)) => locations.into_iter().map(location_link).collect(),
        Some(Link(links)) => links,
    }
}

/// Which server to run for a language, if any.
pub fn server_for(language: &str) -> Option<(&'static str, &'static [&'static str])> {
    Some(match language {
        "rust" => ("rust-analyzer", &[]),
        "go" => ("gopls", &[]),
        "typescript" | "tsx" | "javascript" => ("typescript-language-server", &["--stdio"]),
        "python" => ("pyright-langserver", &["--stdio"]),
        "c" | "cpp" => ("clangd", &[]),
        _ => return None,
    })
}

/// Fails if `path` is outside `root`; servers only answer for their workspace.
pub fn ensure_within(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    if !path.starts_with(root) {
        bail!("{} is outside {}", path.display(), root.display());
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
