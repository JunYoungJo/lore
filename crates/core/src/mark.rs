//! Marks: what a session, while it works, says is to be remembered.
//!
//! A hook tells the session to run `lore-cli mark`. The command writes nothing: it prints one
//! line, and that line lands in the session file as the result of a tool call. The app reads the
//! marks from there when the session is prepared, so marking works inside a sandbox and needs no
//! path into the app's data. Only what a session marks becomes a candidate: nothing is looked for
//! in the conversation afterwards. The session knows who said what, so a mark says it.

use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::candidate::{Kind, Scope, Speaker};
use crate::error::{Error, Result};

pub const PREFIX: &str = "LORE-MARK ";
/// A mark is a sentence or three. A longer one is a report, which the user cannot check at a glance.
pub const TEXT_LIMIT: usize = 600;
/// Output wrapped as JSON inside JSON is unwrapped this many times at most.
const DEPTH: usize = 3;

const USAGE: &str = "Use mark <decision|correction|lesson> \"<sentences>\" --by <user|agent> [--said \"<the user's words>\"]... [--common] [--updates <note> | --retires <note>] [--quote \"<the note's words>\"]... [--replaces <id>], or mark cancel <id>.";

/// What a mark does to a note that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// The note says something that the mark changes.
    Updates,
    /// What the note says no longer applies.
    Retires,
}

/// A note of the memory folder that the session read and names, with the words of it that the mark changes or ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub change: Change,
    /// As the session gave it: relative to the memory folder, or as qmd lists it.
    pub path: String,
    pub quotes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// Printed with the mark, so that the session can replace or withdraw it later.
    pub id: String,
    pub kind: Kind,
    pub text: String,
    /// Whose it is: the user decided, corrected or stated it, or the agent proposed or found it.
    pub by: Speaker,
    /// The user's words it rests on, as the session copied them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub said: Vec<String>,
    #[serde(default)]
    pub scope: Scope,
    /// The id of an earlier mark that this one takes the place of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Target>,
}

impl Mark {
    /// Why the mark cannot be kept as it is, if it cannot.
    pub fn problem(&self) -> Option<String> {
        let length = self.text.chars().count();
        if length == 0 {
            return Some(format!("The mark has no sentence. {USAGE}"));
        }
        if length > TEXT_LIMIT {
            return Some(format!("A mark is one to three sentences, at most {TEXT_LIMIT} characters; this one has {length}."));
        }
        if self.kind == Kind::Correction && self.by == Speaker::Agent {
            return Some("A correction is the user's: use --by user.".into());
        }
        if self.kind != Kind::Lesson && self.said.is_empty() {
            return Some("A decision or a correction rests on what the user said: add --said \"<the user's words, copied exactly>\".".into());
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Entry {
    Mark(Mark),
    /// The id of an earlier mark that the session withdraws.
    Cancel(String),
}

/// What the session printed and the line of the session file it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub line: usize,
    /// When it was printed, as the session file has it. Empty when the file has no time.
    pub at: String,
    pub entry: Entry,
}

/// A mark that is still in force after what the session withdrew and replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marked {
    pub line: usize,
    pub at: String,
    pub mark: Mark,
}

/// The marks of one step of a session, once the replacements and withdrawals in it are applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub marks: Vec<Marked>,
    /// Marks of earlier steps that this step withdrew or replaced.
    pub withdrawn: Vec<String>,
}

fn take_back(id: &str, marks: &mut Vec<Marked>, withdrawn: &mut Vec<String>) {
    let before = marks.len();
    marks.retain(|marked| marked.mark.id != id);
    if marks.len() == before && !withdrawn.iter().any(|earlier| earlier == id) {
        withdrawn.push(id.to_string());
    }
}

/// Apply, in the order they were printed, what a step's marks replace and withdraw. What they
/// name and this step does not hold was marked in an earlier step.
pub fn resolve(placed: &[Placed]) -> Resolved {
    let mut resolved = Resolved::default();
    for item in placed {
        match &item.entry {
            Entry::Mark(mark) => {
                if resolved.marks.iter().any(|marked| marked.mark.id == mark.id) {
                    continue;
                }
                if let Some(earlier) = &mark.replaces {
                    take_back(earlier, &mut resolved.marks, &mut resolved.withdrawn);
                }
                resolved.marks.push(Marked { line: item.line, at: item.at.clone(), mark: mark.clone() });
            }
            Entry::Cancel(id) => take_back(id, &mut resolved.marks, &mut resolved.withdrawn),
        }
    }
    resolved
}

/// A short id for a new mark. A session sees it in the command's output and names it to replace
/// or withdraw the mark; it only has to differ from the other marks of that session.
pub fn new_id() -> String {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0));
    hasher.write_u32(std::process::id());
    format!("{:06x}", hasher.finish() & 0xff_ffff)
}

