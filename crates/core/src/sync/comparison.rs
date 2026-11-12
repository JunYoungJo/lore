//! Bounded, resumable model comparisons. Only immutable plan data can be read;
//! model requests never grant filesystem or shell access.
use super::{analysis_schema, scope, source_paths, validate_analysis, Analysis, Plan};
use crate::error::{Error, Result};
use crate::settings::atomic_write;
use crate::transcript::digest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_CONTEXT: usize = 96_000;
const PAGE: usize = 8_000;
const GROUP_CHANGES: usize = 36_000;
const MAX_ROUNDS: usize = 24;
/// A reply that fails validation goes back to the model with the reason this
/// many times before the comparison stops.
const MAX_REJECTIONS: usize = 3;
const INSTRUCTIONS: &str = include_str!("../sync-prompt.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
    pub stage: String,
    pub reads: usize,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    source: String,
    offset: usize,
    length: usize,
    anchor: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Page {
    source: String,
    offset: usize,
    text: String,
    next_offset: usize,
    total_chars: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    summary: String,
    issues: Vec<super::Issue>,
    reads: Vec<Read>,
    notes: String,
    replace_issues: Vec<usize>,
}

#[derive(Serialize, Deserialize)]
struct Task {
    pages: Vec<Page>,
    seen: BTreeMap<String, Vec<(usize, usize)>>,
    draft: Reply,
    result: Option<Reply>,
    attempts: usize,
    feedback: String,
    #[serde(default = "default_budget")]
    budget: usize,
}

fn default_budget() -> usize {
    MAX_CONTEXT
}

impl Default for Task {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            seen: BTreeMap::new(),
            draft: Reply::default(),
            result: None,
            attempts: 0,
            feedback: String::new(),
            budget: MAX_CONTEXT,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    key: String,
    groups: Vec<Vec<String>>,
    tasks: Vec<Task>,
    consistency: Task,
    progress: Progress,
}

struct Document<'a> {
    side: String,
    path: String,
    text: Cow<'a, str>,
}
type Documents<'a> = BTreeMap<String, Document<'a>>;

fn documents(plan: &Plan) -> Documents<'_> {
    let mut docs: Documents<'_> = plan
        .sources
        .iter()
        .map(|source| {
            (
                source.id.clone(),
                Document {
                    side: source.side.clone(),
                    path: source.path.clone(),
                    text: Cow::Borrowed(source.text.as_str()),
                },
            )
        })
        .collect();
    for file in &plan.files {
        if let Some(text) = &file.merged {
            docs.insert(
                format!("merged:{}", file.path),
                Document {
                    side: "merged".into(),
                    path: file.path.clone(),
                    text: Cow::Borrowed(text),
                },
            );
        }
        for (side, text) in [("local", &file.local), ("remote", &file.remote)] {
            let diff = similar::TextDiff::from_lines(
                file.base.as_deref().unwrap_or(""),
                text.as_deref().unwrap_or(""),
            )
            .unified_diff()
            .context_radius(5)
            .to_string();
            docs.insert(
                format!("changes-{side}:{}", file.path),
                Document {
                    side: format!("changes-{side}"),
                    path: file.path.clone(),
                    text: Cow::Owned(diff),
                },
            );
        }
    }
    docs
}

fn groups(plan: &Plan, docs: &Documents<'_>) -> Vec<Vec<String>> {
    let mut related: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in &plan.files {
        related
            .entry(scope(&file.path).unwrap_or_default())
            .or_default()
            .push(file.path.clone());
    }
    let mut groups = Vec::new();
    for mut files in related.into_values() {
        files.sort();
        let mut group = Vec::new();
        let mut size = 0;
        for file in files {
            let chars: usize = ["local", "remote"]
                .iter()
                .map(|side| docs[&format!("changes-{side}:{file}")].text.chars().count())
                .sum();
            if !group.is_empty() && (size + chars > GROUP_CHANGES || group.len() >= 8) {
                groups.push(std::mem::take(&mut group));
                size = 0;
            }
            size += chars;
            group.push(file);
        }
        if !group.is_empty() {
            groups.push(group);
        }
    }
    groups
}

fn schema() -> Value {
    let mut schema = analysis_schema();
    schema["properties"]["reads"] = json!({"type":"array","items":{
        "type":"object","properties":{
            "source":{"type":"string"},"offset":{"type":"integer","minimum":0},
            "length":{"type":"integer","minimum":1,"maximum":PAGE},"anchor":{"type":"string"}
        },"required":["source","offset","length","anchor"],"additionalProperties":false
    }});
    schema["properties"]["notes"] = json!({"type":"string"});
    schema["properties"]["replace_issues"] =
        json!({"type":"array","items":{"type":"integer","minimum":0}});
    schema["required"] = json!(["summary", "issues", "reads", "notes", "replace_issues"]);
    schema
}

