//! Automatic Git synchronization of the memory folder. A sync commits what was
//! recorded, merges the other PCs' commits and pushes, without asking. Notes
//! where both sides had to be combined are listed for the user to check later;
//! nothing here waits for that check.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::review::{self, is_note};
use crate::run;
use crate::settings::{atomic_write, which};
use crate::transcript::digest;

const LIMIT: Duration = Duration::from_secs(90);
/// Consecutive failed syncs before the user is told. Earlier failures retry quietly.
pub const FAILURES_SHOWN: usize = 3;
mod comparison;
pub use comparison::Progress as ComparisonProgress;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub path: String,
    pub base: Option<String>,
    pub local: Option<String>,
    pub remote: Option<String>,
    pub merged: Option<String>,
    pub text_conflict: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub side: String,
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: String,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub path: String,
    pub find: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionPlan {
    pub label: String,
    pub description: String,
    pub edits: Vec<Edit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub title: String,
    pub explanation: String,
    pub evidence: Vec<Evidence>,
    pub options: Vec<OptionPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    pub summary: String,
    pub issues: Vec<Issue>,
}

/// The notes both PCs changed in one merge, kept so they can be shown and
/// compared after the merge has already been shared.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub local: String,
    pub remote: String,
    pub base: String,
    pub tree: String,
    pub files: Vec<File>,
    pub sources: Vec<Source>,
    pub analysis: Option<Analysis>,
}

/// Something for the user to look at when they want to. `merged`: both PCs'
/// lines were kept. `kept`: one PC deleted the note, the edited one was kept.
/// `decision`: the model found statements that disagree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub plan: String,
    pub path: String,
    pub title: String,
    pub detail: String,
    pub created_at: i64,
    pub issue: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub plan: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub last_synced_at: Option<i64>,
    pub failures: usize,
    pub message: Option<String>,
    pub index_pending: bool,
    pub items: Vec<Item>,
    pub comparisons: Vec<Comparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// `idle`, `retrying` (failed, will try again quietly) or `failing`.
    pub phase: String,
    pub message: Option<String>,
    pub items: usize,
    pub comparing: bool,
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// Notes that came from other PCs.
    pub received: usize,
    pub sent: bool,
    /// Notes newly listed for the user to check.
    pub items: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    pub item: Item,
    pub files: Vec<File>,
    pub issue: Option<Issue>,
    pub sources: Vec<Source>,
}

pub fn folder(paths: &crate::settings::Paths, root: &Path) -> PathBuf {
    let key = root.to_string_lossy();
    paths.root.join("sync").join(&digest(key.as_bytes())[..16])
}

static STATE: Mutex<()> = Mutex::new(());

fn read_state(folder: &Path) -> State {
    if let Some(state) = std::fs::read(folder.join("state.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()) {
        return state;
    }
    // Versions before automatic sync kept only the time of the last sync.
    let last_synced_at = std::fs::read(folder.join("last-sync.json")).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).and_then(|value| value.get("at")?.as_i64());
    State { last_synced_at, ..State::default() }
}

pub fn state(folder: &Path) -> State {
    let _guard = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    read_state(folder)
}

/// Sync and model comparison run on different threads; every change to the
/// stored state is one short read-modify-write under this lock.
fn update<T>(folder: &Path, change: impl FnOnce(&mut State) -> T) -> Result<T> {
    let _guard = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut state = read_state(folder);
    let result = change(&mut state);
    atomic_write(&folder.join("state.json"), &serde_json::to_vec_pretty(&state)?)?;
    Ok(result)
}

pub fn summary(folder: &Path) -> Summary {
    let state = state(folder);
    Summary {
        phase: match state.failures {
            0 => "idle",
            count if count < FAILURES_SHOWN => "retrying",
            _ => "failing",
        }
        .into(),
        message: state.message,
        items: state.items.len(),
        comparing: state.comparisons.iter().any(|comparison| comparison.error.is_none()),
        last_synced_at: state.last_synced_at,
    }
}

