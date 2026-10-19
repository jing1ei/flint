//! A command argument that accepts both `snake_case` and `camelCase` keys.
//!
//! Tauri remaps command argument names to camelCase on the JavaScript side by default, while this
//! app's IPC contract is written in snake_case (`{ preset_id }`, `{ target_id }`) to match the
//! payload structs it sends in the same call. Rather than betting the frontend picks the same
//! spelling we do, multi-word arguments are wrapped in [`Arg<T>`], which looks up *both*.
//!
//! It costs one `.0` at the call site and removes an entire class of "invalid args" bug reports.

use serde::de::DeserializeOwned;
use tauri::ipc::{CommandArg, CommandItem, InvokeBody, InvokeError};
use tauri::Runtime;

/// Wrapper around a command argument value. Deref-free on purpose: `arg.0` makes it obvious at the
/// call site that this is IPC input.
#[derive(Debug, Clone)]
pub struct Arg<T>(pub T);

// Note: `Arg<T>` deliberately does *not* implement `Deserialize`, otherwise this impl would
// overlap with Tauri's blanket `impl<D: Deserialize> CommandArg for D`.
impl<'de, T: DeserializeOwned, R: Runtime> CommandArg<'de, R> for Arg<T> {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        let key = command.key;
        let camel = to_lower_camel_case(key);
        let raw = match command.message.payload() {
            InvokeBody::Json(json) => json.get(key).or_else(|| json.get(camel.as_str())).cloned(),
            // Raw byte payloads are only used by the channel/blob APIs, which this app never calls.
            InvokeBody::Raw(_) => None,
        };
        let raw = raw.ok_or_else(|| {
            InvokeError::from(format!(
                "`{}` is missing the `{key}` argument (also accepted as `{camel}`)",
                command.name
            ))
        })?;
        serde_json::from_value(raw).map(Arg).map_err(|e| {
            InvokeError::from(format!("`{}` got an invalid `{key}`: {e}", command.name))
        })
    }
}

fn to_lower_camel_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper_next = false;
    for ch in key.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_lower_camel_case;

    #[test]
    fn matches_tauris_own_camel_casing() {
        assert_eq!(to_lower_camel_case("preset_id"), "presetId");
        assert_eq!(to_lower_camel_case("target_id"), "targetId");
        assert_eq!(to_lower_camel_case("path"), "path");
        assert_eq!(to_lower_camel_case("a_b_c"), "aBC");
    }
}