fn page(docs: &Documents<'_>, read: &Read) -> Result<Page> {
    let doc = docs
        .get(&read.source)
        .ok_or_else(|| Error::rejected("The requested comparison source is not in this plan."))?;
    let total = doc.text.chars().count();
    if read.length == 0
        || read.length > PAGE
        || read.offset > total
        || read.anchor.chars().count() > 200
    {
        return Err(Error::rejected(
            "The requested source range is outside the allowed page size.",
        ));
    }
    let mut offset = read.offset;
    if !read.anchor.is_empty() {
        let start = doc
            .text
            .char_indices()
            .nth(offset)
            .map_or(doc.text.len(), |(i, _)| i);
        let found = doc.text[start..].find(&read.anchor).ok_or_else(|| {
            Error::rejected(format!(
                "The anchor \"{}\" does not occur in {} after offset {}. Use an exact phrase from text you have read, or an empty anchor with an offset.",
                super::short(&read.anchor),
                read.source,
                read.offset
            ))
        })?;
        offset = doc.text[..start + found]
            .chars()
            .count()
            .saturating_sub((read.length / 4).min(300));
    }
    let text: String = doc.text.chars().skip(offset).take(read.length).collect();
    Ok(Page {
        source: read.source.clone(),
        offset,
        next_offset: offset + text.chars().count(),
        text,
        total_chars: total,
    })
}

fn seed(docs: &Documents<'_>, paths: &[String]) -> Vec<Page> {
    let mut pages = Vec::new();
    // Differences cover all changed sections; full versions stay available for
    // exact edit anchors and context reads, including unchanged paragraphs.
    for path in paths {
        for side in [
            "changes-local",
            "changes-remote",
            "merged",
            "local",
            "remote",
            "base",
        ] {
            let id = format!("{side}:{path}");
            if docs.contains_key(&id) {
                if let Ok(p) = page(
                    docs,
                    &Read {
                        source: id.clone(),
                        offset: 0,
                        length: if side.starts_with("changes-") {
                            PAGE
                        } else {
                            2_000
                        },
                        anchor: String::new(),
                    },
                ) {
                    pages.push(p);
                }
            }
        }
        if let Some(doc) = docs.get(&format!("merged:{path}")) {
            for (byte, _) in doc.text.match_indices("<<<<<<<") {
                let offset = doc.text[..byte].chars().count().saturating_sub(100);
                if let Ok(p) = page(
                    docs,
                    &Read {
                        source: format!("merged:{path}"),
                        offset,
                        length: PAGE,
                        anchor: String::new(),
                    },
                ) {
                    pages.push(p);
                }
            }
        }
    }
    pages
}

fn record_seen(seen: &mut BTreeMap<String, Vec<(usize, usize)>>, page: &Page) {
    let ranges = seen.entry(page.source.clone()).or_default();
    ranges.push((page.offset, page.next_offset));
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for &(start, end) in ranges.iter() {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    *ranges = merged;
}

fn unseen(docs: &Documents<'_>, task: &Task, required: &[String]) -> Vec<Read> {
    let mut reads = Vec::new();
    for id in required {
        let total = docs[id].text.chars().count();
        let mut offset = 0;
        for &(start, end) in task.seen.get(id).into_iter().flatten() {
            if start > offset {
                break;
            }
            offset = offset.max(end);
        }
        if offset < total {
            reads.push(Read {
                source: id.clone(),
                offset,
                length: PAGE,
                anchor: String::new(),
            });
        }
        if reads.len() == 4 {
            break;
        }
    }
    reads
}

fn was_seen(docs: &Documents<'_>, task: &Task, id: &str, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let Some(doc) = docs.get(id) else {
        return false;
    };
    doc.text.match_indices(text).any(|(byte, _)| {
        let start = doc.text[..byte].chars().count();
        let end = start + text.chars().count();
        task.seen
            .get(id)
            .is_some_and(|ranges| ranges.iter().any(|&(a, b)| a <= start && b >= end))
    })
}

fn validate_seen(docs: &Documents<'_>, task: &Task, reply: &Reply) -> Result<()> {
    for issue in &reply.issues {
        for evidence in &issue.evidence {
            if !was_seen(docs, task, &evidence.source, &evidence.quote) {
                return Err(Error::rejected(format!(
                    "This quotation is not in the parts of {} you have read: \"{}\". Read that range first and quote it exactly.",
                    evidence.source,
                    super::short(&evidence.quote)
                )));
            }
        }
        for edit in issue.options.iter().flat_map(|option| &option.edits) {
            if !was_seen(docs, task, &format!("merged:{}", edit.path), &edit.find) {
                return Err(Error::rejected(format!(
                    "This edit target is not in the parts of merged:{} you have read: \"{}\". Read that range first and copy it exactly.",
                    edit.path,
                    super::short(&edit.find)
                )));
            }
        }
    }
    Ok(())
}