fn git_env(root: &Path, args: &[&str], input: Option<&str>, env: &[(&str, String)], accepted: &[i32]) -> Result<run::Output> {
    let program = which("git").ok_or_else(|| Error::permanent("Git 2.38 or later is required."))?;
    let mut command = vec![program.to_string_lossy().to_string(), "-c".into(), "core.quotepath=false".into()];
    command.extend(args.iter().map(|arg| arg.to_string()));
    let mut variables = vec![("GIT_TERMINAL_PROMPT", "0".into()), ("GCM_INTERACTIVE", "Never".into())];
    variables.extend_from_slice(env);
    run::run_allow(&command, root, &variables, input, &|| LIMIT, accepted)
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    Ok(git_env(root, args, None, &[], &[0])?.stdout)
}
fn head(root: &Path) -> Result<String> {
    Ok(git(root, &["rev-parse", "HEAD"])?.trim().to_string())
}
fn branch(root: &Path) -> Result<String> {
    Ok(git(root, &["symbolic-ref", "--short", "HEAD"])?.trim().to_string())
}

pub fn tracking(root: &Path) -> Option<String> {
    let branch = branch(root).ok()?;
    let remote = git(root, &["config", "--get", &format!("branch.{branch}.remote")]).ok()?;
    let merge = git(root, &["config", "--get", &format!("branch.{branch}.merge")]).ok()?;
    Some(format!("{branch} ↔ {}/{}", remote.trim(), merge.trim().trim_start_matches("refs/heads/")))
}
fn ancestor(root: &Path, a: &str, b: &str) -> Result<bool> {
    Ok(git_env(root, &["merge-base", "--is-ancestor", a, b], None, &[], &[0, 1])?.code == Some(0))
}
fn tracked(root: &Path, revision: &str) -> Result<BTreeSet<String>> {
    Ok(git(root, &["ls-tree", "-r", "--name-only", "-z", revision])?.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect())
}
fn changed(root: &Path, base: &str, revision: &str) -> Result<BTreeSet<String>> {
    Ok(git(root, &["diff", "--name-only", "--no-renames", "-z", base, revision])?.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect())
}
fn text(value: String) -> Option<String> {
    Some(value).filter(|text| !text.contains('\0') && !text.contains('\u{fffd}'))
}
/// The file at a revision, when it exists there as a regular UTF-8 text file.
fn blob(root: &Path, revision: &str, path: &str, exists: &BTreeSet<String>) -> Option<String> {
    if !exists.contains(path) {
        return None;
    }
    let entry = git(root, &["ls-tree", revision, "--", path]).ok()?;
    if !entry.starts_with("100644 ") && !entry.starts_with("100755 ") {
        return None;
    }
    text(git(root, &["show", &format!("{revision}:{path}")]).ok()?)
}
fn scope(path: &str) -> Option<String> {
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        ["common", _, ..] => Some("common".into()),
        ["projects", name, _, ..] => Some(format!("projects/{name}")),
        _ => None,
    }
}
fn fetch(root: &Path, remote: &str, reference: &str) -> Result<String> {
    git(root, &["fetch", "--no-tags", "--", remote, reference])?;
    Ok(git(root, &["rev-parse", "FETCH_HEAD"])?.trim().to_string())
}
fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}

/// A review an older version left waiting must not hold anything now. Its
/// inputs are kept; both tips are still in Git and are merged like any others.
fn set_aside_legacy(folder: &Path) -> Result<()> {
    let pending = folder.join("pending.json");
    if pending.exists() {
        std::fs::rename(&pending, folder.join(format!("superseded-{}.json", chrono::Utc::now().timestamp_micros())))?;
    }
    Ok(())
}

/// Commit, fetch, merge and push. Never asks: a clean merge is applied as is,
/// and where both PCs changed the same lines both are kept and the note is
/// listed. `index` refreshes search after notes arrive from another PC.
pub fn sync(root: &Path, folder: &Path, index: impl FnMut() -> Result<()>) -> Result<Outcome> {
    std::fs::create_dir_all(folder)?;
    match exchange(root, folder, index) {
        Ok(outcome) => {
            update(folder, |state| {
                state.failures = 0;
                state.message = None;
                state.last_synced_at = Some(chrono::Utc::now().timestamp());
            })?;
            Ok(outcome)
        }
        Err(error) => {
            update(folder, |state| {
                state.failures += 1;
                state.message = Some(error.to_string());
            })?;
            Err(error)
        }
    }
}