/// What `lore-cli mark` was given, after `mark`. `id` is the id a new mark gets.
pub fn from_args(args: &[String], id: String) -> Result<Entry> {
    let usage = || Error::permanent(USAGE);
    if args.first().map(String::as_str) == Some("cancel") {
        let target = args.get(1).map(|value| value.trim()).filter(|value| !value.is_empty()).ok_or_else(usage)?;
        if args.len() > 2 {
            return Err(usage());
        }
        return Ok(Entry::Cancel(target.to_string()));
    }
    let kind = args.first().and_then(|value| Kind::parse(value)).ok_or_else(usage)?;
    let (mut words, mut by, mut said, mut scope, mut replaces) = (Vec::new(), None, Vec::new(), Scope::Project, None);
    let (mut targets, mut quotes): (Vec<(Change, String)>, Vec<String>) = (Vec::new(), Vec::new());
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            flag @ ("--updates" | "--retires") => {
                let change = if flag == "--updates" { Change::Updates } else { Change::Retires };
                targets.push((change, rest.next().map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).ok_or_else(usage)?));
            }
            "--quote" => {
                let words = rest.next().ok_or_else(usage)?.trim();
                if !words.is_empty() {
                    quotes.push(words.to_string());
                }
            }
            "--by" => {
                by = Some(match rest.next().map(String::as_str) {
                    Some("user") => Speaker::User,
                    Some("agent") => Speaker::Agent,
                    _ => return Err(Error::permanent(format!("--by is user or agent. {USAGE}"))),
                })
            }
            "--said" => {
                let words = rest.next().ok_or_else(usage)?.trim();
                if !words.is_empty() {
                    said.push(words.to_string());
                }
            }
            "--common" => scope = Scope::Common,
            "--replaces" => replaces = Some(rest.next().map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).ok_or_else(usage)?),
            flag if flag.starts_with("--") => return Err(Error::permanent(format!("Unknown option {flag}. {USAGE}"))),
            word => words.push(word),
        }
    }
    let by = by.ok_or_else(|| Error::permanent(format!("Say whose it is with --by user or --by agent. {USAGE}")))?;
    let note = match (targets.len(), quotes.is_empty()) {
        (0, true) => None,
        (0, false) => return Err(Error::permanent("--quote gives the words of the note named by --updates or --retires; name the note.")),
        (1, true) => return Err(Error::permanent("Add --quote \"<the words of the note that this changes or ends, copied exactly>\".")),
        (1, false) => targets.pop().map(|(change, path)| Target { change, path, quotes }),
        _ => return Err(Error::permanent("A mark names one note, with --updates or --retires once. Mark another note's change separately.")),
    };
    let mark = Mark { id, kind, text: words.join(" ").trim().to_string(), by, said, scope, replaces, note };
    match mark.problem() {
        Some(problem) => Err(Error::permanent(problem)),
        None => Ok(Entry::Mark(mark)),
    }
}

/// The line `lore-cli mark` prints.
pub fn line(entry: &Entry) -> String {
    format!("{PREFIX}{}", serde_json::to_string(entry).unwrap_or_default())
}

fn readable(entry: &Entry) -> bool {
    match entry {
        Entry::Mark(mark) => !mark.id.trim().is_empty() && !mark.text.trim().is_empty(),
        Entry::Cancel(id) => !id.trim().is_empty(),
    }
}

