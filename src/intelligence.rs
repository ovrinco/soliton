//! Source-pinned proposals. Proposal and approval intent use the existing
//! journaled repository. The deterministic task ID reconciles a crash between
//! task commitment and the receipt update without a second planner writer.
//!
//! A proposal's checked reference (the public locator of its passage, checked
//! against the archived origin) is kept beside the record in
//! `intelligence/references/`, never in the record's body: the body stays the
//! exact shape the previous release reads, so a rollback still opens it.
use super::{Service, duration, invalid, meta, s};
use crate::{Result, time};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use soliton_kernel::*;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_ORIGIN_BYTES: u64 = 64 * 1024 * 1024;

fn youtube_id(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.");
    let id = match host {
        "youtu.be" => path.split(['?', '&', '#', '/']).next()?,
        "youtube.com" => {
            if let Some(query) = path.strip_prefix("watch?") {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("v="))?
                    .split('#')
                    .next()?
            } else {
                path.strip_prefix("shorts/")
                    .or_else(|| path.strip_prefix("live/"))?
                    .split(['?', '&', '#', '/'])
                    .next()?
            }
        }
        _ => return None,
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
    .then(|| id.to_owned())
}
fn canonical_reference(url: &str) -> Option<(String, Option<String>)> {
    let url = url.trim();
    if let Some(video) = youtube_id(url) {
        return Some((
            format!("https://www.youtube.com/watch?v={video}"),
            Some(video),
        ));
    }
    ((url.starts_with("https://") || url.starts_with("http://")) && url.len() <= 2048)
        .then(|| (url.to_owned(), None))
}
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| invalid("Archived reference evidence is missing"))?;
    if !metadata.is_file() || metadata.len() > MAX_ORIGIN_BYTES {
        return Err(invalid(
            "Archived reference evidence is not a bounded regular file",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_ORIGIN_BYTES + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn write_atomic(path: &Path, value: &Value) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| invalid("Reference folder is missing"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("reference")
    ));
    {
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}
fn epoch_date(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}
fn stage_rank(stage: &str) -> usize {
    [
        "collected",
        "transcribed",
        "waiting",
        "analysed",
        "not relevant",
        "proposed",
        "approved",
        "routed",
    ]
    .iter()
    .position(|s| *s == stage)
    .unwrap_or(99)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    schema: u32,
    #[serde(default)]
    request_id: String,
    source_id: String,
    source_revision: String,
    segment_id: String,
    excerpt: String,
    source_title: String,
    title: String,
    rationale: String,
    goal_id: String,
    goal_hash: String,
    date: String,
    schedule: Value,
    status: String,
    op_id: String,
    task_id: String,
    receipt: Value,
    allow_overlap: bool,
}

fn revision(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
fn operation_id() -> Result<String> {
    let id = ObjectId::mint("operation")?.to_string();
    let mut h = id.split_once('_').unwrap().1.to_owned();
    h.replace_range(12..13, "4");
    h.replace_range(16..17, "8");
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

impl Service {
    fn reference_path(&self, proposal_id: &str) -> PathBuf {
        self.repo
            .root()
            .join("intelligence/references")
            .join(format!(
                "{}.json",
                ContentHash::digest(proposal_id.as_bytes()).short_hex(32)
            ))
    }
    fn sidecar(&self, proposal_id: &str) -> Value {
        std::fs::read(self.reference_path(proposal_id))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|v| v["proposal_id"] == proposal_id)
            .unwrap_or(Value::Null)
    }
    fn reference_load(&self, proposal_id: &str) -> Value {
        self.sidecar(proposal_id)["reference"].clone()
    }
    /// Keeps the first moment the proposal was made across revisions.
    fn reference_save(&self, proposal_id: &str, reference: &Value) -> Result<()> {
        let proposed_at = match self.sidecar(proposal_id)["proposed_at"].as_str() {
            Some(at) => at.to_owned(),
            None => Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        write_atomic(
            &self.reference_path(proposal_id),
            &json!({"schema":1,"proposal_id":proposal_id,"proposed_at":proposed_at,"reference":reference}),
        )
    }
    /// Checks a public reference for one archived passage: against the
    /// source's own URL when it kept one, otherwise against the origin file
    /// recorded in the archive, whose hash must still match and whose same
    /// segment must carry the same public URL. Stale or mismatched evidence
    /// is refused, never repaired.
    fn reference_check(&self, source: &Value, segment: &Value, url: &str) -> Result<Value> {
        let (canonical, video_id) = canonical_reference(url)
            .ok_or_else(|| invalid("A reference must be a public http(s) URL"))?;
        let start_ms = segment["start_ms"].as_i64();
        let locator = match (&video_id, start_ms) {
            (Some(_), Some(ms)) => format!("{canonical}&t={}s", ms / 1000),
            _ => canonical.clone(),
        };
        let checked = |against: Value| {
            json!({"url":canonical,"video_id":video_id,"locator":locator,"start_ms":start_ms,
                "checked_against":against,"checked_at":Utc::now().to_rfc3339()})
        };
        if let Some(own) = source["url"].as_str().filter(|u| !u.is_empty()) {
            return match canonical_reference(own) {
                Some((archived, _)) if archived == canonical => {
                    Ok(checked(json!({"kind":"archived_source_url","url":own})))
                }
                _ => Err(invalid(format!(
                    "Reference does not match the archived source URL ({own})"
                ))),
            };
        }
        let raw_id = &segment["raw_id"];
        let archive = PathBuf::from(s(source, "archive_path"));
        if raw_id.is_null() || !archive.is_absolute() || !archive.is_dir() {
            return Err(invalid(
                "Reference cannot be checked: the archived source keeps no URL or origin segment",
            ));
        }
        let mut mismatch = None;
        for entry in std::fs::read_dir(&archive)?.take(16) {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let Ok(normalized) = serde_json::from_slice::<Value>(&read_bounded(&entry.path())?)
            else {
                continue;
            };
            for origin in normalized["source_refs"].as_array().into_iter().flatten() {
                let (Some(path), Some(hash)) =
                    (origin["path"].as_str(), origin["origin_id"].as_str())
                else {
                    continue;
                };
                if !hash.starts_with("sha256:") {
                    continue;
                }
                let bytes = read_bounded(Path::new(path))?;
                if ContentHash::digest(&bytes).to_string() != hash {
                    return Err(invalid(format!(
                        "Reference evidence is stale: the origin file changed since archiving ({path})"
                    )));
                }
                let original: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| invalid("Archived origin is not readable JSON"))?;
                let Some(found) = original["segments"]
                    .as_array()
                    .and_then(|ss| ss.iter().find(|seg| &seg["id"] == raw_id))
                else {
                    continue;
                };
                if found["start_ms"].as_i64() != start_ms {
                    mismatch = Some("the origin segment starts at a different time".to_owned());
                    continue;
                }
                match found["url"].as_str().and_then(canonical_reference) {
                    Some((origin_url, _)) if origin_url == canonical => {
                        return Ok(checked(
                            json!({"kind":"origin_segment","path":path,"origin_hash":hash,
                            "origin_segment_id":raw_id,"origin_title":original["title"]}),
                        ));
                    }
                    Some((origin_url, _)) => {
                        mismatch = Some(format!("the origin segment names {origin_url}"))
                    }
                    None => mismatch = Some("the origin segment carries no URL".to_owned()),
                }
            }
        }
        Err(invalid(format!(
            "Reference does not match the archived source: {}",
            mismatch.unwrap_or_else(|| "no origin segment could be checked".into())
        )))
    }
    /// One dated line per stage the source has passed, each drawn from the
    /// record that proves it: the archived record (collected; transcribed
    /// only when its text opens), the analysis job (waiting, analysed only
    /// with a result, not relevant with its reason), each proposal (proposed,
    /// approved) and the planner task an approval created (routed).
    fn intelligence_stages(&mut self, source_id: &str) -> Result<Value> {
        let source = self.history_command("history-detail", &json!({"id":source_id}))?;
        let mut lines = Vec::new();
        let imported = source["imported_at"]
            .as_str()
            .or_else(|| source["captured_at"].as_str())
            .unwrap_or("")
            .to_owned();
        lines.push(json!({"stage":"collected","at":imported,"detail":source["title"]}));
        let spoken = source["segments"].as_array().is_some_and(|segments| {
            segments
                .iter()
                .any(|seg| !s(seg, "text").trim().is_empty() && seg["start_ms"].as_i64().is_some())
        });
        if spoken {
            let count = source["segments"].as_array().map_or(0, Vec::len);
            lines.push(
                json!({"stage":"transcribed","at":imported,"detail":format!("{count} passages")}),
            );
        }
        let jobs = self.repo.root().join("briefings");
        lines.extend(crate::briefing::source_stages(&jobs, s(&source, "id"))?);
        for e in self
            .snapshot
            .entities()
            .filter(|e| matches!(e,Entity::Document(d) if d.document_type=="proposal"))
        {
            let (stored, p) = self.proposal_load(e.id().as_str())?;
            if p.source_id != s(&source, "id") {
                continue;
            }
            let id = e.id().as_str();
            let proposed = self.sidecar(id)["proposed_at"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| epoch_date(stored.entity.meta().details.created_at));
            lines.push(json!({"stage":"proposed","at":proposed,"detail":p.title,"proposal_id":id}));
            if let Some(at) = p.receipt["approved_at"].as_str() {
                lines.push(json!({"stage":"approved","at":at,"detail":p.title,"proposal_id":id}));
            }
            if let Ok(task_id) = p.task_id.parse::<ObjectId>()
                && let Some(task) = self.snapshot.get(&task_id)
            {
                let at = p.receipt["routed_at"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| epoch_date(task.entity.meta().details.created_at));
                if p.receipt["approved_at"].is_null() {
                    lines.push(
                        json!({"stage":"approved","at":at,"detail":p.title,"proposal_id":id}),
                    );
                }
                lines.push(json!({"stage":"routed","at":at,"detail":task.entity.meta().title,"task_id":task_id.as_str()}));
            }
        }
        let instant = |line: &Value| {
            chrono::DateTime::parse_from_rfc3339(s(line, "at"))
                .map(|d| d.timestamp_millis())
                .unwrap_or(i64::MAX)
        };
        lines.sort_by_key(|line| (instant(line), stage_rank(s(line, "stage"))));
        for line in &mut lines {
            if let Ok(at) = chrono::DateTime::parse_from_rfc3339(s(line, "at")) {
                line["at"] = json!(
                    at.with_timezone(&Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                );
            }
        }
        Ok(json!({"source_id":source["id"],"title":source["title"],"stages":lines}))
    }
    pub(super) fn history_related(&self, source_id: &str, source_revision: &str) -> Result<Value> {
        let mut links = Vec::new();
        for e in self
            .snapshot
            .entities()
            .filter(|e| matches!(e,Entity::Document(d) if d.document_type=="proposal"))
        {
            let (stored, p) = self.proposal_load(e.id().as_str())?;
            if p.source_id != source_id {
                continue;
            }
            let v = self.proposal_view(&stored, &p);
            links.push(json!({"id":e.id().as_str(),"kind":"proposal","title":p.title,"status":v["status"],"source_revision":p.source_revision,"stale":p.source_revision!=source_revision,"task_id":if v["status"]=="applied"{json!(p.task_id)}else{Value::Null}}));
        }
        Ok(json!(links))
    }
    fn proposal_load(&self, id: &str) -> Result<(StoredEntity, Proposal)> {
        let stored = self.get(id)?.clone();
        if !matches!(&stored.entity,Entity::Document(d) if d.document_type=="proposal") {
            return Err(invalid("This is not a proposal"));
        }
        let p: Proposal = serde_json::from_str(&stored.entity.meta().details.body)?;
        if p.schema != 1 {
            return Err(invalid("Unsupported proposal schema; original preserved"));
        }
        Ok((stored, p))
    }
    fn proposal_view(&self, stored: &StoredEntity, p: &Proposal) -> Value {
        let mut v = serde_json::to_value(p).unwrap();
        v["reference"] = self.reference_load(stored.entity.id().as_str());
        v["id"] = json!(stored.entity.id().as_str());
        v["revision"] = json!(stored.content_hash.to_string());
        v["updated_at"] = json!(stored.entity.meta().details.updated_at);
        if let Ok(id) = p.task_id.parse::<ObjectId>()
            && let Some(task) = self.snapshot.get(&id)
        {
            v["status"] = json!("applied");
            v["task"] = self.record(&task.entity);
        }
        v
    }
    fn proposal_write(&mut self, stored: StoredEntity, p: &Proposal) -> Result<Value> {
        let mut e = stored.entity;
        e.meta_mut().revision += 1;
        e.meta_mut().title = p.title.clone();
        e.meta_mut().details.body = serde_json::to_string(p)?;
        e.meta_mut().details.updated_at = Utc::now().timestamp();
        let next = self.repo.update(e, stored.content_hash)?;
        self.refresh()?;
        Ok(self.proposal_view(&next, p))
    }
    pub(super) fn intelligence_command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "proposal-draft-id" => Ok(json!({"request_id":operation_id()?})),
            "intelligence-stages" | "proposal-stages" => {
                self.intelligence_stages(s(args, "source_id"))
            }
            "intelligence-analyze" => {
                let source =
                    self.history_command("history-detail", &json!({"id":args["source_id"]}))?;
                let policy = self.source_policy_command("source-policy-evaluate", &json!({
                    "source_id": source["id"], "title": source["title"],
                    "url": source.get("url").cloned().unwrap_or_else(|| source["source_refs"][0]["url"].clone()),
                    "goal_id": args["goal_id"], "override_excluded": args["override_excluded"]
                }))?;
                if policy["include"] != true {
                    return Ok(
                        json!({"source_id":source["id"],"source_revision":source["revision"],"title":source["title"],"processor":"local-extractive/1","policy":policy,"items":[],"segments_examined":0,"segments_total":source["segments"].as_array().map_or(0,Vec::len),"matches_total":0,"notice":"Excluded by your local source preferences. You can explicitly read it without changing provider likes or recommendations."}),
                    );
                }
                let segments = source["segments"]
                    .as_array()
                    .ok_or_else(|| invalid("Source has no readable segments"))?;
                let goals: Vec<_> = self
                    .snapshot
                    .entities()
                    .filter(|e| {
                        matches!(e, Entity::Aim(_))
                            && !self.hidden(e)
                            && (s(args, "goal_id").is_empty()
                                || e.id().as_str() == s(args, "goal_id"))
                    })
                    .map(|e| self.record(e))
                    .collect();
                let mut passages = Vec::new();
                for seg in segments.iter().take(20000) {
                    let text = s(seg, "text");
                    if text.trim().is_empty() {
                        continue;
                    }
                    let lower = text.to_lowercase();
                    let mut relevant = Vec::new();
                    for goal in &goals {
                        let words: BTreeSet<_> = s(goal, "title")
                            .split(|c: char| !c.is_alphanumeric())
                            .filter(|w| w.len() >= 4)
                            .map(str::to_lowercase)
                            .filter(|word| {
                                ![
                                    "about", "after", "again", "also", "been", "being", "better",
                                    "build", "could", "daily", "from", "have", "into", "just",
                                    "keep", "make", "more", "need", "next", "only", "other",
                                    "over", "should", "some", "than", "that", "their", "them",
                                    "then", "there", "these", "they", "this", "through", "under",
                                    "using", "want", "what", "when", "where", "which", "while",
                                    "will", "with", "work", "would", "your",
                                ]
                                .contains(&word.as_str())
                            })
                            .collect();
                        let matched: Vec<_> = words
                            .into_iter()
                            .filter(|w| {
                                lower
                                    .split(|c: char| !c.is_alphanumeric())
                                    .any(|token| token == w)
                            })
                            .collect();
                        if !matched.is_empty() {
                            relevant.push(json!({"id":goal["id"],"title":goal["title"],"hash":goal["hash"],"reason":format!("Shares goal terms: {}",matched.join(", ")),"basis":"lexical_match"}));
                        }
                    }
                    let action = lower.contains("should ")
                        || lower.contains("need ")
                        || lower.contains("want ")
                        || lower.contains("build ")
                        || lower.contains("review ");
                    if action || !relevant.is_empty() {
                        passages.push(json!({"segment_id":seg["id"],"text":text,"start_ms":seg["start_ms"],"goals":relevant,"basis":"Quoted source passage; not independently verified","action_language":action}));
                    }
                }
                let total = passages.len();
                passages.truncate(100);
                Ok(
                    json!({"source_id":source["id"],"source_revision":source["revision"],"title":source["title"],"processor":"local-extractive/1","policy":policy,"coverage":source["coverage"],"segments_examined":segments.len().min(20000),"segments_total":segments.len(),"matches_total":total,"items":passages,"limited":total>100||segments.len()>20000,"notice":"Local passage selection and term matching. No model inference or independent fact checking. Review a passage before proposing work."}),
                )
            }
            "proposal-calendar" => {
                let from = time::date(s(args, "from"))?;
                let through = time::date(s(args, "through"))?;
                let tz = s(args, "timezone");
                time::zone(tz)?;
                if through < from || (through - from).num_days() > 400 {
                    return Err(invalid("Choose a calendar window of at most 401 days"));
                }
                let mut events = Vec::new();
                for entity in self
                    .snapshot
                    .entities()
                    .filter(|e| matches!(e,Entity::Document(d) if d.document_type=="proposal"))
                {
                    let (stored, p) = self.proposal_load(entity.id().as_str())?;
                    if !matches!(
                        s(&self.proposal_view(&stored, &p), "status"),
                        "ready" | "deferred" | "applying"
                    ) || p.date.is_empty()
                    {
                        continue;
                    }
                    let (start, end, kind) = if p.schedule.is_object() {
                        let sc = &p.schedule;
                        let resolved = time::scheduled(
                            &p.date,
                            s(sc, "time"),
                            duration(sc)?,
                            s(sc, "timezone"),
                            s(sc, "fold"),
                        )?;
                        (
                            time::local_string(resolved.start_utc, tz)?,
                            time::local_string(
                                resolved.start_utc + i64::from(resolved.duration_minutes) * 60,
                                tz,
                            )?,
                            "timed",
                        )
                    } else {
                        (
                            format!("{}T00:00", p.date),
                            format!(
                                "{}T00:00",
                                time::date(&p.date)?
                                    .succ_opt()
                                    .ok_or_else(|| invalid("Date out of range"))?
                            ),
                            "all_day",
                        )
                    };
                    if time::date(&start[..10])? > through || time::date(&end[..10])? < from {
                        continue;
                    }
                    events.push(json!({"proposal_id":entity.id().as_str(),"event_key":entity.id().as_str(),"title":p.title,"date":&start[..10],"local_start":start,"local_end":end,"event_kind":kind,"schedule":p.schedule,"status":p.status}));
                }
                Ok(json!(events))
            }
            "proposal-list" => {
                let status = s(args, "status");
                let mut items = Vec::new();
                for e in self
                    .snapshot
                    .entities()
                    .filter(|e| matches!(e,Entity::Document(d) if d.document_type=="proposal"))
                {
                    let (stored, p) = self.proposal_load(e.id().as_str())?;
                    let value = self.proposal_view(&stored, &p);
                    if status.is_empty() || status == "all" || s(&value, "status") == status {
                        items.push(value);
                    }
                }
                items.sort_by_key(|v| std::cmp::Reverse(v["updated_at"].as_i64().unwrap_or(0)));
                let total = items.len();
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
                Ok(
                    json!({"items":items.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":total}),
                )
            }
            "proposal-create" | "proposal-revise" => {
                let prior = if command == "proposal-revise" {
                    let (stored, p) = self.proposal_load(s(args, "id"))?;
                    if stored.content_hash.to_string() != s(args, "expected_revision") {
                        return Err(invalid("Proposal changed. Reload before editing."));
                    }
                    if !matches!(p.status.as_str(), "ready" | "deferred") {
                        return Err(invalid("Only an uncommitted proposal can be edited."));
                    }
                    Some(stored)
                } else {
                    None
                };
                let request_id = s(args, "request_id");
                if !request_id.is_empty()
                    && (request_id.len() != 36
                        || !request_id.bytes().enumerate().all(|(i, c)| {
                            if [8, 13, 18, 23].contains(&i) {
                                c == b'-'
                            } else {
                                c.is_ascii_hexdigit()
                            }
                        }))
                {
                    return Err(invalid("Invalid proposal request identity"));
                }
                let requested_id = if command == "proposal-create" && !request_id.is_empty() {
                    Some(ObjectId::derive(
                        "document",
                        format!("soliton-proposal-request/1\0{request_id}").as_bytes(),
                    )?)
                } else {
                    None
                };
                if let Some(id) = &requested_id
                    && self.snapshot.get(id).is_some()
                {
                    let (stored, p) = self.proposal_load(id.as_str())?;
                    if p.source_id != s(args, "source_id")
                        || p.source_revision != s(args, "source_revision")
                        || p.segment_id != s(args, "segment_id")
                        || p.title != s(args, "title").trim()
                        || p.rationale != s(args, "rationale")
                        || p.goal_id != s(args, "goal_id")
                        || p.date != s(args, "date")
                        || p.schedule != args["schedule"]
                        || self.reference_load(id.as_str())["url"]
                            .as_str()
                            .unwrap_or("")
                            != args["reference"]["url"]
                                .as_str()
                                .and_then(canonical_reference)
                                .map(|(url, _)| url)
                                .unwrap_or_default()
                    {
                        return Err(invalid(
                            "This proposal request was already saved with different content. Open the saved proposal to edit it.",
                        ));
                    }
                    return Ok(self.proposal_view(&stored, &p));
                }
                let title = s(args, "title").trim();
                if title.is_empty() || title.len() > 1000 {
                    return Err(invalid("Give the proposal a title of at most 1000 bytes"));
                }
                let source =
                    self.history_command("history-detail", &json!({"id":args["source_id"]}))?;
                if revision(&source["revision"]) != s(args, "source_revision") {
                    return Err(invalid(
                        "Source changed. Review the current passage before proposing work.",
                    ));
                }
                let segment = source["segments"]
                    .as_array()
                    .and_then(|ss| ss.iter().find(|seg| seg["id"] == args["segment_id"]))
                    .ok_or_else(|| invalid("Source segment no longer exists"))?;
                let goal_id = s(args, "goal_id").to_owned();
                let goal_hash = if goal_id.is_empty() {
                    String::new()
                } else {
                    let goal = self.get(&goal_id)?;
                    if !matches!(goal.entity, Entity::Aim(_)) || self.hidden(&goal.entity) {
                        return Err(invalid("Choose an active goal"));
                    }
                    goal.content_hash.to_string()
                };
                let day = s(args, "date");
                if !day.is_empty() {
                    time::date(day)?;
                }
                let schedule = if args["schedule"].is_object() {
                    let sc = &args["schedule"];
                    time::scheduled(
                        day,
                        s(sc, "time"),
                        duration(sc)?,
                        s(sc, "timezone"),
                        s(sc, "fold"),
                    )?;
                    json!({"time":sc["time"],"duration_minutes":sc["duration_minutes"],"timezone":sc["timezone"],"fold":sc["fold"]})
                } else {
                    Value::Null
                };
                if s(args, "rationale").len() > 24000 {
                    return Err(invalid("Rationale exceeds 24 KiB"));
                }
                // A revision without a new reference keeps the prior one,
                // checked again against the passage it now cites.
                let given = match &prior {
                    Some(stored) if args.get("reference").is_none() => {
                        self.reference_load(stored.entity.id().as_str())
                    }
                    _ => args["reference"].clone(),
                };
                let reference = match given["url"].as_str() {
                    Some(url) => self.reference_check(&source, segment, url)?,
                    None if given.is_null() => Value::Null,
                    None => return Err(invalid("A reference needs its public URL")),
                };
                let mut m = match &prior {
                    Some(stored) => stored.entity.meta().clone(),
                    None => meta("document", title)?,
                };
                if let Some(id) = requested_id {
                    m.id = id;
                }
                let op = operation_id()?;
                let task_id = ObjectId::derive(
                    "task",
                    format!("soliton-proposal/1\0{}\0{op}", m.id).as_bytes(),
                )?
                .to_string();
                let p = Proposal {
                    schema: 1,
                    request_id: request_id.into(),
                    source_id: s(&source, "id").into(),
                    source_revision: revision(&source["revision"]),
                    segment_id: s(segment, "id").into(),
                    excerpt: s(segment, "text").into(),
                    source_title: s(&source, "title").into(),
                    title: title.into(),
                    rationale: s(args, "rationale").into(),
                    goal_id,
                    goal_hash,
                    date: day.into(),
                    schedule,
                    status: "ready".into(),
                    op_id: op,
                    task_id,
                    receipt: Value::Null,
                    allow_overlap: false,
                };
                // The reference lands before the record that cites it, so a
                // crash between them leaves no proposal without its evidence.
                self.reference_save(m.id.as_str(), &reference)?;
                if let Some(stored) = prior {
                    return self.proposal_write(stored, &p);
                }
                m.details.body = serde_json::to_string(&p)?;
                let stored = self.repo.create(Entity::Document(Document {
                    meta: m,
                    document_type: "proposal".into(),
                }))?;
                self.refresh()?;
                Ok(self.proposal_view(&stored, &p))
            }
            "proposal-detail" => {
                let (stored, p) = self.proposal_load(s(args, "id"))?;
                Ok(self.proposal_view(&stored, &p))
            }
            "proposal-decide" => {
                let (stored, mut p) = self.proposal_load(s(args, "id"))?;
                let decision = s(args, "decision");
                // Retry of the same admitted intent is reconciliation, even
                // after the source/goal has changed since the original commit.
                if decision == "approve" && s(args, "op_id") == p.op_id {
                    if self.snapshot.get(&p.task_id.parse()?).is_some() {
                        // Already durable: a repeated reply leaves the
                        // proposal's revision and receipt as they are.
                        if p.status == "applied" && p.receipt["task_id"] == p.task_id.as_str() {
                            return Ok(self.proposal_view(&stored, &p));
                        }
                        // The task may have committed before the proposal
                        // receipt was durably written. Reconcile that
                        // unknown outcome through the same journaled writer
                        // so a retry repairs the durable proposal state too.
                        let receipt = json!({"op_id":p.op_id,"status":"applied","task_id":p.task_id,
                            "approved_at":p.receipt["approved_at"]});
                        p.status = "applied".into();
                        p.receipt = receipt;
                        return self.proposal_write(stored, &p);
                    }
                    if p.status == "applying" {
                        return self.proposal_apply(stored, p);
                    }
                }
                if stored.content_hash.to_string() != s(args, "expected_revision") {
                    return Err(invalid(
                        "Proposal changed. Reload and review before deciding.",
                    ));
                }
                if !["ready", "deferred"].contains(&p.status.as_str())
                    && !(p.status == "applying" && decision == "reject")
                {
                    return Err(invalid("This proposal is no longer awaiting a decision"));
                }
                match decision {
                    "reject" | "defer" => {
                        p.status = if decision == "reject" {
                            "rejected"
                        } else {
                            "deferred"
                        }
                        .into();
                        self.proposal_write(stored, &p)
                    }
                    "approve" => {
                        if s(args, "op_id") != p.op_id {
                            return Err(invalid(
                                "Approval operation identity does not match the proposal",
                            ));
                        }
                        let source =
                            self.history_command("history-detail", &json!({"id":p.source_id}))?;
                        if revision(&source["revision"]) != p.source_revision {
                            return Err(invalid(
                                "Source changed. Create a reviewed proposal from the current revision.",
                            ));
                        }
                        let reference = self.reference_load(s(args, "id"));
                        if let Some(url) = reference["url"].as_str() {
                            let segment = source["segments"]
                                .as_array()
                                .and_then(|ss| {
                                    ss.iter().find(|seg| seg["id"] == p.segment_id.as_str())
                                })
                                .ok_or_else(|| invalid("Source segment no longer exists"))?;
                            self.reference_check(&source, segment, url)?;
                        }
                        if !p.goal_id.is_empty()
                            && self.get(&p.goal_id)?.content_hash.to_string() != p.goal_hash
                        {
                            return Err(invalid(
                                "Goal changed. Review a new proposal against the current goal.",
                            ));
                        }
                        p.allow_overlap = args["allow_overlap"].as_bool().unwrap_or(false);
                        self.proposal_validate_capacity(&p)?;
                        // Publish the admitted intent before any planner effect.
                        p.status = "applying".into();
                        p.receipt = json!({"op_id":p.op_id,"approved_at":
                            Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)});
                        self.proposal_write(stored, &p)?;
                        let (stored, p) = self.proposal_load(s(args, "id"))?;
                        self.proposal_apply(stored, p)
                    }
                    _ => Err(invalid("Choose approve, defer or reject")),
                }
            }
            _ => Err(invalid("Unknown intelligence command")),
        }
    }
    fn proposal_validate_capacity(&self, p: &Proposal) -> Result<()> {
        if !p.schedule.is_object() || p.allow_overlap {
            return Ok(());
        }
        let sc = &p.schedule;
        let schedule = time::scheduled(
            &p.date,
            s(sc, "time"),
            duration(sc)?,
            s(sc, "timezone"),
            s(sc, "fold"),
        )?;
        let start = schedule.start_utc;
        let end = start + i64::from(schedule.duration_minutes) * 60;
        let day = time::date(&p.date)?;
        let through = (day + chrono::Duration::days(7)).to_string();
        let events = self.calendar(&p.date, &through, &schedule.timezone)?;
        if events.as_array().unwrap().iter().any(|e| {
            s(e, "event_kind") == "timed"
                && e["start_utc"].as_i64().unwrap_or(i64::MAX) < end
                && e["end_utc"].as_i64().unwrap_or(i64::MIN) > start
        }) {
            return Err(invalid(
                "This time overlaps committed work. Choose another time or explicitly allow the overlap.",
            ));
        }
        Ok(())
    }
    fn proposal_apply(&mut self, stored: StoredEntity, mut p: Proposal) -> Result<Value> {
        let task_id: ObjectId = p.task_id.parse()?;
        let mut routed_at = Value::Null;
        if self.snapshot.get(&task_id).is_none() {
            let source = self.history_command("history-detail", &json!({"id":p.source_id}))?;
            if revision(&source["revision"]) != p.source_revision {
                return Err(invalid(
                    "Source changed while approval was pending. Reject this pending intent and review a new proposal.",
                ));
            }
            // Resume after restart still checks current target/capacity. A
            // changed prerequisite leaves the admitted intent inspectable.
            if !p.goal_id.is_empty()
                && self.get(&p.goal_id)?.content_hash.to_string() != p.goal_hash
            {
                return Err(invalid(
                    "Goal changed while approval was pending; no task created.",
                ));
            }
            self.proposal_validate_capacity(&p)?;
            let mut m = meta("task", &p.title)?;
            m.id = task_id;
            m.details.links = vec![stored.entity.id().to_string()];
            m.details.tags = vec![format!("soliton-proposal-op:{}", p.op_id)];
            let reference = self.reference_load(stored.entity.id().as_str());
            let cited = reference["locator"].as_str().map_or_else(
                || p.source_title.clone(),
                |locator| format!("{} — {locator}", p.source_title),
            );
            m.details.body = format!(
                "{}\n\nSource: {}\nRevision: {}\nPassage: {}\n\n{}",
                p.rationale, cited, p.source_revision, p.segment_id, p.excerpt
            );
            m.details.schedule = if p.schedule.is_object() {
                let sc = &p.schedule;
                Some(time::scheduled(
                    &p.date,
                    s(sc, "time"),
                    duration(sc)?,
                    s(sc, "timezone"),
                    s(sc, "fold"),
                )?)
            } else {
                None
            };
            let e = Entity::Task(Task {
                meta: m,
                category_id: None,
                aim_id: if p.goal_id.is_empty() {
                    None
                } else {
                    Some(p.goal_id.parse()?)
                },
                priority: Priority::Medium,
                status: TaskStatus::Todo,
                scheduled_for: if p.date.is_empty() {
                    None
                } else {
                    Some(p.date.parse()?)
                },
                due_on: None,
                recurrence: None,
                completed_occurrences: BTreeSet::new(),
            });
            self.repo.create(e)?;
            self.refresh()?;
            routed_at = json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        }
        p.status = "applied".into();
        p.receipt = json!({"op_id":p.op_id,"status":"applied","task_id":p.task_id,
            "approved_at":p.receipt["approved_at"],"routed_at":routed_at});
        self.proposal_write(stored, &p)
    }
}