fn exchange(root: &Path, folder: &Path, mut index: impl FnMut() -> Result<()>) -> Result<Outcome> {
    if !review::is_repository(root) {
        return Err(Error::permanent("The memory folder is not a Git repository."));
    }
    set_aside_legacy(folder)?;
    if review::in_progress(root)? {
        return Err(Error::permanent("Finish the Git merge or rebase in the memory folder first."));
    }
    review::record_commit(root)?;
    let branch = branch(root)?;
    let remote_name = git(root, &["config", "--get", &format!("branch.{branch}.remote")])
        .map_err(|_| Error::permanent("No remote upstream is configured. Set an upstream for the memory repository’s current branch first."))?
        .trim()
        .to_string();
    let remote_ref = git(root, &["config", "--get", &format!("branch.{branch}.merge")])?.trim().to_string();
    if remote_name == "." || remote_name.starts_with('-') || !remote_ref.starts_with("refs/heads/") {
        return Err(Error::permanent("An upstream pointing to a remote branch is required."));
    }
    let mut outcome = Outcome::default();
    let mut incoming = false;
    let mut failed = None;
    for attempt in 0..3 {
        let local = head(root)?;
        let remote = fetch(root, &remote_name, &remote_ref)?;
        if local == remote {
            break;
        }
        if ancestor(root, &local, &remote)? {
            git(root, &["merge", "--ff-only", "--", &remote])?;
            outcome.received += changed(root, &local, &remote)?.iter().filter(|path| is_note(path)).count();
            review::absorb(root, &local, &remote)?;
            incoming = true;
            break;
        }
        if !ancestor(root, &remote, &local)? {
            let (commit, items) = merge(root, folder, &local, &remote)?;
            git(root, &["merge", "--ff-only", "--", &commit])?;
            outcome.received += changed(root, &local, &commit)?.iter().filter(|path| is_note(path)).count();
            outcome.items += items;
            review::absorb(root, &local, &commit)?;
            incoming = true;
        }
        match git(root, &["push", "--porcelain", "--", &remote_name, &format!("{}:{remote_ref}", head(root)?)]) {
            Ok(_) => {
                outcome.sent = true;
                break;
            }
            // Another PC shared first: bring its commits in and send again.
            Err(_) if attempt < 2 && fetch(root, &remote_name, &remote_ref).is_ok_and(|now| now != remote) => continue,
            Err(error) => {
                failed = Some(Error::transient(format!("Notes are saved on this PC but could not be sent: {error}")));
                break;
            }
        }
    }
    if incoming || state(folder).index_pending {
        let refreshed = index();
        update(folder, |state| state.index_pending = refreshed.is_err())?;
        if let (Err(error), None) = (refreshed, &failed) {
            failed = Some(Error::transient(format!("Notes are synced, but search was not updated: {error}")));
        }
    }
    match failed {
        Some(error) => Err(error),
        None => Ok(outcome),
    }
}

struct Stage {
    mode: String,
    oid: String,
}

