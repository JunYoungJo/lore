use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use similar::{Algorithm, ChangeTag, TextDiff};

use crate::error::{Error, Result};
use crate::queue::Activity;
use crate::run;
use crate::settings::which;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkingChanges {
    pub notes: Vec<FileChange>,
    pub others: Vec<FileChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Segment {
    pub kind: SegmentKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Same,
    Changed,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRow {
    pub kind: RowKind,
    pub left_no: Option<usize>,
    pub left: Option<Vec<Segment>>,
    pub right_no: Option<usize>,
    pub right: Option<Vec<Segment>>,
}

const GIT_LIMIT: Duration = Duration::from_secs(120);
const DIFF_LIMIT: Duration = Duration::from_secs(2);

pub fn is_note(path: &str) -> bool {
    (path.starts_with("common/") || path.starts_with("projects/")) && path.ends_with(".md")
}

pub fn is_repository(root: &Path) -> bool {
    root.join(".git").exists()
}

pub fn parse_status(output: &str) -> WorkingChanges {
    let mut changes = WorkingChanges::default();
    let mut fields = output.split('\0').filter(|field| !field.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let code = code.as_bytes();
        if matches!(code[0], b'R' | b'C') {
            fields.next();
        }
        let kind = if code.starts_with(b"??") || code[0] == b'A' {
            ChangeKind::Added
        } else if code[0] == b'D' || code[1] == b'D' {
            ChangeKind::Deleted
        } else {
            ChangeKind::Modified
        };
        let change = FileChange { path: path.to_string(), kind };
        if is_note(path) {
            changes.notes.push(change);
        } else {
            changes.others.push(change);
        }
    }
    changes.notes.sort_by(|a, b| a.path.cmp(&b.path));
    changes.others.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

pub fn word_diff(old: &str, new: &str) -> Vec<Segment> {
    let diff = TextDiff::configure().algorithm(Algorithm::Patience).timeout(DIFF_LIMIT).diff_words(old, new);
    let mut segments: Vec<Segment> = Vec::new();
    for change in diff.iter_all_changes() {
        let kind = match change.tag() {
            ChangeTag::Equal => SegmentKind::Same,
            ChangeTag::Insert => SegmentKind::Added,
            ChangeTag::Delete => SegmentKind::Removed,
        };
        match segments.last_mut() {
            Some(last) if last.kind == kind => last.text.push_str(change.value()),
            _ => segments.push(Segment { kind, text: change.value().to_string() }),
        }
    }
    segments
}

fn whole(kind: SegmentKind, text: &str) -> Option<Vec<Segment>> {
    Some(vec![Segment { kind, text: text.to_string() }])
}

fn one_side(segments: &[Segment], dropped: SegmentKind) -> Vec<Segment> {
    let mut side: Vec<Segment> = Vec::new();
    for segment in segments.iter().filter(|segment| segment.kind != dropped) {
        match side.last_mut() {
            Some(last) if last.kind == segment.kind => last.text.push_str(&segment.text),
            _ => side.push(segment.clone()),
        }
    }
    side
}

fn flush(rows: &mut Vec<DiffRow>, removed: &mut Vec<usize>, added: &mut Vec<usize>, old_lines: &[&str], new_lines: &[&str]) {
    for index in 0..removed.len().max(added.len()) {
        let row = match (removed.get(index).copied(), added.get(index).copied()) {
            (Some(left), Some(right)) => {
                let words = word_diff(old_lines[left], new_lines[right]);
                DiffRow {
                    kind: RowKind::Changed,
                    left_no: Some(left + 1),
                    left: Some(one_side(&words, SegmentKind::Added)),
                    right_no: Some(right + 1),
                    right: Some(one_side(&words, SegmentKind::Removed)),
                }
            }
            (Some(left), None) => DiffRow { kind: RowKind::Removed, left_no: Some(left + 1), left: whole(SegmentKind::Removed, old_lines[left]), right_no: None, right: None },
            (None, Some(right)) => DiffRow { kind: RowKind::Added, left_no: None, left: None, right_no: Some(right + 1), right: whole(SegmentKind::Added, new_lines[right]) },
            (None, None) => continue,
        };
        rows.push(row);
    }
    removed.clear();
    added.clear();
}

pub fn side_by_side(old: &str, new: &str) -> Vec<DiffRow> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut rows = Vec::new();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for operation in similar::capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines) {
        let (tag, old_range, new_range) = operation.as_tag_tuple();
        if tag == similar::DiffTag::Equal {
            flush(&mut rows, &mut removed, &mut added, &old_lines, &new_lines);
            for (left, right) in old_range.zip(new_range) {
                rows.push(DiffRow { kind: RowKind::Same, left_no: Some(left + 1), left: whole(SegmentKind::Same, old_lines[left]), right_no: Some(right + 1), right: whole(SegmentKind::Same, new_lines[right]) });
            }
        } else {
            removed.extend(old_range);
            added.extend(new_range);
        }
    }
    flush(&mut rows, &mut removed, &mut added, &old_lines, &new_lines);
    rows
}

pub fn since_commit(recent: &[Activity], last_commit: Option<i64>) -> Vec<&Activity> {
    recent.iter().filter(|activity| activity.kind == "recorded" && last_commit.is_none_or(|time| activity.at > time)).collect()
}

pub fn touching<'a>(since: &[&'a Activity], path: &str) -> Vec<&'a Activity> {
    since.iter().copied().filter(|activity| activity.notes.iter().any(|note| note == path)).collect()
}

