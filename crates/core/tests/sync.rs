use lore_core::sync::{self, Analysis, Choice, Edit, Evidence, Issue, OptionPlan};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fixture {
    _dir: tempfile::TempDir,
    local: PathBuf,
    other: PathBuf,
    journal: PathBuf,
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
fn commit(root: &Path) {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "fixture"]);
}
const NOTE: &str = "projects/demo/policy.md";
const BASE: &str =
    "# 오류 알림\n\n## 결정\n\n오류 안내는 미정입니다.\n\n## 메모\n\n이 문장은 유지합니다.\n";
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
        journal: root.join("app/pending.json"),
        _dir: dir,
    }
}
fn diverge(f: &Fixture) {
    write(
        &f.local,
        NOTE,
        &BASE.replace("오류 안내는 미정입니다.", "팝업 없이 로그만 남깁니다."),
    );
    commit(&f.local);
    write(
        &f.other,
        NOTE,
        &BASE.replace("오류 안내는 미정입니다.", "오류 팝업을 표시합니다."),
    );
    commit(&f.other);
    git(&f.other, &["push"]);
}
fn proposed(plan: &sync::Plan) -> Analysis {
    let merged = plan
        .files
        .iter()
        .find(|f| f.path == NOTE)
        .unwrap()
        .merged
        .clone()
        .unwrap();
    let options = [
        ("로그만", "팝업 없이 로그만 남깁니다."),
        ("팝업", "오류 팝업을 표시합니다."),
    ]
    .into_iter()
    .map(|(label, text)| OptionPlan {
        label: label.into(),
        description: text.into(),
        edits: vec![Edit {
            path: NOTE.into(),
            find: merged.clone(),
            text: BASE.replace("오류 안내는 미정입니다.", text),
        }],
    })
    .collect();
    Analysis {
        summary: "오류 알림 방침이 다릅니다.".into(),
        issues: vec![Issue {
            title: "오류 알림".into(),
            explanation: "양쪽 방침을 확인하십시오.".into(),
            evidence: vec![
                Evidence {
                    source: format!("local:{NOTE}"),
                    quote: "팝업 없이 로그만 남깁니다.".into(),
                },
                Evidence {
                    source: format!("remote:{NOTE}"),
                    quote: "오류 팝업을 표시합니다.".into(),
                },
            ],
            options,
        }],
    }
}

#[test]
fn conflicting_plan_is_isolated_and_a_choice_merges_both_histories() {
    let f = fixture();
    diverge(&f);
    let before = git(&f.local, &["rev-parse", "HEAD"]);
    let content = std::fs::read_to_string(f.local.join(NOTE)).unwrap();
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    assert!(plan.files[0].text_conflict);
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(f.local.join(NOTE)).unwrap(),
        content
    );
    assert_eq!(git(&f.local, &["status", "--porcelain"]), "");
    assert!(sync::preview(&plan, &[]).is_err());
    plan.analysis = Some(proposed(&plan));
    sync::save(&f.journal, &plan).unwrap();
    let choices = [Choice {
        issue: 0,
        option: 0,
        custom_text: None,
    }];
    let preview = sync::preview(&plan, &choices).unwrap();
    assert!(preview[NOTE].contains("이 문장은 유지합니다."));
    let approved = sync::approve(&f.local, &f.journal, &plan.id, &choices).unwrap();
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), before);
    assert!(sync::finish(&f.local, &f.journal, || Ok(()))
        .unwrap()
        .is_none());
    assert_eq!(
        git(&f.local, &["rev-parse", "HEAD"]),
        approved.approved.unwrap()
    );
    assert!(
        git(&f.local, &["show", "-s", "--format=%P"])
            .split_whitespace()
            .count()
            == 2
    );
    assert_eq!(
        std::fs::read_to_string(f.local.join(NOTE)).unwrap(),
        preview[NOTE]
    );
    git(&f.other, &["pull", "--ff-only"]);
    assert_eq!(
        std::fs::read_to_string(f.other.join(NOTE)).unwrap(),
        preview[NOTE]
    );
}

#[test]
fn semantic_overlap_is_reviewed_even_when_git_reports_a_clean_merge() {
    let f = fixture();
    let base = format!(
        "{BASE}{}\n현장 방침은 미정입니다.\n",
        "\n중간 문맥\n".repeat(12)
    );
    write(&f.local, NOTE, &base);
    commit(&f.local);
    git(&f.local, &["push"]);
    git(&f.other, &["pull", "--ff-only"]);
    write(
        &f.local,
        NOTE,
        &base.replace("오류 안내는 미정입니다.", "팝업 없이 로그만 남깁니다."),
    );
    commit(&f.local);
    write(
        &f.other,
        NOTE,
        &base.replace("현장 방침은 미정입니다.", "오류 팝업을 항상 표시합니다."),
    );
    commit(&f.other);
    git(&f.other, &["push"]);
    let plan = sync::prepare(&f.local, &f.journal).unwrap();
    assert!(!plan.files[0].text_conflict);
    assert!(plan.analysis.is_none());
    assert_eq!(plan.phase, "review");
    assert!(sync::preview(&plan, &[]).is_err());
}