fn prompt(
    plan: &Plan,
    docs: &Documents<'_>,
    paths: &[String],
    task: &mut Task,
    existing: &Analysis,
    consistency: bool,
) -> Result<String> {
    let mut relevant = BTreeSet::new();
    for file in plan.files.iter().filter(|file| paths.contains(&file.path)) {
        for text in [&file.base, &file.local, &file.remote]
            .into_iter()
            .flatten()
        {
            relevant.extend(source_paths(text));
        }
    }
    let catalog: Vec<_> = docs.iter().filter(|(_, doc)| paths.contains(&doc.path) || relevant.contains(&doc.path) || doc.side == "result")
        .map(|(id, doc)| json!({"id":id,"side":doc.side,"path":doc.path,"chars":doc.text.chars().count()})).collect();
    // The full catalog is itself pageable. A very large evidence index therefore
    // cannot force the body of every conversation into a single request.
    let catalog_text = serde_json::to_string(&catalog)?;
    let catalog_preview: String = catalog_text.chars().take(8_000).collect();
    let files: Vec<_> = plan.files.iter().filter(|file| paths.contains(&file.path)).map(|file| json!({
        "path":file.path,"textConflict":file.text_conflict,
        "availableVersions": (["base","local","remote","merged"].into_iter().filter(|side| docs.contains_key(&format!("{side}:{}", file.path))).collect::<Vec<_>>())
    })).collect();
    let issue_index = if consistency {
        serde_json::to_string(&index(existing))?
    } else {
        "[]".into()
    };
    loop {
        let payload = json!({
            "stage":if consistency {"consistency"} else {"comparison"},
            "files":files,"catalog":catalog_preview,"catalogChars":catalog_text.chars().count(),
            "pages":task.pages,"workingNotes":task.draft.notes,"draftIssues":task.draft.issues,
            "draftReplacements":task.draft.replace_issues,"feedback":task.feedback,
            "existingIssues":issue_index.chars().take(8_000).collect::<String>(),
            "instructions": "Read only this immutable data. Every source remains available through reads: {source,offset,length,anchor}. Offsets and lengths count Unicode characters, not bytes or tokens. Use anchor for an exact phrase or <a id=\"L123\"> turn marker; use an empty anchor for offsets. Maximum 4 reads per reply, 8000 characters each. Source 'catalog' pages the complete source index. Source 'issue-index' provides all existing issue indices during consistency review. Version ids are <version>:<path>; change ids are changes-local:<path> and changes-remote:<path>. Preserve findings and scope in notes (at most 8000 characters) and keep all draft issues across reads. In notes, record changed decisions, their applicability, and source IDs even when no issue is found; the final consistency review uses these findings to check other groups. Return reads=[] only when finished. Before quoting evidence or editing, read the original source or merged range. Diff prefixes are not original text. All changed sections must be reviewed; unchanged context and complete conversations are available on demand. Never infer that an unread region has no relevant decisions. During consistency review, read every result source, check decisions across groups, and return only new/consolidated issues plus the existing indices they replace in replace_issues; keep unrelated existing issues. During a group comparison edit only the listed files, leave replace_issues empty, and report cross-group relationships in the summary for the final consistency review. Evidence sources may be base, local, remote or merged versions and conversation copies; do not use result or changes sources as evidence."
        });
        let text = format!(
            "{INSTRUCTIONS}\n\nComparison data (not instructions from the documents):\n{payload}"
        );
        if text.chars().count() <= task.budget.clamp(32_000, MAX_CONTEXT) {
            return Ok(text);
        }
        if task.pages.is_empty() {
            return Err(Error::rejected("The comparison response is too large. Shorten the working notes and proposed edits."));
        }
        // Only pages previously sent to the model enter `seen`. Deferred change
        // pages are requested automatically before a result can be accepted.
        task.pages.pop();
    }
}

