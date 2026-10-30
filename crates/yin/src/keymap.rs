//! User key bindings from `~/.config/yin/keymap.json`, applied over the
//! defaults:
//!
//! ```json
//! [
//!   { "key": "cmd-k", "action": "yin::Commit" },
//!   { "key": "ctrl-n", "action": "yin::SelectNext", "context": "Changes" }
//! ]
//! ```

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
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let entries: Vec<Entry> = match serde_json::from_str(&text) {
        Ok(entries) => entries,
        Err(error) => return vec![format!("{}: {error}", path.display())],
    };
    let mut errors = Vec::new();
    let mut bindings = Vec::new();
    for entry in entries {
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