/// Merge two diverged histories into one commit without touching the worktree.
/// Returns the merge commit and how many notes were listed for a later check.
fn merge(root: &Path, folder: &Path, local: &str, remote: &str) -> Result<(String, usize)> {
    let bases = git(root, &["merge-base", "--all", local, remote])?;
    let bases: Vec<_> = bases.lines().collect();
    if bases.len() != 1 {
        return Err(Error::permanent("The memory histories have no single common base. Merge them with a Git tool once, then sync again."));
    }
    let base = bases[0].to_string();
    // Renames are treated as delete plus add, so every conflict is about one path.
    let output = git_env(root, &["-c", "merge.renames=false", "merge-tree", "--write-tree", "-z", local, remote], None, &[], &[0, 1])?;
    let mut fields = output.stdout.split('\0');
    let tree = fields.next().unwrap_or("").trim().to_string();
    if !valid_oid(&tree) {
        return Err(Error::permanent("Could not read the Git merge preview. Check that Git 2.38 or later is installed."));
    }
    let mut conflicts: BTreeMap<String, BTreeMap<u8, Stage>> = BTreeMap::new();
    if output.code == Some(1) {
        for entry in fields.take_while(|field| !field.is_empty()) {
            let Some((meta, path)) = entry.split_once('\t') else { continue };
            let parts: Vec<_> = meta.split(' ').collect();
            if let [mode, oid, stage] = parts.as_slice() {
                conflicts.entry(path.to_string()).or_default().insert(stage.parse().unwrap_or(0), Stage { mode: mode.to_string(), oid: oid.to_string() });
            }
        }
    }
    let mut resolved: Vec<(String, String, String)> = Vec::new();
    let mut kinds: BTreeMap<String, &str> = BTreeMap::new();
    for (path, stages) in &conflicts {
        match (stages.get(&2), stages.get(&3)) {
            (Some(ours), Some(theirs)) => {
                let read = |stage: Option<&Stage>| -> Result<Option<String>> {
                    Ok(match stage {
                        Some(stage) => text(git(root, &["cat-file", "blob", &stage.oid])?),
                        None => Some(String::new()),
                    })
                };
                match (read(Some(ours))?, read(stages.get(&1))?, read(Some(theirs))?) {
                    (Some(ours_text), Some(base_text), Some(theirs_text)) => {
                        let merged = union(root, folder, &ours_text, &base_text, &theirs_text)?;
                        let oid = git_env(root, &["hash-object", "-w", "--stdin"], Some(&merged), &[], &[0])?.stdout.trim().to_string();
                        resolved.push((path.clone(), ours.mode.clone(), oid));
                        kinds.insert(path.clone(), "merged");
                    }
                    // Not text: lines cannot be combined, so this PC's file stays.
                    _ => {
                        resolved.push((path.clone(), ours.mode.clone(), ours.oid.clone()));
                        kinds.insert(path.clone(), "kept");
                    }
                }
            }
            (Some(only), None) | (None, Some(only)) => {
                resolved.push((path.clone(), only.mode.clone(), only.oid.clone()));
                kinds.insert(path.clone(), "kept");
            }
            (None, None) => {}
        }
    }
    let final_tree = if resolved.is_empty() {
        tree
    } else {
        let index = folder.join("merge.index");
        let env = [("GIT_INDEX_FILE", index.to_string_lossy().to_string())];
        let built = (|| {
            // read-tree replaces our private index. It never uses the user's staging area.
            git_env(root, &["read-tree", &tree], None, &env, &[0])?;
            for (path, mode, oid) in &resolved {
                git_env(root, &["update-index", "--add", "--cacheinfo", &format!("{mode},{oid},{path}")], None, &env, &[0])?;
            }
            Ok::<_, Error>(git_env(root, &["write-tree"], None, &env, &[0])?.stdout.trim().to_string())
        })();
        let _ = std::fs::remove_file(index);
        built?
    };
    let commit = git_env(root, &["commit-tree", &final_tree, "-p", local, "-p", remote, "-F", "-"], Some("Merge memory from another PC\n"), &[], &[0])?.stdout.trim().to_string();

    let left = changed(root, &base, local)?;
    let right = changed(root, &base, remote)?;
    let both: Vec<&String> = left.intersection(&right).filter(|path| is_note(path)).collect();
    if both.is_empty() {
        return Ok((commit, 0));
    }
    let (base_files, local_files, remote_files, merged_files) = (tracked(root, &base)?, tracked(root, local)?, tracked(root, remote)?, tracked(root, &commit)?);
    let mut plan = Plan { id: digest(format!("{local}:{remote}").as_bytes())[..16].into(), local: local.into(), remote: remote.into(), base: base.clone(), tree: final_tree, files: Vec::new(), sources: Vec::new(), analysis: None };
    for path in both {
        plan.files.push(File {
            path: path.clone(),
            base: blob(root, &base, path, &base_files),
            local: blob(root, local, path, &local_files),
            remote: blob(root, remote, path, &remote_files),
            merged: blob(root, &commit, path, &merged_files),
            text_conflict: kinds.get(path) == Some(&"merged"),
        });
    }
    for file in &plan.files {
        for (side, text) in [("base", &file.base), ("local", &file.local), ("remote", &file.remote)] {
            if let Some(text) = text {
                plan.sources.push(Source { id: format!("{side}:{}", file.path), side: side.into(), path: file.path.clone(), text: text.clone() });
            }
        }
    }
    collect_evidence(root, &mut plan, &base_files, &local_files, &remote_files);
    save_plan(folder, &plan)?;
    let now = chrono::Utc::now().timestamp();
    let items: Vec<Item> = plan
        .files
        .iter()
        .filter_map(|file| {
            let kind = *kinds.get(&file.path)?;
            let (title, detail) = if kind == "merged" {
                ("Both PCs changed the same lines", "Lore kept the lines from both PCs. Check the note for repeated or disagreeing lines.")
            } else {
                ("Deleted on one PC, edited on the other", "Lore kept the edited note. Delete it again if it should be gone.")
            };
            Some(Item { id: digest(format!("{}:{kind}:{}", plan.id, file.path).as_bytes())[..12].into(), kind: kind.into(), plan: plan.id.clone(), path: file.path.clone(), title: title.into(), detail: detail.into(), created_at: now, issue: None })
        })
        .collect();
    let listed = items.len();
    update(folder, |state| {
        state.items.retain(|known| items.iter().all(|item| item.id != known.id));
        state.items.extend(items);
        state.comparisons.retain(|comparison| comparison.plan != plan.id);
        state.comparisons.push(Comparison { plan: plan.id.clone(), error: None });
    })?;
    Ok((commit, listed))
}