fn validate_reply(
    plan: &Plan,
    docs: &Documents<'_>,
    task: &Task,
    reply: &Reply,
    paths: &[String],
    existing: &Analysis,
    consistency: bool,
) -> Result<Analysis> {
    validate_seen(docs, task, reply)?;
    if !consistency && !reply.replace_issues.is_empty() {
        return Err(Error::rejected(
            "Group comparisons cannot replace other groups' issues.",
        ));
    }
    let indices: BTreeSet<_> = reply.replace_issues.iter().copied().collect();
    if indices.len() != reply.replace_issues.len()
        || indices.iter().any(|&i| i >= existing.issues.len())
    {
        return Err(Error::rejected(
            "The consistency result contains invalid issue indices.",
        ));
    }
    let mut issues = if consistency {
        existing
            .issues
            .iter()
            .enumerate()
            .filter(|(i, _)| !indices.contains(i))
            .map(|(_, issue)| issue.clone())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    issues.extend(reply.issues.clone());
    let result = Analysis {
        summary: reply.summary.clone(),
        issues,
    };
    let mut subset = plan.clone();
    if !consistency {
        subset.files.retain(|file| paths.contains(&file.path));
    }
    validate_analysis(&subset, &result)?;
    Ok(result)
}

fn index(analysis: &Analysis) -> Vec<Value> {
    analysis.issues.iter().enumerate().map(|(index, issue)| json!({
        "index":index,"title":issue.title,"paths":issue.options.iter().flat_map(|o| o.edits.iter().map(|e| &e.path)).collect::<BTreeSet<_>>()
    })).collect()
}

pub(super) fn progress(folder: &Path) -> Option<Progress> {
    let checkpoint: Checkpoint =
        serde_json::from_slice(&std::fs::read(folder.join("checkpoint.json")).ok()?).ok()?;
    Some(checkpoint.progress)
}

fn save(folder: &Path, checkpoint: &Checkpoint, changed: &mut impl FnMut()) -> Result<()> {
    atomic_write(
        &folder.join("checkpoint.json"),
        &serde_json::to_vec(checkpoint)?,
    )?;
    changed();
    Ok(())
}

pub(super) fn run(
    plan: &Plan,
    folder: &Path,
    request: &mut impl FnMut(&str, &Value, &Path) -> Result<Value>,
    changed: &mut impl FnMut(),
) -> Result<Analysis> {
    let mut docs = documents(plan);
    // The key names the notes being compared, not the model: a finished group stays
    // finished when another CLI takes over, because the program checked its result.
    let key = digest(
        format!(
            "paged-v3:{}",
            serde_json::to_string(&json!([
                plan.id,
                plan.local,
                plan.remote,
                plan.tree,
                plan.files,
                plan.sources
            ]))?
        )
        .as_bytes(),
    );
    let groups = groups(plan, &docs);
    let mut checkpoint: Checkpoint = std::fs::read(folder.join("checkpoint.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(|c: &Checkpoint| {
            c.key == key
                && c.groups == groups
                && c.tasks.len() == groups.len()
                && c.progress.stage != "complete"
        })
        .unwrap_or_else(|| Checkpoint {
            key,
            tasks: groups
                .iter()
                .map(|paths| Task {
                    pages: seed(&docs, paths),
                    ..Task::default()
                })
                .collect(),
            progress: Progress {
                completed: 0,
                total: groups.len(),
                stage: "comparing".into(),
                reads: 0,
                error: None,
            },
            groups,
            consistency: Task::default(),
        });
    checkpoint.progress.error = None;
    let result = run_tasks(plan, folder, &mut docs, &mut checkpoint, request, changed);
    match result {
        Ok(analysis) => {
            checkpoint.progress.stage = "complete".into();
            save(folder, &checkpoint, changed)?;
            Ok(analysis)
        }
        Err(error) => {
            checkpoint.progress.stage = "paused".into();
            checkpoint.progress.error = Some(error.to_string());
            save(folder, &checkpoint, changed)?;
            Err(error)
        }
    }
}

fn run_tasks(
    plan: &Plan,
    folder: &Path,
    docs: &mut Documents<'_>,
    checkpoint: &mut Checkpoint,
    request: &mut impl FnMut(&str, &Value, &Path) -> Result<Value>,
    changed: &mut impl FnMut(),
) -> Result<Analysis> {
    let mut existing = Analysis {
        summary: String::new(),
        issues: Vec::new(),
    };
    for index in 0..checkpoint.groups.len() {
        let paths = checkpoint.groups[index].clone();
        checkpoint.progress.stage = "comparing".into();
        let required: Vec<_> = paths
            .iter()
            .flat_map(|path| {
                [
                    format!("changes-local:{path}"),
                    format!("changes-remote:{path}"),
                ]
            })
            .collect();
        execute(
            plan,
            folder,
            docs,
            checkpoint,
            Some(index),
            &paths,
            &required,
            &existing,
            request,
            changed,
        )?;
        let reply = checkpoint.tasks[index].result.as_ref().unwrap();
        existing.summary.push_str(&format!("{}\n", reply.summary));
        existing.issues.extend(reply.issues.clone());
        docs.insert(
            format!("result:{index}"),
            Document {
                side: "result".into(),
                path: paths.join(", "),
                text: Cow::Owned(serde_json::to_string(&json!({"summary":reply.summary,"issues":reply.issues,"notes":reply.notes,"paths":paths}))?),
            },
        );
        checkpoint.progress.completed = checkpoint.progress.completed.max(index + 1);
        save(folder, checkpoint, changed)?;
    }
    if checkpoint.groups.len() > 1 {
        checkpoint.progress.stage = "checking".into();
        let paths: Vec<_> = plan.files.iter().map(|file| file.path.clone()).collect();
        let mut required: Vec<_> = (0..checkpoint.groups.len())
            .map(|i| format!("result:{i}"))
            .collect();
        docs.insert(
            "issue-index".into(),
            Document {
                side: "index".into(),
                path: String::new(),
                text: Cow::Owned(serde_json::to_string(&index(&existing))?),
            },
        );
        required.push("issue-index".into());
        if checkpoint.consistency.pages.is_empty() && checkpoint.consistency.attempts == 0 {
            checkpoint.consistency.pages = required
                .iter()
                .map(|id| {
                    page(
                        docs,
                        &Read {
                            source: id.clone(),
                            offset: 0,
                            length: PAGE,
                            anchor: String::new(),
                        },
                    )
                })
                .collect::<Result<_>>()?;
            // Evidence already inspected in individual groups remains inspected;
            // the consistency pass must still read new edit anchors itself.
            for task in &checkpoint.tasks {
                for (id, ranges) in &task.seen {
                    for &(a, b) in ranges {
                        record_seen(
                            &mut checkpoint.consistency.seen,
                            &Page {
                                source: id.clone(),
                                offset: a,
                                next_offset: b,
                                text: String::new(),
                                total_chars: 0,
                            },
                        );
                    }
                }
            }
        }
        execute(
            plan, folder, docs, checkpoint, None, &paths, &required, &existing, request, changed,
        )?;
        let task = &checkpoint.consistency;
        return validate_reply(
            plan,
            docs,
            task,
            task.result.as_ref().unwrap(),
            &paths,
            &existing,
            true,
        );
    }
    if checkpoint.groups.is_empty() {
        existing.summary = "No changed notes require model comparison.".into();
    }
    validate_analysis(plan, &existing)?;
    Ok(existing)
}

#[allow(clippy::too_many_arguments)]
fn execute(
    plan: &Plan,
    folder: &Path,
    docs: &mut Documents<'_>,
    checkpoint: &mut Checkpoint,
    index: Option<usize>,
    paths: &[String],
    required: &[String],
    existing: &Analysis,
    request: &mut impl FnMut(&str, &Value, &Path) -> Result<Value>,
    changed: &mut impl FnMut(),
) -> Result<()> {
    if task_at(checkpoint, index).result.is_some() {
        return Ok(());
    }
    // The complete catalog can be read using exactly the same bounded protocol.
    let catalog: Vec<_> = docs.iter().filter(|(id, _)| id.as_str() != "catalog").map(|(id, doc)| json!({"id":id,"side":doc.side,"path":doc.path,"chars":doc.text.chars().count()})).collect();
    docs.insert(
        "catalog".into(),
        Document {
            side: "catalog".into(),
            path: String::new(),
            text: Cow::Owned(serde_json::to_string(&catalog)?),
        },
    );
    let mut rejections = 0;
    for _ in 0..MAX_ROUNDS {
        let task = task_at(checkpoint, index);
        let text = prompt(plan, docs, paths, task, existing, index.is_none())?;
        task.attempts += 1;
        let attempt = task.attempts;
        save(folder, checkpoint, changed)?;
        let call_folder = folder
            .join(index.map_or_else(|| "consistency".into(), |i| format!("group-{i}")))
            .join(format!("request-{attempt}"));
        let value = match request(&text, &schema(), &call_folder) {
            Ok(value) => value,
            Err(error) => {
                let message = error.to_string().to_lowercase();
                let task = task_at(checkpoint, index);
                let budget = task.budget.clamp(32_000, MAX_CONTEXT);
                if budget > 32_000
                    && [
                        "context limit",
                        "context length",
                        "context window",
                        "prompt is too long",
                        "prompt too long",
                        "input is too long",
                        "too many tokens",
                    ]
                    .iter()
                    .any(|phrase| message.contains(phrase))
                {
                    task.budget = (budget / 2).max(32_000);
                    task.feedback = "The provider rejected the previous input size. Continue using smaller source pages.".into();
                    save(folder, checkpoint, changed)?;
                    continue;
                }
                return Err(error);
            }
        };
        let reply: Reply = serde_json::from_value(value)
            .map_err(|e| Error::rejected(format!("Invalid comparison response: {e}")))?;
        if reply.notes.chars().count() > 8_000
            || reply.reads.len() > 4
            || serde_json::to_string(&reply)?.chars().count() > 64_000
        {
            task_at(checkpoint, index).feedback = "Keep notes below 8000 characters, return at most four reads, and use focused edits.".into();
            save(folder, checkpoint, changed)?;
            return Err(Error::rejected("The comparison response exceeded its page budget. Resume to request a smaller response."));
        }
        let task = task_at(checkpoint, index);
        for p in &task.pages {
            record_seen(&mut task.seen, p);
        }
        let reads = if reply.reads.is_empty() {
            unseen(docs, task, required)
        } else {
            reply.reads.clone()
        };
        if reads.is_empty() {
            if let Err(error) =
                validate_reply(plan, docs, task, &reply, paths, existing, index.is_none())
            {
                rejections += 1;
                task.feedback = format!("Your previous reply was rejected: {error} Correct this and return the complete result again.");
                // Keep the draft so the model corrects its reply instead of starting over.
                task.draft = reply;
                save(folder, checkpoint, changed)?;
                if rejections >= MAX_REJECTIONS {
                    return Err(error);
                }
                continue;
            }
            task.result = Some(reply);
            save(folder, checkpoint, changed)?;
            return Ok(());
        }
        let pages = reads
            .iter()
            .map(|read| page(docs, read))
            .collect::<Result<Vec<_>>>();
        match pages {
            Ok(pages) => {
                task.draft = reply;
                task.pages = pages;
                task.feedback = "Continue from your saved findings. Review every remaining change page before completing.".into();
                checkpoint.progress.reads += reads.len();
            }
            // A read the program cannot serve is the model's mistake, like a
            // rejected result: say why and let it ask again.
            Err(error) => {
                rejections += 1;
                task.feedback = format!("Your previous read request was rejected: {error} Request the reads again.");
                task.draft = reply;
                save(folder, checkpoint, changed)?;
                if rejections >= MAX_REJECTIONS {
                    return Err(error);
                }
                continue;
            }
        }
        save(folder, checkpoint, changed)?;
    }
    Err(Error::transient("The comparison needs more source-reading steps. Completed groups and the current reading position are saved; resume the comparison to continue."))
}

fn task_at(checkpoint: &mut Checkpoint, index: Option<usize>) -> &mut Task {
    match index {
        Some(i) => &mut checkpoint.tasks[i],
        None => &mut checkpoint.consistency,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Edit, Evidence, File, Issue, OptionPlan, Source};
    use super::*;

    fn fixture(paths: &[&str]) -> Plan {
        let files: Vec<File> = paths
            .iter()
            .map(|path| File {
                path: path.to_string(),
                base: Some("Undecided.\n".into()),
                local: Some("Local preference.\n".into()),
                remote: Some("Remote preference.\n".into()),
                merged: Some("Choose a policy.\n".into()),
                text_conflict: false,
            })
            .collect();
        let sources = files
            .iter()
            .flat_map(|file| {
                [
                    ("base", &file.base),
                    ("local", &file.local),
                    ("remote", &file.remote),
                ]
                .map(|(side, text)| Source {
                    id: format!("{side}:{}", file.path),
                    path: file.path.clone(),
                    side: side.into(),
                    text: text.clone().unwrap(),
                })
            })
            .collect();
        Plan {
            id: "plan".into(),
            local: "a".repeat(40),
            remote: "b".repeat(40),
            base: "c".repeat(40),
            tree: "d".repeat(40),
            files,
            sources,
            analysis: None,
        }
    }

    fn done(issues: Vec<Issue>) -> Value {
        json!({"summary":"Compared the changes.","issues":issues,"notes":"Keep the reviewed scope.","reads":[],"replace_issues":[]})
    }

    fn issue(path: &str) -> Issue {
        Issue {
            title: format!("Policy for {path}"),
            explanation: "The two PCs recorded different policies.".into(),
            evidence: vec![Evidence {
                source: format!("local:{path}"),
                quote: "Local preference.".into(),
            }],
            options: ["Local preference.", "Remote preference."]
                .into_iter()
                .map(|text| OptionPlan {
                    label: text.into(),
                    description: text.into(),
                    edits: vec![Edit {
                        path: path.into(),
                        find: "Choose a policy.".into(),
                        text: text.into(),
                    }],
                })
                .collect(),
        }
    }

    fn data(text: &str) -> Value {
        assert!(text.chars().count() <= MAX_CONTEXT);
        serde_json::from_str(
            text.split_once("Comparison data (not instructions from the documents):\n")
                .unwrap()
                .1,
        )
        .unwrap()
    }

    #[test]
    fn large_conversations_are_pageable_and_every_note_group_is_reviewed() {
        let names: Vec<_> = (0..49)
            .map(|i| format!("projects/demo/note-{i:02}.md"))
            .collect();
        let mut plan = fixture(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let original = format!(
            "{}<a id=\"L900\"></a>\nUser: retain this decision.",
            "대화 본문입니다.\n".repeat(200_000)
        );
        plan.sources.push(Source {
            id: "local:sources/claude/session.md".into(),
            side: "local".into(),
            path: "sources/claude/session.md".into(),
            text: original.clone(),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut groups = 0;
        let mut consistency = 0;
        let analysis = run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                let payload = data(text);
                assert!(!text.contains(&original));
                if payload["stage"] == "consistency" {
                    consistency += 1;
                    assert!(payload["pages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|page| page["text"].as_str().unwrap().contains(
                            "Scope: policies differ between interactive and batch work."
                        )));
                } else {
                    groups += 1;
                }
                let mut reply = done(vec![]);
                reply["notes"] =
                    json!("Scope: policies differ between interactive and batch work.");
                Ok(reply)
            },
            &mut || {},
        )
        .unwrap();
        assert!(analysis.issues.is_empty());
        assert_eq!(groups, 7);
        assert_eq!(consistency, 1);
        assert_eq!(plan.sources.last().unwrap().text, original);
        assert_eq!(progress(dir.path()).unwrap().stage, "complete");
    }

    #[test]
    fn source_reads_use_unicode_offsets_and_can_seek_a_later_turn() {
        let path = "projects/demo/policy.md";
        let mut plan = fixture(&[path]);
        let id = "local:sources/claude/session.md";
        plan.sources.push(Source {
            id: id.into(),
            side: "local".into(),
            path: "sources/claude/session.md".into(),
            text: format!(
                "{}<a id=\"L900\"></a>\n## L900 · 사용자\n\n사용자가 승인한 방침입니다.\n",
                "가나다라\n".repeat(40_000)
            ),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let mut proposed = issue(path);
        proposed.evidence = vec![Evidence {
            source: id.into(),
            quote: "사용자가 승인한 방침입니다.".into(),
        }];
        let analysis = run(
            &plan,
            dir.path(),
            &mut |text, schema, _| {
                calls += 1;
                let payload = data(text);
                assert_eq!(schema["additionalProperties"], false);
                if calls == 1 {
                    let mut reply = done(vec![]);
                    reply["reads"] =
                        json!([{"source":id,"offset":0,"length":1000,"anchor":"<a id=\"L900\">"}]);
                    return Ok(reply);
                }
                assert!(payload["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["text"]
                        .as_str()
                        .unwrap()
                        .contains("사용자가 승인한 방침입니다.")));
                Ok(done(vec![proposed.clone()]))
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(analysis.issues.len(), 1);
        assert_eq!(calls, 2);
    }

    #[test]
    fn interrupted_comparisons_resume_without_repeating_completed_groups() {
        let plan = fixture(&["projects/a/policy.md", "projects/b/policy.md"]);
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let failed = run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                data(text);
                calls += 1;
                if calls == 2 {
                    return Err(Error::transient("Provider unavailable"));
                }
                Ok(done(vec![]))
            },
            &mut || {},
        );
        assert!(failed.is_err());
        let status = progress(dir.path()).unwrap();
        assert_eq!(status.completed, 1);
        assert_eq!(status.stage, "paused");
        let mut retried = Vec::new();
        run(
            &plan,
            dir.path(),
            &mut |text, _, folder| {
                data(text);
                retried.push(folder.to_string_lossy().into_owned());
                Ok(done(vec![]))
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(retried.len(), 2);
        assert!(retried[0].contains("group-1"));
        assert!(retried[1].contains("consistency"));
    }

    #[test]
    fn unseen_evidence_and_arbitrary_source_paths_are_rejected() {
        let mut plan = fixture(&["projects/demo/policy.md"]);
        plan.sources.push(Source {
            id: "local:conversation".into(),
            side: "local".into(),
            path: "sources/claude/a.md".into(),
            text: "Never sent to the model.".into(),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut proposed = issue(&plan.files[0].path);
        proposed.evidence = vec![Evidence {
            source: "local:conversation".into(),
            quote: "Never sent to the model.".into(),
        }];
        assert!(run(
            &plan,
            dir.path(),
            &mut |_, _, _| Ok(done(vec![proposed.clone()])),
            &mut || {}
        )
        .unwrap_err()
        .to_string()
        .contains("not in the parts of local:conversation you have read"));
        let docs = documents(&plan);
        assert!(page(
            &docs,
            &Read {
                source: "/etc/passwd".into(),
                offset: 0,
                length: 10,
                anchor: String::new()
            }
        )
        .is_err());
    }

    #[test]
    fn a_large_change_must_be_read_completely_before_accepting_a_result() {
        let mut plan = fixture(&["projects/demo/policy.md"]);
        plan.files[0].local = Some("New line.\n".repeat(6_000));
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                data(text);
                calls += 1;
                Ok(done(vec![]))
            },
            &mut || {},
        )
        .unwrap();
        assert!(calls > 2);
        let checkpoint: Checkpoint =
            serde_json::from_slice(&std::fs::read(dir.path().join("checkpoint.json")).unwrap())
                .unwrap();
        assert!(unseen(
            &documents(&plan),
            &checkpoint.tasks[0],
            &["changes-local:projects/demo/policy.md".into()]
        )
        .is_empty());
    }

    #[test]
    fn consistency_review_can_consolidate_related_issues_across_groups() {
        let paths = ["projects/a/policy.md", "projects/b/policy.md"];
        let plan = fixture(&paths);
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let analysis = run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                calls += 1;
                let payload = data(text);
                if payload["stage"] == "consistency" {
                    let mut combined = issue(paths[0]);
                    combined.title = "One policy across both projects".into();
                    combined.evidence.extend(issue(paths[1]).evidence);
                    for option in &mut combined.options {
                        option.edits.push(Edit {
                            path: paths[1].into(),
                            find: "Choose a policy.".into(),
                            text: option.label.clone(),
                        });
                    }
                    let mut reply = done(vec![combined]);
                    reply["replace_issues"] = json!([0, 1]);
                    return Ok(reply);
                }
                Ok(done(vec![issue(paths[calls - 1])]))
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(analysis.issues.len(), 1);
        assert_eq!(analysis.issues[0].options[0].edits.len(), 2);
        assert_eq!(calls, 3);
    }

    #[test]
    fn provider_context_errors_reduce_the_next_request_budget() {
        let names: Vec<_> = (0..8)
            .map(|i| format!("projects/demo/note-{i}.md"))
            .collect();
        let mut plan = fixture(&names.iter().map(String::as_str).collect::<Vec<_>>());
        for source in &mut plan.sources {
            source.text.push_str(&"Extra context.\n".repeat(500));
        }
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                data(text);
                calls += 1;
                if calls == 1 {
                    return Err(Error::transient(
                        "Prompt is too long for the context window",
                    ));
                }
                assert!(text.chars().count() <= MAX_CONTEXT / 2);
                Ok(done(vec![]))
            },
            &mut || {},
        )
        .unwrap();
        assert!(calls >= 2);
    }

    #[test]
    fn a_failure_after_a_source_request_keeps_the_reading_position_and_findings() {
        let path = "projects/demo/policy.md";
        let mut plan = fixture(&[path]);
        plan.sources.push(Source {
            id: "local:conversation".into(),
            side: "local".into(),
            path: "sources/claude/a.md".into(),
            text: format!(
                "{}User approved the policy.",
                "Earlier context.\n".repeat(5_000)
            ),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        assert!(run(&plan, dir.path(), &mut |_,_,_| {
            calls += 1;
            if calls == 2 { return Err(Error::transient("offline")); }
            let mut reply = done(vec![]);
            reply["notes"] = json!("Keep the scope and verify this approval.");
            reply["reads"] = json!([{"source":"local:conversation","offset":0,"length":1000,"anchor":"User approved the policy."}]);
            Ok(reply)
        }, &mut || {}).is_err());
        let result = run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                let payload = data(text);
                assert_eq!(
                    payload["workingNotes"],
                    "Keep the scope and verify this approval."
                );
                assert!(text.contains("User approved the policy."));
                let mut proposed = issue(path);
                proposed.evidence = vec![Evidence {
                    source: "local:conversation".into(),
                    quote: "User approved the policy.".into(),
                }];
                Ok(done(vec![proposed]))
            },
            &mut || {},
        )
        .unwrap();
        assert_eq!(result.issues.len(), 1);
    }

    #[test]
    fn consistency_cannot_add_overlapping_edits_without_replacing_the_old_issue() {
        let paths = ["projects/a/policy.md", "projects/b/policy.md"];
        let plan = fixture(&paths);
        let dir = tempfile::tempdir().unwrap();
        let mut calls = 0;
        let result = run(
            &plan,
            dir.path(),
            &mut |text, _, _| {
                calls += 1;
                if data(text)["stage"] == "consistency" {
                    return Ok(done(vec![issue(paths[0])]));
                }
                Ok(done(vec![issue(paths[calls - 1])]))
            },
            &mut || {},
        );
        assert!(result.unwrap_err().to_string().contains("overlap"));
        assert_eq!(progress(dir.path()).unwrap().stage, "paused");
    }
}