pub fn local_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn record_message(date: &str) -> String {
    format!("Record memory notes ({date})")
}

/// The notes as they were when the user last reviewed them. Commits are made
/// automatically, so this private ref, not HEAD, is what review compares with.
pub const REVIEWED: &str = "refs/lore/reviewed";

fn git_with(root: &Path, args: &[&str], stdin: Option<&str>, env: &[(&str, String)]) -> Result<String> {
    let program = which("git").ok_or_else(|| Error::permanent("Git was not found. Install Git and add it to PATH."))?;
    let mut command = vec![program.to_string_lossy().to_string(), "-c".to_string(), "core.quotepath=false".to_string()];
    command.extend(args.iter().map(|arg| arg.to_string()));
    Ok(run::run(&command, root, env, stdin, &|| GIT_LIMIT)?.stdout)
}

fn git(root: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    git_with(root, args, stdin, &[])
}

fn resolve(root: &Path, revision: &str) -> Option<String> {
    let found = git(root, &["rev-parse", "--verify", "-q", &format!("{revision}^{{commit}}")], None).ok()?;
    Some(found.trim().to_string()).filter(|oid| !oid.is_empty())
}

pub fn reviewed(root: &Path) -> Option<String> {
    resolve(root, REVIEWED)
}

/// Start tracking review state from the current commit. Notes that are not
/// committed yet stay waiting for review, as they were before automatic commits.
pub fn ensure_reviewed(root: &Path) -> Result<()> {
    if is_repository(root) && reviewed(root).is_none() {
        if let Some(head) = resolve(root, "HEAD") {
            git(root, &["update-ref", REVIEWED, &head], None)?;
        }
    }
    Ok(())
}

/// An interrupted merge or rebase started outside Lore must be finished there.
pub fn in_progress(root: &Path) -> Result<bool> {
    for name in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "rebase-merge", "rebase-apply"] {
        let path = git(root, &["rev-parse", "--git-path", name], None)?;
        if root.join(path.trim()).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Commit every change in the memory folder. Returns false when nothing changed.
pub fn commit_all(root: &Path, message: &str) -> Result<bool> {
    if !is_repository(root) || git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], None)?.is_empty() {
        return Ok(false);
    }
    if in_progress(root)? {
        return Err(Error::permanent("Finish the Git merge or rebase in the memory folder first."));
    }
    git(root, &["add", "-A"], None)?;
    git(root, &["commit", "-q", "-F", "-"], Some(message))?;
    Ok(true)
}

