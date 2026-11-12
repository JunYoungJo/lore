use lore_core::review;
use lore_core::sync::{self};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A time after every scheduled retry, for asking what is waiting at all.
const LATER: i64 = i64::MAX;

struct Fixture {
    _dir: tempfile::TempDir,
    local: PathBuf,
    other: PathBuf,
    folder: PathBuf,
}
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn config(root: &Path) {
    git(root, &["config", "user.name", "Sync test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["config", "core.autocrlf", "false"]);
}
fn write(root: &Path, file: &str, text: &str) {
    let file = root.join(file);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}
fn read(root: &Path, file: &str) -> String {
    std::fs::read_to_string(root.join(file)).unwrap()
}
fn commit(root: &Path) {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "fixture"]);
}
const NOTE: &str = "projects/demo/policy.md";
const OTHER_NOTE: &str = "projects/elsewhere/plan.md";
const BASE: &str =
    "# 오류 알림\n\n## 결정\n\n오류 안내는 미정입니다.\n\n## 근거\n\n- 처음 논의\n";
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(
        root,
        &["init", "--bare", "--initial-branch=main", "remote.git"],
    );
    // Exercise Windows-style machine settings on every OS. A clone must set
    // its line-ending policy before checkout; changing it afterwards makes
    // existing notes appear dirty and invalidates the divergence scenarios.
    let global = root.join("clone.gitconfig");
    std::fs::write(&global, "[core]\n\tautocrlf = true\n").unwrap();
    let clone = |name: &str| {
        let output = Command::new("git").env("GIT_CONFIG_GLOBAL", &global)
            .args(["clone", "--config", "core.autocrlf=false", "remote.git", name]).current_dir(root).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    };
    clone("local");
    let local = root.join("local");
    config(&local);
    write(&local, NOTE, BASE);
    commit(&local);
    git(&local, &["push", "-u", "origin", "main"]);
    clone("other");
    let other = root.join("other");
    config(&other);
    Fixture {
        local,
        other,
        folder: root.join("app/sync"),
        _dir: dir,
    }
}
fn run(f: &Fixture) -> sync::Outcome {
    sync::sync(&f.local, &f.folder, || Ok(())).unwrap()
}
fn remote_head(f: &Fixture) -> String {
    git(&f.local, &["ls-remote", "origin", "refs/heads/main"])
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}
/// Both PCs add a line at the end of the same list.
fn append_on_both(f: &Fixture) {
    write(&f.local, NOTE, &format!("{BASE}- 이 PC 근거\n"));
    write(&f.other, NOTE, &format!("{BASE}- 다른 PC 근거\n"));
    commit(&f.other);
    git(&f.other, &["push"]);
}
/// Both PCs record a different decision in the same sentence.
fn contradict(f: &Fixture) {
    write(
        &f.local,
        NOTE,
        &BASE.replace("오류 안내는 미정입니다.", "팝업 없이 로그만 남깁니다."),
    );
    write(
        &f.other,
        NOTE,
        &BASE.replace("오류 안내는 미정입니다.", "오류 팝업을 표시합니다."),
    );
    commit(&f.other);
    git(&f.other, &["push"]);
}
fn reply(issues: Value) -> Value {
    json!({"summary":"Compared the notes.","issues":issues,"notes":"Reviewed.","reads":[],"replace_issues":[]})
}
fn decision(source: &str, quote: &str) -> Value {
    json!([{
        "title":"Error notification policy",
        "explanation":"The two PCs recorded different policies.",
        "evidence":[{"source":source,"quote":quote}],
        "options":[
            {"label":"Log only","description":"Keep this PC's policy.","edits":[{"path":NOTE,"find":"오류 팝업을 표시합니다.\n","text":""}]},
            {"label":"Show a popup","description":"Keep the other PC's policy.","edits":[{"path":NOTE,"find":"팝업 없이 로그만 남깁니다.\n","text":""}]}
        ]
    }])
}
fn compare(f: &Fixture, replies: &mut dyn FnMut(usize) -> Value) -> lore_core::error::Result<usize> {
    let id = sync::pending_comparison(&f.folder, LATER).expect("a comparison is waiting");
    let mut calls = 0;
    sync::compare(&f.folder, &id, &mut |_, _, _| {
        calls += 1;
        Ok(replies(calls))
    }, &mut || {})
}