fn collect(value: &Value, depth: usize, found: &mut Vec<Entry>) {
    match value {
        Value::String(text) => {
            let before = found.len();
            for line in text.lines() {
                let Some(rest) = line.trim().strip_prefix(PREFIX) else { continue };
                if let Ok(entry) = serde_json::from_str::<Entry>(rest.trim()) {
                    if readable(&entry) && !found.contains(&entry) {
                        found.push(entry);
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

/// What the output of a tool call marked, in the order it was printed.
pub fn entries_in(output: &Value) -> Vec<Entry> {
    let mut found = Vec::new();
    collect(output, 0, &mut found);
    found
}

/// What the hook adds to each prompt. `cli` is the command that runs `lore-cli`, quoted for a shell.
pub fn instruction(cli: &str) -> String {
    format!(
        "Lore keeps for later sessions only what you mark here; nothing is looked for in the conversation afterwards. \
         When, in this session, the user decides something, corrects how you work, or the work establishes a lesson a later session would need, run this for it at once and carry on: \
         {cli} mark <decision|correction|lesson> \"<one to three sentences that a later session understands without this conversation: what holds, the reason when one was given, and where it applies>\" --by <user|agent> [--said \"<the user's words it rests on, copied exactly>\"]... [--common] \
         --by user: the user decided, corrected or stated it. --by agent: you proposed it and the user approved it, or you found it; put the user's approval in --said. \
         A decision or a correction needs --said. Add --common when it applies to every project, not only this one. \
         When it changes what a note of Lore's memory says (one you read with qmd), add --updates <the note's path>; when what the note says no longer applies, add --retires <the note's path>; with either, add --quote \"<the note's words it changes or ends, copied exactly>\". \
         The command prints the mark with its id. When the user changes it later, mark the new version with --replaces <id>; to withdraw a mark that was wrong, run {cli} mark cancel <id>. \
         Do not mark routine progress. The user reviews every mark before it is kept."
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

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    fn mark(id: &str, kind: Kind, text: &str) -> Mark {
        Mark { id: id.into(), kind, text: text.into(), by: Speaker::User, said: vec!["no alerts".into()], scope: Scope::Project, replaces: None, note: None }
    }

    fn placed(line: usize, entry: Entry) -> Placed {
        Placed { line, at: String::new(), entry }
    }

    #[test]
    fn a_printed_mark_is_read_back_from_plain_and_from_wrapped_output() {
        let first = Entry::Mark(Mark { by: Speaker::Agent, said: vec!["알겠어 일단 놔두자.".into()], replaces: Some("0a1b2c".into()), ..mark("3fa9c2", Kind::Decision, "No desktop alerts for finished jobs; \"quiet\" was the user's word.") });
        let second = Entry::Cancel("0a1b2c".into());
        let printed = format!("{}\nother output\n  {}\n", line(&first), line(&second));
        assert_eq!(entries_in(&json!(printed)), vec![first.clone(), second.clone()]);
        // Claude Code: a list of text blocks. Codex: the output as a JSON text.
        assert_eq!(entries_in(&json!([{"type": "text", "text": printed}])), vec![first.clone(), second.clone()]);
        let wrapped = json!({"output": printed, "metadata": {"exit_code": 0}}).to_string();
        assert_eq!(entries_in(&json!(wrapped)), vec![first, second]);
    }

    #[test]
    fn text_that_only_mentions_the_prefix_is_not_a_mark() {
        assert!(entries_in(&json!("Run LORE-MARK later")).is_empty());
        assert!(entries_in(&json!("LORE-MARK not json")).is_empty());
        assert!(entries_in(&json!("LORE-MARK {\"mark\":{\"id\":\"a\",\"kind\":\"rumour\",\"text\":\"x\",\"by\":\"user\"}}")).is_empty());
        assert!(entries_in(&json!("LORE-MARK {\"mark\":{\"id\":\"a\",\"kind\":\"lesson\",\"text\":\"  \",\"by\":\"user\"}}")).is_empty());
        assert!(entries_in(&json!("LORE-MARK {\"cancel\":\" \"}")).is_empty());
        // A mark without an id or without whose it is cannot be replaced or attributed.
        assert!(entries_in(&json!("LORE-MARK {\"kind\":\"lesson\",\"text\":\"Pages time out.\"}")).is_empty());
        assert!(entries_in(&json!("LORE-MARK {\"mark\":{\"id\":\"a\",\"kind\":\"lesson\",\"text\":\"Pages time out.\"}}")).is_empty());
        let once = line(&Entry::Mark(mark("a", Kind::Correction, "Ask before deleting.")));
        assert_eq!(entries_in(&json!(format!("{once}\n{once}"))).len(), 1);
    }

    #[test]
    fn the_command_line_becomes_a_mark_with_whose_it_is_and_the_users_words() {
        let entry = from_args(&args(&["decision", "Finished jobs raise no desktop alert.", "--by", "agent", "--said", " 알겠어 일단 놔두자. ", "--said", "", "--common", "--replaces", "0a1b2c"]), "3fa9c2".into()).unwrap();
        let expected = Mark { by: Speaker::Agent, said: vec!["알겠어 일단 놔두자.".into()], scope: Scope::Common, replaces: Some("0a1b2c".into()), ..mark("3fa9c2", Kind::Decision, "Finished jobs raise no desktop alert.") };
        assert_eq!(entry, Entry::Mark(expected));
        // A sentence given as several words is joined; a lesson of the agent needs none of the user's words.
        let lesson = from_args(&args(&["lesson", "Paging", "fixes", "the", "timeout.", "--by", "agent"]), "b".into()).unwrap();
        assert_eq!(lesson, Entry::Mark(Mark { by: Speaker::Agent, said: Vec::new(), ..mark("b", Kind::Lesson, "Paging fixes the timeout.") }));
        assert_eq!(from_args(&args(&["cancel", " 3fa9c2 "]), "c".into()).unwrap(), Entry::Cancel("3fa9c2".into()));
    }

    #[test]
    fn a_mark_names_the_note_it_changes_or_ends_with_the_words_of_it() {
        let updates = from_args(&args(&["decision", "No alerts.", "--by", "user", "--said", "no alerts", "--updates", "project-demo/alerts.md", "--quote", " Every job alerts. ", "--quote", "Alerts are on."]), "a".into()).unwrap();
        let target = Target { change: Change::Updates, path: "project-demo/alerts.md".into(), quotes: vec!["Every job alerts.".into(), "Alerts are on.".into()] };
        assert_eq!(updates, Entry::Mark(Mark { note: Some(target.clone()), ..mark("a", Kind::Decision, "No alerts.") }));
        let retires = from_args(&args(&["decision", "The export is gone.", "--by", "user", "--said", "no alerts", "--retires", "projects/demo/export.md", "--quote", "The export pages."]), "b".into()).unwrap();
        let Entry::Mark(retired) = &retires else { panic!("expected a mark") };
        assert_eq!(retired.note.as_ref().map(|note| note.change), Some(Change::Retires));
        // The printed line carries the note, and is read back as it was printed.
        assert_eq!(entries_in(&json!(line(&updates))), vec![updates]);
        let refused = |words: &[&str]| from_args(&args(words), "a".into()).unwrap_err().message;
        assert!(refused(&["decision", "x", "--by", "user", "--said", "no alerts", "--updates", "a.md"]).contains("--quote"));
        assert!(refused(&["decision", "x", "--by", "user", "--said", "no alerts", "--quote", "y"]).contains("name the note"));
        assert!(refused(&["decision", "x", "--by", "user", "--said", "no alerts", "--updates", "a.md", "--retires", "b.md", "--quote", "y"]).contains("one note"));
    }

    #[test]
    fn a_mark_that_cannot_be_kept_as_given_is_refused_with_what_to_change() {
        let refused = |words: &[&str]| from_args(&args(words), "a".into()).unwrap_err().message;
        assert!(refused(&["decision", "No alerts.", "--said", "no alerts"]).contains("--by user or --by agent"));
        assert!(refused(&["decision", "No alerts.", "--by", "someone", "--said", "x"]).contains("--by is user or agent"));
        assert!(refused(&["decision", "No alerts.", "--by", "user"]).contains("--said"));
        assert!(refused(&["correction", "Ask first.", "--by", "agent", "--said", "ask me"]).contains("--by user"));
        assert!(refused(&["lesson", "--by", "agent"]).contains("no sentence"));
        assert!(refused(&["lesson", &"word ".repeat(200), "--by", "agent"]).contains("at most"));
        assert!(refused(&["lesson", "Pages.", "--by", "agent", "--why", "x"]).contains("Unknown option --why"));
        assert!(refused(&["rumour", "x", "--by", "user"]).contains("mark cancel <id>"));
        assert!(refused(&["cancel"]).contains("mark cancel <id>"));
        assert!(refused(&["cancel", "a", "b"]).contains("mark cancel <id>"));
    }

    #[test]
    fn replacements_and_withdrawals_apply_in_order_and_what_an_earlier_step_holds_is_withdrawn() {
        let entry = |id: &str, replaces: Option<&str>| Entry::Mark(Mark { replaces: replaces.map(str::to_string), ..mark(id, Kind::Decision, id) });
        let resolved = resolve(&[
            placed(3, entry("a", None)),
            placed(4, entry("b", Some("a"))),
            placed(5, entry("c", None)),
            placed(6, Entry::Cancel("c".into())),
            placed(7, entry("d", Some("old1"))),
            placed(8, Entry::Cancel("old2".into())),
            placed(9, Entry::Cancel("old2".into())),
            placed(10, entry("b", None)),
        ]);
        assert_eq!(resolved.marks.iter().map(|marked| (marked.line, marked.mark.id.as_str())).collect::<Vec<_>>(), vec![(4, "b"), (7, "d")]);
        assert_eq!(resolved.withdrawn, vec!["old1".to_string(), "old2".to_string()]);
    }

    #[test]
    fn a_new_id_is_six_hex_digits() {
        let id = new_id();
        assert_eq!(id.len(), 6);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{id}");
    }

    #[test]
    fn the_instruction_names_the_command_its_options_and_how_to_take_a_mark_back() {
        let text = instruction(&shell_command("C:\\Users\\A B\\Lore\\lore-cli.exe"));
        assert!(text.contains("\"C:/Users/A B/Lore/lore-cli.exe\" mark <decision|correction|lesson>"));
        for option in ["--by <user|agent>", "--said", "--common", "--updates <the note's path>", "--retires <the note's path>", "--quote", "--replaces <id>", "\"C:/Users/A B/Lore/lore-cli.exe\" mark cancel <id>"] {
            assert!(text.contains(option), "{option}");
        }
        assert_eq!(shell_command("/Applications/Lore.app/Contents/MacOS/lore-cli"), "/Applications/Lore.app/Contents/MacOS/lore-cli");
        assert!(!text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c)));
    }
}