#[test]
fn different_nested_documents_in_one_project_still_need_model_comparison() {
    let f = fixture();
    write(
        &f.local,
        "projects/demo/operations/alerts.md",
        "팝업 없이 로그만 남깁니다.\n",
    );
    commit(&f.local);
    write(
        &f.other,
        "projects/demo/decisions/alerts.md",
        "오류 팝업을 항상 표시합니다.\n",
    );
    commit(&f.other);
    git(&f.other, &["push"]);
    let plan = sync::prepare(&f.local, &f.journal).unwrap();
    assert_eq!(plan.files.len(), 2);
    assert!(plan.files.iter().all(|file| !file.text_conflict));
    assert!(plan.analysis.is_none());
    assert!(sync::preview(&plan, &[]).is_err());
}

#[test]
fn tracking_names_the_branch_and_its_upstream() {
    let f = fixture();
    assert_eq!(sync::tracking(&f.local).as_deref(), Some("main ↔ origin/main"));
    git(&f.local, &["branch", "--unset-upstream"]);
    assert_eq!(sync::tracking(&f.local), None);
}

#[test]
fn fast_forward_reindexes_and_resumes_index_failure_without_reapplying() {
    let f = fixture();
    write(
        &f.other,
        NOTE,
        &BASE.replace("미정입니다", "로그만 남깁니다"),
    );
    commit(&f.other);
    git(&f.other, &["push"]);
    let plan = sync::prepare(&f.local, &f.journal).unwrap();
    assert_eq!(plan.phase, "applying");
    let pending = sync::finish(&f.local, &f.journal, || {
        Err(lore_core::error::Error::transient(
            "index fixture failed",
        ))
    })
    .unwrap()
    .unwrap();
    assert_eq!(pending.phase, "index_pending");
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), plan.remote);
    assert_eq!(sync::summary(&f.journal).last_synced_at, None);
    let before = chrono::Utc::now().timestamp();
    assert!(sync::finish(&f.local, &f.journal, || Ok(()))
        .unwrap()
        .is_none());
    let after = chrono::Utc::now().timestamp();
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), plan.remote);
    let summary = sync::summary(&f.journal);
    assert_eq!(summary.phase, "idle");
    assert!(summary.last_synced_at.is_some_and(|at| (before..=after).contains(&at)));
}

#[test]
fn dirty_worktrees_stale_heads_and_fabricated_evidence_are_refused() {
    let f = fixture();
    diverge(&f);
    write(&f.local, "common/unreviewed.md", "not reviewed");
    assert!(sync::prepare(&f.local, &f.journal).is_err());
    std::fs::remove_file(f.local.join("common/unreviewed.md")).unwrap();
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    let mut analysis = proposed(&plan);
    analysis.issues[0].evidence[0].quote = "invented quote".into();
    assert!(sync::validate_analysis(&plan, &analysis).is_err());
    plan.analysis = Some(proposed(&plan));
    sync::save(&f.journal, &plan).unwrap();
    write(&f.other, "common/new.md", "new remote change");
    commit(&f.other);
    git(&f.other, &["push"]);
    let before = git(&f.local, &["rev-parse", "HEAD"]);
    assert!(sync::approve(
        &f.local,
        &f.journal,
        &plan.id,
        &[Choice {
            issue: 0,
            option: 0,
            custom_text: None
        }]
    )
    .is_err());
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), before);
}

#[test]
fn push_race_keeps_the_approved_local_commit() {
    let f = fixture();
    diverge(&f);
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    plan.analysis = Some(proposed(&plan));
    sync::save(&f.journal, &plan).unwrap();
    let approved = sync::approve(
        &f.local,
        &f.journal,
        &plan.id,
        &[Choice {
            issue: 0,
            option: 0,
            custom_text: None,
        }],
    )
    .unwrap()
    .approved
    .unwrap();
    write(&f.other, "common/new.md", "race");
    commit(&f.other);
    git(&f.other, &["push"]);
    let pending = sync::finish(&f.local, &f.journal, || Ok(()))
        .unwrap()
        .unwrap();
    assert_eq!(pending.phase, "push_pending");
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), approved);
    assert!(!pending.blocks_recording());
    assert_eq!(sync::summary(&f.journal).last_synced_at, None);
}

#[test]
fn a_push_retry_does_not_commit_dirty_unreviewed_notes() {
    let f = fixture();
    diverge(&f);
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    plan.analysis = Some(proposed(&plan));
    sync::save(&f.journal, &plan).unwrap();
    let approved = sync::approve(
        &f.local,
        &f.journal,
        &plan.id,
        &[Choice {
            issue: 0,
            option: 0,
            custom_text: None,
        }],
    )
    .unwrap()
    .approved
    .unwrap();
    // First stop after indexing, simulating loss of network for the push.
    let mut applying = sync::load(&f.journal).unwrap().unwrap();
    git(&f.local, &["merge", "--ff-only", &approved]);
    applying.phase = "push_pending".into();
    sync::save(&f.journal, &applying).unwrap();
    write(&f.local, "common/unreviewed.md", "이 내용은 공유하지 않음");
    assert!(
        sync::finish(&f.local, &f.journal, || panic!("already indexed"))
            .unwrap()
            .is_none()
    );
    assert_eq!(git(&f.local, &["rev-parse", "HEAD"]), approved);
    git(&f.other, &["pull", "--ff-only"]);
    assert!(!f.other.join("common/unreviewed.md").exists());
    assert!(f.local.join("common/unreviewed.md").exists());
}

