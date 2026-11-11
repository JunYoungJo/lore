use serde::Serialize;

use crate::transcript::{Parsed, Turn};

pub const TURN_LIMIT: usize = 12_000;
pub const OMISSION_MARK: &str = "[… omitted …]";

#[derive(Debug, Clone, Serialize)]
pub struct Batch {
    pub new_turns: Vec<Turn>,
    pub context_turns: Vec<Turn>,
    pub omitted_context_turns: usize,
    pub end_line: usize,
}

pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

pub fn excerpt(text: &str, limit: usize) -> String {
    let length = char_len(text);
    if length <= limit {
        return text.to_string();
    }
    let head = limit * 3 / 4;
    let tail = limit - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(length - tail).collect();
    let omitted = length - head - tail;
    format!("{start}\n\n{OMISSION_MARK} {omitted} of {length} characters were left out of the middle.\n\n{end}")
}

pub fn make_batch(parsed: &Parsed, after_line: usize, max_chars: usize) -> Batch {
    let limit = TURN_LIMIT.min(max_chars);
    let shown = |turn: &Turn| Turn { text: excerpt(&turn.text, limit), ..turn.clone() };
    let new: Vec<Turn> = parsed.turns.iter().filter(|turn| turn.line > after_line).map(shown).collect();
    let mut selected = Vec::new();
    let mut size = 0;
    for turn in &new {
        let length = char_len(&turn.text);
        if !selected.is_empty() && size + length > max_chars {
            break;
        }
        size += length;
        selected.push(turn.clone());
    }
    let prior: Vec<Turn> = parsed.turns.iter().filter(|turn| turn.line <= after_line).map(shown).collect();
    let mut context = Vec::new();
    let mut context_size = 0;
    for turn in prior.iter().rev() {
        let length = char_len(&turn.text);
        if context_size + length > max_chars {
            break;
        }
        context_size += length;
        context.push(turn.clone());
    }
    context.reverse();
    let end_line = if selected.len() < new.len() {
        selected.last().map(|turn| turn.line).unwrap_or(after_line)
    } else {
        parsed.complete_lines.max(after_line)
    };
    Batch { omitted_context_turns: prior.len() - context.len(), new_turns: selected, context_turns: context, end_line }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Origin;

    fn turn(line: usize, text: &str) -> Turn {
        Turn { reference: format!("L{line}"), line, role: "user".into(), origin: Origin::User, timestamp: String::new(), text: text.into() }
    }

    fn parsed(turns: Vec<Turn>, complete_lines: usize) -> Parsed {
        Parsed { turns, complete_lines, ..Parsed::default() }
    }

    fn refs(turns: &[Turn]) -> Vec<&str> {
        turns.iter().map(|turn| turn.reference.as_str()).collect()
    }

    #[test]
    fn batch_stops_at_the_limit_and_keeps_prior_context() {
        let input = parsed(vec![turn(1, "aaaaaa"), turn(2, "bbbbbb"), turn(3, "cccccc")], 4);
        let batch = make_batch(&input, 1, 10);
        assert_eq!(refs(&batch.new_turns), vec!["L2"]);
        assert_eq!(refs(&batch.context_turns), vec!["L1"]);
        assert_eq!(batch.end_line, 2);
        assert_eq!(batch.omitted_context_turns, 0);
    }

    #[test]
    fn finished_batch_consumes_every_complete_line() {
        let input = parsed(vec![turn(1, "aaaaaa"), turn(3, "bbbbbb")], 5);
        let batch = make_batch(&input, 0, 100);
        assert_eq!(refs(&batch.new_turns), vec!["L1", "L3"]);
        assert_eq!(batch.end_line, 5);
    }

    #[test]
    fn oversized_turn_is_shown_as_head_and_tail_instead_of_failing() {
        let long = format!("{}{}{}", "가".repeat(20), "나".repeat(60), "다".repeat(20));
        let input = parsed(vec![turn(1, &long), turn(2, "next")], 2);
        let batch = make_batch(&input, 0, 40);
        assert_eq!(refs(&batch.new_turns), vec!["L1"]);
        assert_eq!(batch.end_line, 1);
        let shown = &batch.new_turns[0].text;
        assert!(shown.starts_with(&"가".repeat(20)));
        assert!(shown.ends_with(&"다".repeat(10)));
        assert!(shown.contains("[… omitted …] 60 of 100 characters were left out of the middle."), "{shown}");
        assert!(!shown.contains(&"나".repeat(60)));
    }

    #[test]
    fn context_that_does_not_fit_is_counted_as_omitted() {
        let input = parsed(vec![turn(1, "aaaaaa"), turn(2, "bbbbbb"), turn(3, "cc")], 3);
        let batch = make_batch(&input, 2, 8);
        assert_eq!(refs(&batch.context_turns), vec!["L2"]);
        assert_eq!(batch.omitted_context_turns, 1);
    }

    #[test]
    fn no_new_turns_yields_empty_batch_at_the_end() {
        let input = parsed(vec![turn(1, "aaaaaa")], 3);
        let batch = make_batch(&input, 3, 10);
        assert!(batch.new_turns.is_empty());
        assert_eq!(batch.end_line, 3);
    }
}