/// Combine two versions keeping the lines of both where they changed the same place.
fn union(root: &Path, folder: &Path, ours: &str, base: &str, theirs: &str) -> Result<String> {
    let work = folder.join("merge-files");
    std::fs::create_dir_all(&work)?;
    let merged = (|| {
        let mut paths = Vec::new();
        for (name, text) in [("ours", ours), ("base", base), ("theirs", theirs)] {
            let path = work.join(name);
            std::fs::write(&path, text)?;
            paths.push(path.to_string_lossy().to_string());
        }
        git(root, &["merge-file", "-p", "--union", &paths[0], &paths[1], &paths[2]])
    })();
    let _ = std::fs::remove_dir_all(&work);
    merged
}

fn source_paths(text: &str) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for part in text.split("sources/").skip(1) {
        let Some(end) = part.find(".md") else {
            continue;
        };
        let suffix = &part[..end];
        let items: Vec<_> = suffix.split('/').collect();
        if let [provider, session] = items.as_slice() {
            if matches!(*provider, "claude" | "codex") && !session.is_empty() && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                result.insert(format!("sources/{suffix}.md"));
            }
        }
    }
    result
}
fn collect_evidence(root: &Path, plan: &mut Plan, base_files: &BTreeSet<String>, local_files: &BTreeSet<String>, remote_files: &BTreeSet<String>) {
    let mut requests = BTreeSet::new();
    for source in &plan.sources {
        for path in source_paths(&source.text) {
            requests.insert((source.side.clone(), path));
        }
    }
    for (side, path) in requests {
        let (revision, files) = match side.as_str() {
            "base" => (&plan.base, base_files),
            "local" => (&plan.local, local_files),
            _ => (&plan.remote, remote_files),
        };
        if let Some(text) = blob(root, revision, &path, files) {
            plan.sources.push(Source { id: format!("{side}:{path}"), side, path, text });
        }
    }
}

fn plan_path(folder: &Path, id: &str) -> PathBuf {
    folder.join(format!("merge-{id}.json"))
}
fn save_plan(folder: &Path, plan: &Plan) -> Result<()> {
    atomic_write(&plan_path(folder, &plan.id), &serde_json::to_vec(plan)?)
}
fn load_plan(folder: &Path, id: &str) -> Result<Plan> {
    let bytes = std::fs::read(plan_path(folder, id)).map_err(|_| Error::permanent("The saved versions of this note are no longer available. Dismiss the item."))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Remove saved merges nothing refers to any more.
fn prune(folder: &Path, state: &State) {
    let used: BTreeSet<&str> = state.items.iter().map(|item| item.plan.as_str()).chain(state.comparisons.iter().map(|comparison| comparison.plan.as_str())).collect();
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let id = name.strip_prefix("merge-").and_then(|rest| rest.strip_suffix(".json")).or_else(|| name.strip_prefix("analysis-"));
        if id.is_some_and(|id| !used.contains(id)) {
            let _ = if entry.path().is_dir() { std::fs::remove_dir_all(entry.path()) } else { std::fs::remove_file(entry.path()) };
        }
    }
}