#[test]
fn a_recorded_note_is_committed_and_shared_without_any_user_step() {
    let f = fixture();
    write(&f.local, OTHER_NOTE, "# 계획\n\n새 기록입니다.\n");
    let outcome = run(&f);
    assert!(outcome.sent);
    assert_eq!(git(&f.local, &["status", "--porcelain"]), "");
    assert_eq!(remote_head(&f), git(&f.local, &["rev-parse", "HEAD"]));
    git(&f.other, &["pull", "--ff-only"]);
    assert_eq!(read(&f.other, OTHER_NOTE), "# 계획\n\n새 기록입니다.\n");
    let summary = sync::summary(&f.folder);
    assert_eq!(summary.phase, "idle");
    assert!(summary.last_synced_at.is_some());
}

#[test]
fn notes_changed_on_different_pcs_merge_without_a_review() {
    let f = fixture();
    write(&f.local, OTHER_NOTE, "# 계획\n\n이 PC 기록입니다.\n");
    write(&f.other, "projects/third/note.md", "# 다른 PC\n");
    commit(&f.other);
    git(&f.other, &["push"]);
    let mut indexed = 0;
    let outcome = sync::sync(&f.local, &f.folder, || {
        indexed += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(outcome.received, 1);
    assert_eq!(indexed, 1);
    assert_eq!(read(&f.local, "projects/third/note.md"), "# 다른 PC\n");
    assert_eq!(
        git(&f.local, &["show", "-s", "--format=%P"]).split_whitespace().count(),
        2
    );
    assert_eq!(remote_head(&f), git(&f.local, &["rev-parse", "HEAD"]));
    let summary = sync::summary(&f.folder);
    assert_eq!(summary.items, 0);
    assert!(!summary.comparing);
}

#[test]
fn lines_added_at_the_same_place_on_both_pcs_are_both_kept_and_the_note_is_listed() {
    let f = fixture();
    append_on_both(&f);
    let outcome = run(&f);
    let merged = read(&f.local, NOTE);
    assert!(merged.contains("- 이 PC 근거\n") && merged.contains("- 다른 PC 근거\n"));
    assert!(!merged.contains("<<<<<<<") && !merged.contains(">>>>>>>"));
    assert_eq!(remote_head(&f), git(&f.local, &["rev-parse", "HEAD"]));
    assert_eq!(outcome.items, 1);
    let state = sync::state(&f.folder);
    assert_eq!(state.items.len(), 1);
    assert_eq!(state.items[0].kind, "merged");
    assert_eq!(state.items[0].path, NOTE);
    let detail = sync::detail(&f.folder, &state.items[0].id).unwrap();
    assert_eq!(detail.files[0].local.as_deref(), Some(format!("{BASE}- 이 PC 근거\n").as_str()));
    assert_eq!(detail.files[0].merged.as_deref(), Some(merged.as_str()));
    sync::dismiss_item(&f.folder, &state.items[0].id).unwrap();
    assert_eq!(sync::summary(&f.folder).items, 0);
}

#[test]
fn a_note_deleted_on_one_pc_and_edited_on_the_other_keeps_the_edited_note() {
    let f = fixture();
    write(&f.local, NOTE, &format!("{BASE}- 이 PC 근거\n"));
    git(&f.other, &["rm", "-q", NOTE]);
    git(&f.other, &["commit", "-m", "delete"]);
    git(&f.other, &["push"]);
    run(&f);
    assert_eq!(read(&f.local, NOTE), format!("{BASE}- 이 PC 근거\n"));
    let state = sync::state(&f.folder);
    assert_eq!(state.items[0].kind, "kept");
    assert_eq!(remote_head(&f), git(&f.local, &["rev-parse", "HEAD"]));
}

#[test]
fn incoming_notes_refresh_search_and_a_failed_refresh_is_retried_without_blocking_sharing() {
    let f = fixture();
    write(&f.other, NOTE, &BASE.replace("미정입니다", "로그만 남깁니다"));
    commit(&f.other);
    git(&f.other, &["push"]);
    let failed = sync::sync(&f.local, &f.folder, || {
        Err(lore_core::error::Error::transient("index fixture failed"))
    });
    assert!(failed.unwrap_err().to_string().contains("index fixture failed"));
    assert!(read(&f.local, NOTE).contains("로그만 남깁니다"));
    assert!(sync::state(&f.folder).index_pending);
    let mut indexed = 0;
    sync::sync(&f.local, &f.folder, || {
        indexed += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(indexed, 1);
    let state = sync::state(&f.folder);
    assert!(!state.index_pending);
    assert_eq!(state.failures, 0);
}

#[test]
fn failures_are_counted_until_a_sync_succeeds() {
    let f = fixture();
    git(&f.local, &["remote", "set-url", "origin", "missing.git"]);
    assert!(sync::sync(&f.local, &f.folder, || Ok(())).is_err());
    assert_eq!(sync::summary(&f.folder).phase, "retrying");
    assert!(sync::sync(&f.local, &f.folder, || Ok(())).is_err());
    assert!(sync::sync(&f.local, &f.folder, || Ok(())).is_err());
    let summary = sync::summary(&f.folder);
    assert_eq!(summary.phase, "failing");
    assert!(summary.message.is_some());
    git(&f.local, &["remote", "set-url", "origin", "../remote.git"]);
    run(&f);
    assert_eq!(sync::summary(&f.folder).phase, "idle");
}

#[test]
fn a_review_left_waiting_by_an_older_version_no_longer_holds_sync() {
    let f = fixture();
    write(&f.folder, "pending.json", "{\"phase\":\"review\"}");
    write(&f.local, OTHER_NOTE, "# 계획\n");
    assert!(run(&f).sent);
    assert!(!f.folder.join("pending.json").exists());
    assert!(std::fs::read_dir(&f.folder).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("superseded-")));
}

#[test]
fn tracking_names_the_branch_and_its_upstream() {
    let f = fixture();
    assert_eq!(sync::tracking(&f.local).as_deref(), Some("main ↔ origin/main"));
    git(&f.local, &["branch", "--unset-upstream"]);
    assert_eq!(sync::tracking(&f.local), None);
    assert!(sync::sync(&f.local, &f.folder, || Ok(())).unwrap_err().to_string().contains("upstream"));
}

#[test]
fn notes_changed_on_both_pcs_are_compared_after_sync_and_a_decision_is_applied_as_a_normal_commit() {
    let f = fixture();
    contradict(&f);
    run(&f);
    assert!(sync::summary(&f.folder).comparing);
    let merged = read(&f.local, NOTE);
    assert!(merged.contains("팝업 없이 로그만 남깁니다.") && merged.contains("오류 팝업을 표시합니다."));
    let found = compare(&f, &mut |_| reply(decision(&format!("remote:{NOTE}"), "오류 팝업을 표시합니다."))).unwrap();
    assert_eq!(found, 1);
    let state = sync::state(&f.folder);
    assert!(state.comparisons.is_empty());
    let item = state.items.iter().find(|item| item.kind == "decision").unwrap().clone();
    let detail = sync::detail(&f.folder, &item.id).unwrap();
    assert_eq!(detail.issue.as_ref().unwrap().options.len(), 2);
    assert_eq!(detail.sources[0].text, BASE.replace("오류 안내는 미정입니다.", "오류 팝업을 표시합니다."));
    let preview = sync::preview_item(&f.local, &f.folder, &item.id, 0, None).unwrap();
    assert!(!preview[NOTE].1.contains("오류 팝업을 표시합니다."));
    assert_eq!(read(&f.local, NOTE), merged);
    sync::resolve_item(&f.local, &f.folder, &item.id, 0, None).unwrap();
    let resolved = read(&f.local, NOTE);
    assert!(resolved.contains("팝업 없이 로그만 남깁니다.") && !resolved.contains("오류 팝업을 표시합니다."));
    assert_eq!(git(&f.local, &["status", "--porcelain"]), "");
    assert!(sync::state(&f.folder).items.iter().all(|other| other.id != item.id));
    assert!(run(&f).sent);
    git(&f.other, &["pull", "--ff-only"]);
    assert_eq!(read(&f.other, NOTE), resolved);
}

#[test]
fn evidence_quoted_from_the_merged_note_is_accepted() {
    let f = fixture();
    contradict(&f);
    run(&f);
    let found = compare(&f, &mut |_| reply(decision(&format!("merged:{NOTE}"), "팝업 없이 로그만 남깁니다."))).unwrap();
    assert_eq!(found, 1);
}

#[test]
fn a_rejected_model_reply_is_sent_back_with_the_reason_instead_of_stopping_the_comparison() {
    let f = fixture();
    contradict(&f);
    run(&f);
    let mut prompts = Vec::new();
    let id = sync::pending_comparison(&f.folder, LATER).unwrap();
    let mut calls = 0;
    let found = sync::compare(&f.folder, &id, &mut |prompt, _, _| {
        calls += 1;
        prompts.push(prompt.to_string());
        Ok(if calls == 1 { reply(decision(&format!("remote:{NOTE}"), "원문에 없는 문장")) } else { reply(decision(&format!("remote:{NOTE}"), "오류 팝업을 표시합니다.")) })
    }, &mut || {})
    .unwrap();
    assert_eq!(found, 1);
    assert_eq!(calls, 2);
    assert!(prompts[1].contains("원문에 없는 문장"));
}

#[test]
fn a_read_request_the_program_cannot_serve_is_sent_back_instead_of_stopping_the_comparison() {
    let f = fixture();
    contradict(&f);
    run(&f);
    let mut prompts = Vec::new();
    let id = sync::pending_comparison(&f.folder, LATER).unwrap();
    let mut calls = 0;
    let found = sync::compare(&f.folder, &id, &mut |prompt, _, _| {
        calls += 1;
        prompts.push(prompt.to_string());
        Ok(if calls == 1 {
            json!({"summary":"Reading.","issues":[],"notes":"","reads":[{"source":format!("local:{NOTE}"),"offset":0,"length":200,"anchor":"문서에 없는 문구"}],"replace_issues":[]})
        } else {
            reply(decision(&format!("remote:{NOTE}"), "오류 팝업을 표시합니다."))
        })
    }, &mut || {})
    .unwrap();
    assert_eq!(found, 1);
    assert_eq!(calls, 2);
    assert!(prompts[1].contains("문서에 없는 문구"));
    assert!(sync::state(&f.folder).comparisons.is_empty());
}

#[test]
fn a_comparison_that_keeps_failing_is_reported_without_touching_sync() {
    let f = fixture();
    contradict(&f);
    run(&f);
    let error = compare(&f, &mut |_| reply(decision(&format!("remote:{NOTE}"), "원문에 없는 문장"))).unwrap_err();
    assert!(error.to_string().contains("원문에 없는 문장"));
    let state = sync::state(&f.folder);
    assert!(state.comparisons[0].error.is_some());
    assert!(sync::pending_comparison(&f.folder, LATER).is_none());
    assert!(!sync::summary(&f.folder).comparing);
    write(&f.local, OTHER_NOTE, "# 계획\n");
    assert!(run(&f).sent);
    sync::retry_comparison(&f.folder).unwrap();
    assert!(sync::pending_comparison(&f.folder, LATER).is_some());
}

#[test]
fn a_comparison_whose_model_could_not_run_waits_and_then_runs_again_by_itself() {
    let f = fixture();
    contradict(&f);
    run(&f);
    let id = sync::pending_comparison(&f.folder, LATER).unwrap();
    let before = chrono::Utc::now().timestamp();
    let error = sync::compare(&f.folder, &id, &mut |_, _, _| Err(lore_core::error::Error::transient("usage limit reached")), &mut || {}).unwrap_err();
    assert_eq!(error.to_string(), "usage limit reached");
    let waiting = sync::state(&f.folder).comparisons.remove(0);
    assert_eq!((waiting.error, waiting.attempts, waiting.last_failure.as_deref()), (None, 1, Some("usage limit reached")));
    // The first wait is one minute, as for a recording.
    assert!(waiting.next_attempt_at >= before + 60);
    assert!(sync::pending_comparison(&f.folder, waiting.next_attempt_at - 1).is_none());
    assert_eq!(sync::pending_comparison(&f.folder, waiting.next_attempt_at).as_deref(), Some(id.as_str()));
    assert!(sync::summary(&f.folder).comparing);

    let found = compare(&f, &mut |_| reply(decision(&format!("remote:{NOTE}"), "오류 팝업을 표시합니다."))).unwrap();
    assert_eq!(found, 1);
    assert!(sync::state(&f.folder).comparisons.is_empty());
}

#[test]
fn a_comparison_stops_for_the_user_after_the_last_automatic_try() {
    let f = fixture();
    contradict(&f);
    run(&f);
    let id = sync::pending_comparison(&f.folder, LATER).unwrap();
    for attempt in 1..=lore_core::queue::MAX_TRANSIENT_ATTEMPTS {
        assert_eq!(sync::pending_comparison(&f.folder, LATER).as_deref(), Some(id.as_str()), "attempt {attempt}");
        sync::compare(&f.folder, &id, &mut |_, _, _| Err(lore_core::error::Error::transient("usage limit reached")), &mut || {}).unwrap_err();
    }
    let stopped = sync::state(&f.folder).comparisons.remove(0);
    assert_eq!(stopped.error.as_deref(), Some("usage limit reached"));
    assert!(sync::pending_comparison(&f.folder, LATER).is_none());
    sync::retry_comparison(&f.folder).unwrap();
    let again = sync::state(&f.folder).comparisons.remove(0);
    assert_eq!((again.error, again.attempts, again.next_attempt_at, again.last_failure), (None, 0, 0, None));
}

#[test]
fn a_decision_is_refused_when_the_note_changed_after_the_comparison() {
    let f = fixture();
    contradict(&f);
    run(&f);
    compare(&f, &mut |_| reply(decision(&format!("remote:{NOTE}"), "오류 팝업을 표시합니다."))).unwrap();
    let item = sync::state(&f.folder).items.into_iter().find(|item| item.kind == "decision").unwrap();
    write(&f.local, NOTE, "# 오류 알림\n\n다시 썼습니다.\n");
    assert!(sync::resolve_item(&f.local, &f.folder, &item.id, 0, None).is_err());
    assert_eq!(read(&f.local, NOTE), "# 오류 알림\n\n다시 썼습니다.\n");
    assert!(sync::resolve_item(&f.local, &f.folder, &item.id, 9, None).is_err());
}

#[test]
fn recorded_notes_wait_for_review_until_marked_and_received_notes_do_not() {
    let f = fixture();
    review::ensure_reviewed(&f.local).unwrap();
    write(&f.local, OTHER_NOTE, "# 계획\n");
    write(&f.local, "projects/demo/extra.md", "# 추가\n");
    write(&f.other, "projects/third/note.md", "# 다른 PC\n");
    commit(&f.other);
    git(&f.other, &["push"]);
    run(&f);
    let waiting = |f: &Fixture| -> Vec<String> {
        review::working_changes(&f.local).unwrap().unwrap().notes.into_iter().map(|change| change.path).collect()
    };
    assert_eq!(waiting(&f), vec!["projects/demo/extra.md".to_string(), OTHER_NOTE.to_string()]);
    assert_eq!(review::file_diff(&f.local, OTHER_NOTE).unwrap().len(), 1);
    review::mark_reviewed(&f.local, &[OTHER_NOTE.to_string()]).unwrap();
    assert_eq!(waiting(&f), vec!["projects/demo/extra.md".to_string()]);
    write(&f.local, OTHER_NOTE, "# 계획\n\n다시 기록했습니다.\n");
    assert_eq!(waiting(&f), vec!["projects/demo/extra.md".to_string(), OTHER_NOTE.to_string()]);
    assert_eq!(review::file_diff(&f.local, OTHER_NOTE).unwrap().len(), 3);
    review::mark_reviewed(&f.local, &["projects/demo/extra.md".to_string(), OTHER_NOTE.to_string()]).unwrap();
    assert!(waiting(&f).is_empty());
    assert!(review::mark_reviewed(&f.local, &[NOTE.to_string()]).is_err());
}

#[test]
fn synchronization_schema_uses_strict_objects_at_every_level() {
    fn check(value: &serde_json::Value) {
        if let Some(object) = value.as_object() {
            if object.get("type").and_then(|v| v.as_str()) == Some("object") {
                assert_eq!(object.get("additionalProperties"), Some(&serde_json::Value::Bool(false)));
                let properties = object["properties"].as_object().unwrap();
                let required: Vec<&str> = object["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
                assert_eq!(required.len(), properties.len());
                for property in properties.values() {
                    check(property);
                }
            }
            if let Some(items) = object.get("items") {
                check(items);
            }
        }
    }
    check(&sync::analysis_schema());
}
