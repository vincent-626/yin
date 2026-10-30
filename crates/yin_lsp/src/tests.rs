use std::fs;
use std::path::Path;

use super::*;

#[test]
fn uris_round_trip_with_escapes() {
    let path = Path::new("/tmp/a dir/ünï#code.rs");
    let uri = path_to_uri(path).unwrap();
    assert!(
        uri.as_str().starts_with("file:///tmp/a%20dir/"),
        "{}",
        uri.as_str()
    );
    assert_eq!(uri_to_path(&uri).unwrap(), path);
}

#[test]
fn frames_round_trip() {
    let mut buffer = Vec::new();
    let message = json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": "ü"}});
    write_frame(&mut buffer, &message).unwrap();
    let read = read_frame(&mut std::io::Cursor::new(buffer))
        .unwrap()
        .unwrap();
    assert_eq!(read, message);
}

#[test]
fn answers_configuration_requests_with_defaults() {
    let request = json!({
        "jsonrpc": "2.0", "id": 7, "method": "workspace/configuration",
        "params": {"items": [{"section": "a"}, {"section": "b"}]}
    });
    let reply = reply_to_server(&request);
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["result"], json!([null, null]));
}

/// Needs rust-analyzer on PATH:
/// `cargo test -p yin_lsp -- --ignored --nocapture`
#[test]
#[ignore]
fn rust_analyzer_finds_a_definition() {
    let Some(command) = find_executable("rust-analyzer") else {
        eprintln!("rust-analyzer not found; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let main = root.join("src/main.rs");
    let text = "fn helper() -> u32 { 1 }\n\nfn main() {\n    helper();\n}\n";
    fs::write(&main, text).unwrap();

    let server = LanguageServer::start("rust-analyzer", &command, &[], &root).unwrap();
    server.sync_document(&main, "rust", text).unwrap();
    let params = lsp_types::GotoDefinitionParams {
        text_document_position_params: lsp_types::TextDocumentPositionParams {
            text_document: lsp_types::TextDocumentIdentifier {
                uri: path_to_uri(&main).unwrap(),
            },
            position: lsp_types::Position {
                line: 3,
                character: 6,
            },
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    // rust-analyzer answers empty until it has indexed; retry for a while.
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    loop {
        // While indexing it may also answer "content modified"; retry that too.
        let links = server
            .request_blocking::<lsp_types::request::GotoDefinition>(
                params.clone(),
                Duration::from_secs(30),
            )
            .map(definition_links)
            .unwrap_or_default();
        if let Some(link) = links.first() {
            assert_eq!(uri_to_path(&link.target_uri).unwrap(), main);
            assert_eq!(link.target_selection_range.start.line, 0);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no definition before the deadline"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `cargo test -p yin_lsp rust_analyzer_publishes -- --ignored --nocapture`
#[test]
#[ignore]
fn rust_analyzer_publishes_diagnostics() {
    let Some(command) = find_executable("rust-analyzer") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let main = root.join("src/main.rs");
    let text = "fn main() {\n    let count: u32 = \"three\";\n}\n";
    fs::write(&main, text).unwrap();
    let server = LanguageServer::start("rust-analyzer", &command, &[], &root).unwrap();
    server.sync_document(&main, "rust", text).unwrap();
    let published = server.diagnostics();
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!remaining.is_zero(), "no diagnostics before the deadline");
        let (tx, rx) = std::sync::mpsc::channel();
        let receiver = published.clone();
        std::thread::spawn(move || tx.send(receiver.recv_blocking()).ok());
        let Ok(Ok(params)) = rx.recv_timeout(remaining) else {
            continue;
        };
        eprintln!("{} -> {}", params.uri.as_str(), params.diagnostics.len());
        if !params.diagnostics.is_empty() {
            break;
        }
    }
}