pub fn analysis_schema() -> Value {
    fn object(properties: Value) -> Value {
        let required: Vec<String> = properties.as_object().unwrap().keys().cloned().collect();
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    }
    let edit = object(json!({"path":{"type":"string"},"find":{"type":"string"},"text":{"type":"string"}}));
    let option = object(json!({"label":{"type":"string"},"description":{"type":"string"},"edits":{"type":"array","items":edit}}));
    let evidence = object(json!({"source":{"type":"string"},"quote":{"type":"string"}}));
    let issue = object(json!({"title":{"type":"string"},"explanation":{"type":"string"},"evidence":{"type":"array","items":evidence},"options":{"type":"array","items":option}}));
    object(json!({"summary":{"type":"string"},"issues":{"type":"array","items":issue}}))
}

pub fn analysis_artifacts(folder: &Path, id: &str) -> PathBuf {
    folder.join(format!("analysis-{id}"))
}

pub fn comparison_progress(folder: &Path, id: &str) -> Option<ComparisonProgress> {
    comparison::progress(&analysis_artifacts(folder, id))
}

/// The next merge whose notes still need a model comparison.
pub fn pending_comparison(folder: &Path) -> Option<String> {
    state(folder).comparisons.into_iter().find(|comparison| comparison.error.is_none()).map(|comparison| comparison.plan)
}

/// Let failed comparisons run again. They continue from their saved progress.
pub fn retry_comparison(folder: &Path) -> Result<()> {
    update(folder, |state| state.comparisons.iter_mut().for_each(|comparison| comparison.error = None))
}

/// Stop waiting for comparisons, for example when the user does not want them.
pub fn skip_comparisons(folder: &Path) -> Result<()> {
    update(folder, |state| {
        state.comparisons.clear();
        prune(folder, state);
    })
}

/// Ask the model whether the notes both PCs changed now disagree. Reads only
/// the saved versions, so recording and sync keep running meanwhile. Each
/// disagreement becomes an item; returns how many were found.
pub fn compare(folder: &Path, id: &str, identity: &str, request: &mut impl FnMut(&str, &Value, &Path) -> Result<Value>, changed: &mut impl FnMut()) -> Result<usize> {
    let mut plan = load_plan(folder, id)?;
    let result = comparison::run(&plan, &analysis_artifacts(folder, id), identity, request, changed).and_then(|analysis| {
        validate_analysis(&plan, &analysis)?;
        Ok(analysis)
    });
    match result {
        Ok(analysis) => {
            let now = chrono::Utc::now().timestamp();
            let items: Vec<Item> = analysis
                .issues
                .iter()
                .enumerate()
                .map(|(index, issue)| Item {
                    id: digest(format!("{id}:decision:{index}").as_bytes())[..12].into(),
                    kind: "decision".into(),
                    plan: id.into(),
                    path: issue.options.iter().flat_map(|option| &option.edits).map(|edit| edit.path.clone()).next().unwrap_or_default(),
                    title: issue.title.clone(),
                    detail: issue.explanation.clone(),
                    created_at: now,
                    issue: Some(index),
                })
                .collect();
            let found = items.len();
            plan.analysis = Some(analysis);
            save_plan(folder, &plan)?;
            update(folder, |state| {
                state.items.retain(|known| items.iter().all(|item| item.id != known.id));
                state.items.extend(items);
                state.comparisons.retain(|comparison| comparison.plan != id);
                prune(folder, state);
            })?;
            Ok(found)
        }
        Err(error) => {
            update(folder, |state| {
                if let Some(comparison) = state.comparisons.iter_mut().find(|comparison| comparison.plan == id) {
                    comparison.error = Some(error.to_string());
                }
            })?;
            Err(error)
        }
    }
}