#[test]
fn missing_anchors_and_removing_existing_evidence_are_rejected() {
    let f = fixture();
    let link = "[L1](<../../sources/claude/session.md#L1>)";
    let with_link = format!("{BASE}\n## 원문 근거\n{link}\n");
    write(&f.local, NOTE, &with_link);
    write(
        &f.local,
        "sources/claude/session.md",
        "## L1 · 사용자\n근거만 있고 앵커는 없음\n",
    );
    commit(&f.local);
    git(&f.local, &["push"]);
    git(&f.other, &["pull", "--ff-only"]);
    write(
        &f.local,
        NOTE,
        &with_link.replace("오류 안내는 미정입니다.", "팝업 없이 로그만 남깁니다."),
    );
    commit(&f.local);
    write(
        &f.other,
        NOTE,
        &with_link.replace("오류 안내는 미정입니다.", "오류 팝업을 표시합니다."),
    );
    commit(&f.other);
    git(&f.other, &["push"]);
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    assert!(plan
        .sources
        .iter()
        .any(|s| s.id == "local:sources/claude/session.md"));
    let mut analysis = proposed(&plan);
    plan.analysis = Some(analysis.clone());
    let choices = [Choice {
        issue: 0,
        option: 0,
        custom_text: None,
    }];
    assert!(
        sync::preview(&plan, &choices).is_err(),
        "discarded evidence must not pass"
    );
    for option in &mut analysis.issues[0].options {
        option.edits[0]
            .text
            .push_str(&format!("\n## 원문 근거\n{link}\n"));
    }
    plan.analysis = Some(analysis);
    sync::save(&f.journal, &plan).unwrap();
    assert!(sync::preview(&plan, &choices).is_ok());
    assert!(sync::approve(&f.local, &f.journal, &plan.id, &choices)
        .unwrap_err()
        .message
        .contains("anchor"));
}

#[test]
fn duplicate_choices_bad_spans_and_custom_empty_text_are_rejected() {
    let f = fixture();
    diverge(&f);
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    let mut analysis = proposed(&plan);
    analysis.issues[0].options[0].edits[0].find = "not in document".into();
    assert!(sync::validate_analysis(&plan, &analysis).is_err());
    plan.analysis = Some(proposed(&plan));
    assert!(sync::preview(
        &plan,
        &[Choice {
            issue: 0,
            option: 0,
            custom_text: Some(" ".into())
        }]
    )
    .is_err());
    assert!(sync::preview(
        &plan,
        &[
            Choice {
                issue: 0,
                option: 0,
                custom_text: None
            },
            Choice {
                issue: 0,
                option: 1,
                custom_text: None
            }
        ]
    )
    .is_err());
    let custom = BASE.replace("오류 안내는 미정입니다.", "운영은 로그만, 점검은 팝업.");
    assert_eq!(
        sync::preview(
            &plan,
            &[Choice {
                issue: 0,
                option: 0,
                custom_text: Some(custom.clone())
            }]
        )
        .unwrap()[NOTE],
        custom
    );
}

#[test]
fn non_note_conflicts_are_not_silently_resolved_by_the_model() {
    let f = fixture();
    write(&f.local, "sources/claude/s.md", "base");
    commit(&f.local);
    git(&f.local, &["push"]);
    git(&f.other, &["pull", "--ff-only"]);
    write(&f.local, "sources/claude/s.md", "local");
    commit(&f.local);
    write(&f.other, "sources/claude/s.md", "remote");
    commit(&f.other);
    git(&f.other, &["push"]);
    let mut plan = sync::prepare(&f.local, &f.journal).unwrap();
    assert_eq!(plan.unsupported, vec!["sources/claude/s.md"]);
    plan.analysis = Some(Analysis {
        summary: "not safe".into(),
        issues: vec![],
    });
    assert!(sync::preview(&plan, &[]).is_err());
}

#[test]
fn synchronization_schema_uses_strict_objects_at_every_level() {
    fn check(value: &serde_json::Value) {
        if value["type"] == "object" {
            assert_eq!(value["additionalProperties"], false);
            let properties = value["properties"].as_object().unwrap();
            let required = value["required"].as_array().unwrap();
            assert_eq!(properties.len(), required.len());
            for (key, child) in properties {
                assert!(required.iter().any(|item| item.as_str() == Some(key)));
                check(child);
            }
        }
        if let Some(items) = value.get("items") {
            check(items);
        }
    }
    check(&sync::analysis_schema());
}