/// Commit what a recording just wrote, so it can be shared without a manual step.
pub fn record_commit(root: &Path) -> Result<bool> {
    ensure_reviewed(root)?;
    commit_all(root, &record_message(&local_date()))
}

fn parse_name_status(output: &str) -> Vec<FileChange> {
    let mut changes = Vec::new();
    let mut fields = output.split('\0').filter(|field| !field.is_empty());
    while let (Some(code), Some(path)) = (fields.next(), fields.next()) {
        let kind = match code.as_bytes().first() {
            Some(b'A') => ChangeKind::Added,
            Some(b'D') => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        };
        changes.push(FileChange { path: path.to_string(), kind });
    }
    changes
}

/// Changes since the last review: committed or not, as long as the files differ
/// from the reviewed state. Without review state this is the uncommitted work.
pub fn working_changes(root: &Path) -> Result<Option<WorkingChanges>> {
    if !is_repository(root) {
        return Ok(None);
    }
    let status = parse_status(&git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], None)?);
    let Some(base) = reviewed(root) else { return Ok(Some(status)) };
    let mut all = parse_name_status(&git(root, &["diff", "--name-status", "--no-renames", "-z", &base, "--"], None)?);
    // A tree diff does not list files Git is not tracking yet.
    for change in status.notes.into_iter().chain(status.others) {
        if change.kind == ChangeKind::Added && !all.iter().any(|known| known.path == change.path) {
            all.push(change);
        }
    }
    all.sort_by(|a, b| a.path.cmp(&b.path));
    let (notes, others) = all.into_iter().partition(|change| is_note(&change.path));
    Ok(Some(WorkingChanges { notes, others }))
}

pub fn last_review_time(root: &Path) -> Option<i64> {
    let base = reviewed(root).unwrap_or_else(|| "HEAD".into());
    git(root, &["log", "-1", "--format=%ct", &base], None).ok()?.trim().parse().ok()
}

pub fn file_diff(root: &Path, path: &str) -> Result<Vec<DiffRow>> {
    let base = reviewed(root).unwrap_or_else(|| "HEAD".into());
    let committed = git(root, &["show", &format!("{base}:{path}")], None).unwrap_or_default().replace("\r\n", "\n");
    let current = match std::fs::read_to_string(root.join(path)) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    Ok(side_by_side(&committed, &current))
}

type Entries = std::collections::BTreeMap<String, (String, String)>;

fn entries(root: &Path, revision: &str) -> Result<Entries> {
    let mut found = Entries::new();
    for entry in git(root, &["ls-tree", "-r", "-z", revision], None)?.split('\0').filter(|entry| !entry.is_empty()) {
        if let Some((meta, path)) = entry.split_once('\t') {
            let mut parts = meta.split(' ');
            if let (Some(mode), Some(_), Some(oid)) = (parts.next(), parts.next(), parts.next()) {
                found.insert(path.to_string(), (mode.to_string(), oid.to_string()));
            }
        }
    }
    Ok(found)
}

/// Store `start` with some paths replaced as the new reviewed state. Uses a
/// private index, so the user's staging area and files are not touched.
fn store_reviewed(root: &Path, start: &str, replace: &[(String, Option<(String, String)>)]) -> Result<()> {
    let index = std::env::temp_dir().join(format!("lore-reviewed-{}-{}.index", std::process::id(), chrono::Utc::now().timestamp_micros()));
    let env = [("GIT_INDEX_FILE", index.to_string_lossy().to_string())];
    let result = (|| {
        git_with(root, &["read-tree", start], None, &env)?;
        for (path, entry) in replace {
            match entry {
                Some((mode, oid)) => git_with(root, &["update-index", "--add", "--cacheinfo", &format!("{mode},{oid},{path}")], None, &env)?,
                None => git_with(root, &["update-index", "--force-remove", "--", path], None, &env)?,
            };
        }
        let tree = git_with(root, &["write-tree"], None, &env)?;
        let commit = git(root, &["commit-tree", tree.trim(), "-F", "-"], Some("Reviewed memory notes\n"))?;
        git(root, &["update-ref", REVIEWED, commit.trim()], None)
    })();
    let _ = std::fs::remove_file(&index);
    result.map(|_| ())
}