fn short(text: &str) -> String {
    let cut: String = text.chars().take(120).collect();
    if cut.len() < text.len() { format!("{cut}…") } else { cut }
}

/// The text a quotation may come from: a provided version or conversation, or
/// the merged note itself.
fn evidence_text<'a>(plan: &'a Plan, source: &str) -> Option<&'a str> {
    if let Some(path) = source.strip_prefix("merged:") {
        return plan.files.iter().find(|file| file.path == path).and_then(|file| file.merged.as_deref());
    }
    plan.sources.iter().find(|known| known.id == source).map(|known| known.text.as_str())
}

pub fn validate_analysis(plan: &Plan, analysis: &Analysis) -> Result<()> {
    if analysis.summary.trim().is_empty() || analysis.issues.len() > 512 {
        return Err(Error::rejected("The model comparison summary or issue count is invalid."));
    }
    let mut ranges: Vec<(usize, String, usize, usize)> = Vec::new();
    for (issue_index, issue) in analysis.issues.iter().enumerate() {
        if issue.title.trim().is_empty() || issue.explanation.trim().is_empty() || issue.evidence.is_empty() || !(2..=4).contains(&issue.options.len()) {
            return Err(Error::rejected("Each issue requires an explanation, evidence, and 2–4 options."));
        }
        for evidence in &issue.evidence {
            if evidence.quote.trim().is_empty() || !evidence_text(plan, &evidence.source).is_some_and(|text| text.contains(&evidence.quote)) {
                return Err(Error::rejected(format!("This quotation is not in source {}: \"{}\". Quote text exactly as it appears in a source you read.", evidence.source, short(&evidence.quote))));
            }
        }
        for option in &issue.options {
            if option.label.trim().is_empty() || option.edits.is_empty() {
                return Err(Error::rejected("Empty options are not allowed."));
            }
            let mut preview: BTreeMap<String, String> = plan.files.iter().filter_map(|file| file.merged.as_ref().map(|text| (file.path.clone(), text.clone()))).collect();
            for edit in &option.edits {
                let original = plan.files.iter().find(|file| file.path == edit.path).and_then(|file| file.merged.as_ref()).ok_or_else(|| Error::rejected(format!("The edit targets a document outside this comparison: {}", edit.path)))?;
                if edit.find.is_empty() || original.matches(&edit.find).count() != 1 {
                    return Err(Error::rejected(format!("The edit target must occur exactly once in merged:{}: \"{}\"", edit.path, short(&edit.find))));
                }
                let start = original.find(&edit.find).unwrap();
                let end = start + edit.find.len();
                if ranges.iter().any(|(index, path, a, b)| *index != issue_index && path == &edit.path && start < *b && end > *a) {
                    return Err(Error::rejected("Edits from different issues overlap. Group these edits into one issue."));
                }
                ranges.push((issue_index, edit.path.clone(), start, end));
                apply_edit(&mut preview, edit)?;
            }
        }
    }
    Ok(())
}

fn apply_edit(files: &mut BTreeMap<String, String>, edit: &Edit) -> Result<()> {
    let text = files.get_mut(&edit.path).ok_or_else(|| Error::rejected(format!("Cannot edit a document outside the comparison: {}", edit.path)))?;
    if edit.find.is_empty() || text.matches(&edit.find).count() != 1 {
        return Err(Error::rejected(format!("The edit target does not occur exactly once: {}", edit.path)));
    }
    *text = text.replacen(&edit.find, &edit.text, 1);
    Ok(())
}

fn evidence_links(text: &str) -> BTreeSet<String> {
    let mut links = BTreeSet::new();
    for part in text.split("](").skip(1) {
        if let Some(end) = part.find(')') {
            let link = &part[..end];
            if link.contains("sources/") {
                links.insert(link.to_string());
            }
        }
    }
    links
}

fn find_item(folder: &Path, id: &str) -> Result<Item> {
    state(folder).items.into_iter().find(|item| item.id == id).ok_or_else(|| Error::permanent("This item is no longer listed."))
}

