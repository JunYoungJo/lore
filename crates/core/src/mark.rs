//! Marks: what a session says, while it works, is worth remembering.
//!
//! A hook tells the session to run `lore-cli mark`. The command writes nothing: it prints one
//! line, and that line lands in the session file as the result of a tool call. The app reads the
//! marks from there when the session is prepared, so marking works inside a sandbox and needs no
//! path into the app's data.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::candidate::Kind;

pub const PREFIX: &str = "LORE-MARK ";
/// Output wrapped as JSON inside JSON is unwrapped this many times at most.
const DEPTH: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub kind: Kind,
    pub text: String,
}

/// A mark and the line of the session file it was read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Placed {
    pub line: usize,
    #[serde(flatten)]
    pub mark: Mark,
}

/// The line `lore-cli mark` prints.
pub fn line(mark: &Mark) -> String {
    format!("{PREFIX}{}", serde_json::to_string(mark).unwrap_or_default())
}

fn collect(value: &Value, depth: usize, found: &mut Vec<Mark>) {
    match value {
        Value::String(text) => {
            let before = found.len();
            for line in text.lines() {
                let Some(rest) = line.trim().strip_prefix(PREFIX) else { continue };
                if let Ok(mark) = serde_json::from_str::<Mark>(rest.trim()) {
                    if !mark.text.trim().is_empty() && !found.contains(&mark) {
                        found.push(mark);
                    }
                }
            }
            // Codex stores a command's output as a JSON text inside the record.
            if found.len() == before && depth < DEPTH && text.trim_start().starts_with(['{', '[']) {
                if let Ok(inner) = serde_json::from_str::<Value>(text) {
                    collect(&inner, depth + 1, found);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect(item, depth, found)),
        Value::Object(map) => map.values().for_each(|item| collect(item, depth, found)),
        _ => {}
    }
}

/// The marks in the output of a tool call, in the order they were printed.
pub fn marks_in(output: &Value) -> Vec<Mark> {
    let mut found = Vec::new();
    collect(output, 0, &mut found);
    found
}

/// What the hook adds to each prompt. `cli` is the command that runs `lore-cli`, quoted for a shell.
pub fn instruction(cli: &str) -> String {
    format!(
        "Lore keeps what this user decides for later sessions. When, in this session, the user decides something, corrects how you work, or the work establishes a lesson a later session would need, run this once for it and carry on: \
         {cli} mark <decision|correction|lesson> \"<one sentence that can be understood without this conversation>\" \
         Do not mark routine progress. The command only prints a line; the user reviews every mark before it is kept."
    )
}

/// The path of an executable as a shell command: quoted when it needs to be, with forward slashes.
pub fn shell_command(path: &str) -> String {
    let path = path.replace('\\', "/");
    if path.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:".contains(c)) {
        path
    } else {
        format!("\"{}\"", path.replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mark(kind: Kind, text: &str) -> Mark {
        Mark { kind, text: text.into() }
    }

    #[test]
    fn a_printed_mark_is_read_back_from_plain_and_from_wrapped_output() {
        let first = mark(Kind::Decision, "No desktop alerts for finished jobs; \"quiet\" was the user's word.");
        let second = mark(Kind::Lesson, "한 번에 모두 읽으면 큰 계정에서 시간 초과가 난다.");
        let printed = format!("{}\nother output\n  {}\n", line(&first), line(&second));
        assert_eq!(marks_in(&json!(printed)), vec![first.clone(), second.clone()]);
        // Claude Code: a list of text blocks. Codex: the output as a JSON text.
        assert_eq!(marks_in(&json!([{"type": "text", "text": printed}])), vec![first.clone(), second.clone()]);
        let wrapped = json!({"output": printed, "metadata": {"exit_code": 0}}).to_string();
        assert_eq!(marks_in(&json!(wrapped)), vec![first, second]);
    }

    #[test]
    fn text_that_only_mentions_the_prefix_is_not_a_mark() {
        assert!(marks_in(&json!("Run LORE-MARK later")).is_empty());
        assert!(marks_in(&json!("LORE-MARK not json")).is_empty());
        assert!(marks_in(&json!("LORE-MARK {\"kind\":\"rumour\",\"text\":\"x\"}")).is_empty());
        assert!(marks_in(&json!("LORE-MARK {\"kind\":\"lesson\",\"text\":\"  \"}")).is_empty());
        let once = line(&mark(Kind::Correction, "Ask before deleting."));
        assert_eq!(marks_in(&json!(format!("{once}\n{once}"))).len(), 1);
    }

    #[test]
    fn the_instruction_names_the_command_and_the_three_kinds() {
        let text = instruction(&shell_command("C:\\Users\\A B\\Lore\\lore-cli.exe"));
        assert!(text.contains("\"C:/Users/A B/Lore/lore-cli.exe\" mark <decision|correction|lesson>"));
        assert_eq!(shell_command("/Applications/Lore.app/Contents/MacOS/lore-cli"), "/Applications/Lore.app/Contents/MacOS/lore-cli");
        assert!(!text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));
    }
}