/// Mark the checked notes as reviewed. Unchecked notes keep waiting; files that
/// are not notes, such as conversation copies, never wait.
pub fn mark_reviewed(root: &Path, notes: &[String]) -> Result<usize> {
    let changes = working_changes(root)?.ok_or_else(|| Error::permanent("The memory folder is not a Git repository."))?;
    if let Some(stray) = notes.iter().find(|note| !changes.notes.iter().any(|change| &change.path == *note)) {
        return Err(Error::permanent(format!("This note has not changed since the last review: {stray}. Close and reopen the review window.")));
    }
    if changes.notes.is_empty() && changes.others.is_empty() {
        return Err(Error::permanent("Nothing is waiting for review."));
    }
    record_commit(root)?;
    let head = resolve(root, "HEAD").ok_or_else(|| Error::permanent("The memory folder has no commits yet."))?;
    let unchecked: Vec<&FileChange> = changes.notes.iter().filter(|change| !notes.contains(&change.path)).collect();
    let before = match reviewed(root) {
        Some(base) if !unchecked.is_empty() => entries(root, &base)?,
        _ => {
            git(root, &["update-ref", REVIEWED, &head], None)?;
            return Ok(notes.len());
        }
    };
    let keep: Vec<_> = unchecked.into_iter().map(|change| (change.path.clone(), before.get(&change.path).cloned())).collect();
    store_reviewed(root, &head, &keep)?;
    Ok(notes.len())
}

