//! User key bindings from `~/.config/yin/keymap.json`, applied over the
//! defaults:
//!
//! ```json
//! [
//!   { "key": "cmd-k", "action": "yin::Commit" },
//!   { "key": "ctrl-n", "action": "yin::SelectNext", "context": "Changes" },
//!   { "key": "cmd-shift-space", "action": "yin::SwitchRepository", "context": "Global" }
//! ]
//! ```
//!
//! The `Global` context is the system-wide hotkey for the repository switcher,
//! which works from any app; `"key": "none"` turns it off.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{App, KeyBinding, KeyBindingContextPredicate};

#[derive(serde::Deserialize)]
struct Entry {
    key: String,
    action: String,
    context: Option<String>,
}

pub fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/yin/keymap.json"))
}

/// Loads the user keymap, if there is one. Returns one message per problem;
/// valid entries still apply.
pub fn load(cx: &mut App) -> Vec<String> {
    let entries = match entries() {
        Ok(entries) => entries,
        Err(error) => return vec![error],
    };
    let mut errors = Vec::new();
    let mut bindings = Vec::new();
    for entry in entries.into_iter().filter(|entry| !is_global(entry)) {
        let action = match cx.build_action(&entry.action, None) {
            Ok(action) => action,
            Err(error) => {
                errors.push(format!("{}: {error}", entry.action));
                continue;
            }
        };
        let context = match entry
            .context
            .as_deref()
            .map(KeyBindingContextPredicate::parse)
        {
            None => None,
            Some(Ok(predicate)) => Some(Rc::new(predicate)),
            Some(Err(error)) => {
                errors.push(format!("context {:?}: {error}", entry.context));
                continue;
            }
        };
        match KeyBinding::load(&entry.key, action, context, false, None, cx) {
            Ok(binding) => bindings.push(binding),
            Err(error) => errors.push(format!("key {:?}: {error}", entry.key)),
        }
    }
    cx.bind_keys(bindings);
    errors
}

fn entries() -> Result<Vec<Entry>, String> {
    let Some(path) = path() else {
        return Ok(Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

const GLOBAL: &str = "Global";

fn is_global(entry: &Entry) -> bool {
    entry.context.as_deref() == Some(GLOBAL)
}

/// The system-wide hotkey for the repository switcher: the keymap's `Global`
/// binding, else the default. `None` when it is turned off.
pub fn global_hotkey() -> Result<Option<String>, String> {
    let entries = entries().unwrap_or_default();
    let Some(entry) = entries.iter().rev().find(|entry| is_global(entry)) else {
        return Ok(Some(crate::hotkey::DEFAULT.to_owned()));
    };
    if entry.action != "yin::SwitchRepository" {
        return Err(format!(
            "only yin::SwitchRepository can be global, not {}",
            entry.action
        ));
    }
    Ok((entry.key != "none").then(|| entry.key.clone()))
}