/// Everything needed to show one item: the versions of its notes and, for a
/// decision, the model's explanation with the sources it quoted.
pub fn detail(folder: &Path, id: &str) -> Result<Detail> {
    let item = find_item(folder, id)?;
    let plan = load_plan(folder, &item.plan)?;
    let issue = item.issue.and_then(|index| plan.analysis.as_ref()?.issues.get(index).cloned());
    let paths: BTreeSet<String> = match &issue {
        Some(issue) => issue.options.iter().flat_map(|option| option.edits.iter().map(|edit| edit.path.clone())).collect(),
        None => BTreeSet::from([item.path.clone()]),
    };
    let mut sources = Vec::new();
    for evidence in issue.iter().flat_map(|issue| &issue.evidence) {
        if sources.iter().any(|known: &Source| known.id == evidence.source) {
            continue;
        }
        if let Some(text) = evidence_text(&plan, &evidence.source) {
            let (side, path) = evidence.source.split_once(':').unwrap_or(("", &evidence.source));
            sources.push(Source { id: evidence.source.clone(), side: side.into(), path: path.into(), text: text.into() });
        }
    }
    Ok(Detail { files: plan.files.into_iter().filter(|file| paths.contains(&file.path)).collect(), item, issue, sources })
}

/// What choosing an option would do to the notes as they are now: path to
/// (current text, text after). Nothing is written.
pub fn preview_item(root: &Path, folder: &Path, id: &str, option: usize, custom: Option<&str>) -> Result<BTreeMap<String, (String, String)>> {
    let item = find_item(folder, id)?;
    let plan = load_plan(folder, &item.plan)?;
    let chosen = item
        .issue
        .and_then(|index| plan.analysis.as_ref()?.issues.get(index))
        .and_then(|issue| issue.options.get(option))
        .ok_or_else(|| Error::permanent("This option is not available. Open the item again."))?;
    if custom.is_some_and(|text| chosen.edits.len() != 1 || text.trim().is_empty()) {
        return Err(Error::permanent("Custom wording must be nonempty and can only be used for a single-edit option."));
    }
    let mut before: BTreeMap<String, String> = BTreeMap::new();
    for edit in &chosen.edits {
        if !is_note(&edit.path) || edit.path.split('/').any(|part| part == ".." || part.is_empty()) {
            return Err(Error::rejected("The option edits a file that is not a memory note."));
        }
        if !before.contains_key(&edit.path) {
            let current = std::fs::read_to_string(root.join(&edit.path)).map_err(|_| Error::permanent(format!("The note no longer exists: {}", edit.path)))?;
            before.insert(edit.path.clone(), current.replace("\r\n", "\n"));
        }
    }
    let mut after = before.clone();
    for edit in &chosen.edits {
        let mut edit = edit.clone();
        if let Some(text) = custom {
            edit.text = text.to_string();
        }
        apply_edit(&mut after, &edit).map_err(|_| Error::permanent(format!("{} changed after the comparison, so this option no longer fits. Edit the note directly, then dismiss the item.", edit.path)))?;
    }
    // Source links are the record of where a statement came from: never drop or invent one.
    for (path, text) in &after {
        if evidence_links(text) != evidence_links(&before[path]) {
            return Err(Error::rejected(format!("The option would add or remove a source link: {path}")));
        }
    }
    Ok(before.into_iter().map(|(path, current)| { let next = after[&path].clone(); (path, (current, next)) }).collect())
}

/// Apply an option to the notes and commit it like any other change. The next
/// sync shares it.
pub fn resolve_item(root: &Path, folder: &Path, id: &str, option: usize, custom: Option<&str>) -> Result<()> {
    let changes = preview_item(root, folder, id, option, custom)?;
    review::record_commit(root)?;
    for (path, (_, after)) in &changes {
        let target = root.join(path);
        let crlf = std::fs::read_to_string(&target).is_ok_and(|current| current.contains("\r\n"));
        std::fs::write(&target, if crlf { after.replace('\n', "\r\n") } else { after.clone() })?;
    }
    review::commit_all(root, "Resolve a memory sync decision")?;
    dismiss_item(folder, id)
}

pub fn dismiss_item(folder: &Path, id: &str) -> Result<()> {
    update(folder, |state| {
        state.items.retain(|item| item.id != id);
        prune(folder, state);
    })
}