/// Notes received from another PC were recorded and reviewed there, so they do
/// not wait for review here unless this PC also has unreviewed changes to them.
pub fn absorb(root: &Path, before: &str, after: &str) -> Result<()> {
    let Some(base) = reviewed(root) else { return Ok(()) };
    let tree = |revision: &str| git(root, &["rev-parse", &format!("{revision}^{{tree}}")], None);
    if tree(&base)? == tree(before)? {
        git(root, &["update-ref", REVIEWED, after], None)?;
        return Ok(());
    }
    let (reviewed_files, old, new) = (entries(root, &base)?, entries(root, before)?, entries(root, after)?);
    let incoming = git(root, &["diff", "--name-only", "--no-renames", "-z", before, after], None)?;
    let replace: Vec<_> = incoming
        .split('\0')
        .filter(|path| !path.is_empty() && reviewed_files.get(*path) == old.get(*path))
        .map(|path| (path.to_string(), new.get(path).cloned()))
        .collect();
    if replace.is_empty() {
        return Ok(());
    }
    store_reviewed(root, &base, &replace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Provider;

    #[test]
    fn status_separates_memory_notes_from_other_files() {
        let output = " M projects/demo/a.md\0?? projects/demo/new.md\0 D common/old.md\0 M sources/claude/s.md\0 M README.md\0R  projects/demo/b.md\0projects/demo/a2.md\0?? .qmd/config/index.yml\0A  projects/demo/staged.md\0";
        let changes = parse_status(output);
        let notes: Vec<(&str, ChangeKind)> = changes.notes.iter().map(|change| (change.path.as_str(), change.kind)).collect();
        assert_eq!(
            notes,
            vec![
                ("common/old.md", ChangeKind::Deleted),
                ("projects/demo/a.md", ChangeKind::Modified),
                ("projects/demo/b.md", ChangeKind::Modified),
                ("projects/demo/new.md", ChangeKind::Added),
                ("projects/demo/staged.md", ChangeKind::Added),
            ]
        );
        let others: Vec<&str> = changes.others.iter().map(|change| change.path.as_str()).collect();
        assert_eq!(others, vec![".qmd/config/index.yml", "README.md", "sources/claude/s.md"]);
        assert_eq!(parse_status(""), WorkingChanges::default());
    }

    fn joined(segments: &[Segment], skip: SegmentKind) -> String {
        segments.iter().filter(|segment| segment.kind != skip).map(|segment| segment.text.as_str()).collect()
    }

    fn side_text(cell: &Option<Vec<Segment>>) -> Option<String> {
        cell.as_ref().map(|segments| segments.iter().map(|segment| segment.text.as_str()).collect())
    }

    #[test]
    fn side_by_side_pairs_changed_lines_and_marks_the_changed_words_inside_them() {
        let old = "# 방침\n\n경고 팝업을 띄웁니다.\n\n## 남은 점\n로그 위치\n";
        let new = "# 방침\n\n경고 팝업 없이 로그만 남깁니다.\n\n## 변경 이유\n방해가 됩니다.\n\n## 남은 점\n로그 위치\n";
        let rows = side_by_side(old, new);
        let left: Vec<String> = rows.iter().filter_map(|row| side_text(&row.left)).collect();
        let right: Vec<String> = rows.iter().filter_map(|row| side_text(&row.right)).collect();
        assert_eq!(left, old.lines().collect::<Vec<_>>());
        assert_eq!(right, new.lines().collect::<Vec<_>>());
        let left_numbers: Vec<usize> = rows.iter().filter_map(|row| row.left_no).collect();
        let right_numbers: Vec<usize> = rows.iter().filter_map(|row| row.right_no).collect();
        assert_eq!(left_numbers, (1..=6).collect::<Vec<_>>());
        assert_eq!(right_numbers, (1..=9).collect::<Vec<_>>());

        let changed = rows.iter().find(|row| row.kind == RowKind::Changed).unwrap();
        let left_cell = changed.left.as_ref().unwrap();
        let right_cell = changed.right.as_ref().unwrap();
        assert!(left_cell.iter().any(|segment| segment.kind == SegmentKind::Removed && segment.text.contains("띄웁니다.")), "{left_cell:?}");
        assert!(left_cell.iter().all(|segment| segment.kind != SegmentKind::Added));
        assert!(right_cell.iter().any(|segment| segment.kind == SegmentKind::Added && segment.text.contains("로그만")), "{right_cell:?}");
        assert!(right_cell.iter().all(|segment| segment.kind != SegmentKind::Removed));
        assert!(left_cell.iter().any(|segment| segment.kind == SegmentKind::Same && segment.text.contains("경고")));

        let added = rows.iter().find(|row| side_text(&row.right).as_deref() == Some("## 변경 이유")).unwrap();
        assert_eq!((added.kind, added.left.is_none()), (RowKind::Added, true));
        let same = rows.iter().find(|row| side_text(&row.right).as_deref() == Some("로그 위치")).unwrap();
        assert_eq!((same.kind, same.left_no, same.right_no), (RowKind::Same, Some(6), Some(9)));
    }

    #[test]
    fn a_new_note_is_all_added_and_a_deleted_note_all_removed() {
        let added = side_by_side("", "# 새 문서\n\n본문\n");
        assert!(added.iter().all(|row| row.kind == RowKind::Added && row.left.is_none()));
        assert_eq!(added.iter().filter_map(|row| row.right_no).collect::<Vec<_>>(), vec![1, 2, 3]);
        let removed = side_by_side("# 지운 문서\n본문\n", "");
        assert!(removed.iter().all(|row| row.kind == RowKind::Removed && row.right.is_none()));
        assert_eq!(removed.len(), 2);
    }

    #[test]
    fn word_diff_rebuilds_both_texts_and_marks_only_the_changed_words() {
        let old = "## 결정\n\n경고 팝업을 띄웁니다. 장비 화면은 그대로 둡니다.\n";
        let new = "## 결정\n\n경고 팝업 없이 TP 로그만 남깁니다. 장비 화면은 그대로 둡니다.\n\n## 변경 이유\n\n방해가 됩니다.\n";
        let segments = word_diff(old, new);
        assert_eq!(joined(&segments, SegmentKind::Added), old);
        assert_eq!(joined(&segments, SegmentKind::Removed), new);
        let same: String = segments.iter().filter(|segment| segment.kind == SegmentKind::Same).map(|segment| segment.text.as_str()).collect();
        assert!(same.contains("장비 화면은 그대로 둡니다."), "{segments:?}");
        assert!(segments.iter().any(|segment| segment.kind == SegmentKind::Removed && segment.text.contains("띄웁니다.")), "{segments:?}");
        assert!(segments.iter().any(|segment| segment.kind == SegmentKind::Added && segment.text.contains("## 변경 이유")), "{segments:?}");
        assert!(segments.windows(2).all(|pair| pair[0].kind != pair[1].kind), "{segments:?}");
        assert_eq!(word_diff("", "새 문서"), vec![Segment { kind: SegmentKind::Added, text: "새 문서".into() }]);
        assert_eq!(word_diff("같음", "같음"), vec![Segment { kind: SegmentKind::Same, text: "같음".into() }]);
    }

    fn activity(id: i64, at: i64, project: &str, kind: &str, notes: &[&str], summary: &str) -> Activity {
        Activity {
            id,
            at,
            job_id: format!("job{id}"),
            provider: Provider::Claude,
            project: Some(project.into()),
            kind: kind.into(),
            summary: Some(summary.into()),
            notes: notes.iter().map(|note| note.to_string()).collect(),
            error: None,
        }
    }

    #[test]
    fn recordings_since_the_last_commit_are_matched_to_the_notes_they_changed() {
        let recent = vec![
            activity(4, 300, "demo", "recorded", &["projects/demo/a.md", "common/b.md"], "둘째 기록"),
            activity(3, 250, "demo", "no_change", &[], "바꿀 것 없음"),
            activity(2, 200, "demo", "recorded", &["projects/demo/a.md"], "첫 기록"),
            activity(1, 100, "demo", "recorded", &["projects/demo/a.md"], "커밋 전 기록"),
        ];
        let since = since_commit(&recent, Some(150));
        assert_eq!(since.iter().map(|item| item.id).collect::<Vec<_>>(), vec![4, 2]);
        assert_eq!(touching(&since, "projects/demo/a.md").iter().map(|item| item.id).collect::<Vec<_>>(), vec![4, 2]);
        assert_eq!(touching(&since, "common/b.md").iter().map(|item| item.id).collect::<Vec<_>>(), vec![4]);
        assert_eq!(since_commit(&recent, None).len(), 3);
    }

    #[test]
    fn a_tree_diff_lists_added_deleted_and_modified_files() {
        let changes = parse_name_status("M\0projects/demo/a.md\0A\0projects/demo/new.md\0D\0common/old.md\0");
        let listed: Vec<(&str, ChangeKind)> = changes.iter().map(|change| (change.path.as_str(), change.kind)).collect();
        assert_eq!(listed, vec![("projects/demo/a.md", ChangeKind::Modified), ("projects/demo/new.md", ChangeKind::Added), ("common/old.md", ChangeKind::Deleted)]);
    }

    #[test]
    fn the_automatic_commit_message_is_only_the_dated_title() {
        assert_eq!(record_message("2026-11-05"), "Record memory notes (2026-11-05)");
    }

    #[test]
    fn a_folder_is_reviewable_only_when_it_is_the_top_of_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_repository(dir.path()));
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_repository(dir.path()));
        let worktree = tempfile::tempdir().unwrap();
        std::fs::write(worktree.path().join(".git"), "gitdir: D:/elsewhere/.git/worktrees/w\n").unwrap();
        assert!(is_repository(worktree.path()));
    }
}
