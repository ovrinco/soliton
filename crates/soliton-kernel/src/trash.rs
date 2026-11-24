//! Recoverable planner deletion. Object bytes and Archive are never changed.
use crate::{Entity, EntityKind, GovernedRepository, KernelError, ObjectId, Result, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const RETENTION: i64 = 30 * 86400;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashEntry {
    pub kind: String,
    pub id: String,
    /// Unix seconds in UTC.
    pub deleted_at: i64,
}
#[derive(Default)]
pub struct TrashIndex {
    entries: Vec<TrashEntry>,
    purged: Vec<TrashEntry>,
}
fn fault(message: impl Into<String>) -> KernelError {
    KernelError::InvalidObject(message.into())
}
fn object_kind(kind: &str) -> Result<&'static str> {
    match kind {
        "goal" => Ok("aim"),
        "task" => Ok("task"),
        "category" => Ok("category"),
        _ => Err(fault("Choose a goal, task or category for Trash.")),
    }
}
fn check_entry(entry: &TrashEntry) -> Result<ObjectId> {
    let id: ObjectId = entry
        .id
        .parse()
        .map_err(|_| fault("The Trash item identifier is invalid."))?;
    if id.kind() != object_kind(&entry.kind)? || entry.deleted_at < 0 {
        return Err(fault("The Trash item kind or deletion time is invalid."));
    }
    Ok(id)
}
pub(crate) fn safe(path: &Path) -> Result<()> {
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(KernelError::PathDenied(
                    "Trash storage must not use symbolic links.".into(),
                ));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub(crate) fn load(root: &Path, name: &str) -> Result<Vec<TrashEntry>> {
    let path = root.join("trash").join(name);
    safe(&path)?;
    let data = match fs::read(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    if data.len() > 16 * 1024 * 1024 {
        return Err(fault("Trash index is too large."));
    }
    let entries: Vec<TrashEntry> = serde_json::from_slice(&data)
        .map_err(|_| fault("Trash index cannot be read. Keep the file for recovery."))?;
    let mut seen = BTreeSet::new();
    for entry in &entries {
        let id = check_entry(entry)?;
        if !seen.insert(id) {
            return Err(fault("Trash index contains a repeated item."));
        }
    }
    Ok(entries)
}
pub(crate) fn write<T: Serialize + ?Sized>(root: &Path, name: &str, entries: &T) -> Result<()> {
    let dir = root.join("trash");
    safe(&dir)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    safe(&path)?;
    let temp = dir.join(format!("{}.pending", ObjectId::mint("document")?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let bytes = serde_json::to_vec(entries).map_err(|_| fault("Trash index cannot be saved."))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, &path)?;
    File::open(dir)?.sync_all()?;
    Ok(())
}
impl TrashIndex {
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            entries: load(root, "index.json")?,
            purged: load(root, "purged.json")?,
        })
    }
    pub fn entries(&self) -> &[TrashEntry] {
        &self.entries
    }
    /// Durable purge plan, used by repository recovery before reading objects.
    pub(crate) fn purged_ids(root: &Path) -> Result<BTreeSet<ObjectId>> {
        load(root, "purged.json")?.iter().map(check_entry).collect()
    }
    pub fn contains(&self, entity: &Entity, snapshot: &Snapshot) -> bool {
        self.entries
            .iter()
            .any(|entry| follows(entity, &entry.id, snapshot))
    }
    pub fn trash(
        &mut self,
        root: &Path,
        snapshot: &Snapshot,
        kind: &str,
        id: &str,
        now: i64,
    ) -> Result<()> {
        let entry = TrashEntry {
            kind: kind.into(),
            id: id.into(),
            deleted_at: now,
        };
        let parsed = check_entry(&entry)?;
        let stored = snapshot
            .get(&parsed)
            .ok_or_else(|| fault("This item is no longer available."))?;
        if self.contains(&stored.entity, snapshot) {
            return Err(fault("This item is already in Trash."));
        }
        // Retain bytes before the delete is admitted, including repositories
        // from before immutable version history was introduced.
        let ids = snapshot
            .entities()
            .filter(|e| follows(e, id, snapshot))
            .map(|e| e.id().clone())
            .collect();
        GovernedRepository::open(root)?.retain_trash_versions(&ids)?;
        let mut next = self.entries.clone();
        next.push(entry);
        write(root, "index.json", &next)?;
        self.entries = next;
        Ok(())
    }
    pub fn restore(&mut self, root: &Path, kind: &str, id: &str) -> Result<()> {
        let probe = TrashEntry {
            kind: kind.into(),
            id: id.into(),
            deleted_at: 0,
        };
        check_entry(&probe)?;
        if !self.entries.iter().any(|e| e.id == id && e.kind == kind) {
            return Err(fault(
                "This item is not in Trash or its recovery window has ended.",
            ));
        }
        let roots: Vec<_> = load(root, "recovery-roots.json")?
            .into_iter()
            .filter(|entry| entry.id != id)
            .collect();
        write(root, "recovery-roots.json", &roots)?;
        let next = self
            .entries
            .iter()
            .filter(|e| e.id != id)
            .cloned()
            .collect::<Vec<_>>();
        write(root, "index.json", &next)?;
        self.entries = next;
        Ok(())
    }
    pub fn purge(
        &mut self,
        repo: &GovernedRepository,
        snapshot: &Snapshot,
        now: i64,
    ) -> Result<()> {
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter(|e| now.saturating_sub(e.deleted_at) >= RETENTION)
            .collect();
        let removing: BTreeSet<_> = snapshot
            .entities()
            .filter(|e| expired.iter().any(|t| follows(e, &t.id, snapshot)))
            .map(|e| e.id().clone())
            .collect();
        // Never remove an unrelated dependent or leave an invalid repository.
        for entity in snapshot.entities().filter(|e| !removing.contains(e.id())) {
            let targets: Vec<_> = match entity {
                Entity::Aim(a) => a.category_id.iter().collect(),
                Entity::Task(t) => t.category_id.iter().chain(t.aim_id.iter()).collect(),
                Entity::Reminder(r) => r.related_to.iter().collect(),
                Entity::Routine(r) => r.category_id.iter().collect(),
                _ => Vec::new(),
            };
            if targets.iter().any(|id| removing.contains(*id)) {
                return Err(fault(
                    "An expired Trash item still has linked work. Restore it and move the linked work before deleting it again.",
                ));
            }
        }
        let mut next_purged = self.purged.clone();
        for id in &removing {
            if next_purged.iter().any(|e| e.id == id.as_str()) {
                continue;
            }
            let kind = match id.kind() {
                "aim" => "goal",
                "task" => "task",
                "category" => "category",
                _ => return Err(fault("Only planner items may expire from Trash.")),
            };
            next_purged.push(TrashEntry {
                kind: kind.into(),
                id: id.to_string(),
                deleted_at: now,
            });
        }
        // Archive explicit deletions separately from inherited descendants.
        // This is saved before the durable purge so interruption keeps intent.
        let mut roots = load(repo.root(), "recovery-roots.json")?;
        for entry in &self.entries {
            if !roots.iter().any(|old| old.id == entry.id) {
                roots.push(entry.clone());
            }
        }
        write(repo.root(), "recovery-roots.json", &roots)?;
        if next_purged != self.purged {
            write(repo.root(), "purged.json", &next_purged)?;
            self.purged = next_purged;
        }
        // Recovery completes this durable plan, including journal bookkeeping.
        repo.recover()?;
        let next = self
            .entries
            .iter()
            // A younger independently deleted descendant keeps its own Trash
            // window even when an ancestor has expired into Recovery.
            .filter(|e| now.saturating_sub(e.deleted_at) < RETENTION)
            .cloned()
            .collect::<Vec<_>>();
        if next != self.entries {
            write(repo.root(), "index.json", &next)?;
            self.entries = next;
        }
        Ok(())
    }
}
/// Dynamic cascades preserve independent deletions: restoring an ancestor
/// removes its own entry only. A child's own Trash entry continues to hide it.
pub(crate) fn follows(entity: &Entity, root: &str, snapshot: &Snapshot) -> bool {
    if entity.id().as_str() == root {
        return true;
    }
    match entity {
        Entity::Aim(a) => a.category_id.as_ref().is_some_and(|id| id.as_str() == root),
        Entity::Task(t) => {
            t.category_id.as_ref().is_some_and(|id| id.as_str() == root)
                || t.aim_id.as_ref().is_some_and(|id| {
                    id.as_str() == root
                        || snapshot
                            .get(id)
                            .is_some_and(|goal| follows(&goal.entity, root, snapshot))
                })
        }
        _ => false,
    }
}

impl GovernedRepository {
    pub fn trash_purge_path(&self, id: &ObjectId) -> Result<PathBuf> {
        let kind = match id.kind() {
            "aim" => EntityKind::Aim,
            "task" => EntityKind::Task,
            "category" => EntityKind::Category,
            _ => return Err(fault("Only planner items may expire from Trash.")),
        };
        let path = self
            .root()
            .join("objects")
            .join(kind.as_str())
            .join(format!("{id}.soliton"));
        safe(&path)?;
        Ok(path)
    }
}

/// Records outside the planner that a removal moves to the Trash: a kitchen
/// profile entry and a collection source. Their bytes are kept beside the
/// index; after the retention window an entry moves to Recovery, where it can
/// still be restored.
pub const RECORD_KINDS: [&str; 2] = ["kitchen-profile", "collection-source"];
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordEntry {
    pub kind: String,
    pub id: String,
    pub title: String,
    /// Unix seconds in UTC.
    pub deleted_at: i64,
}
#[derive(Default)]
pub struct RecordTrash {
    root: PathBuf,
    entries: Vec<RecordEntry>,
    expired: Vec<RecordEntry>,
}
#[must_use]
pub fn is_record_kind(kind: &str) -> bool {
    RECORD_KINDS.contains(&kind)
}
fn check_record(kind: &str, id: &str) -> Result<()> {
    if !is_record_kind(kind) {
        return Err(fault("This kind of item is not kept in Trash."));
    }
    if id.is_empty()
        || id.len() > 200
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(fault("The Trash item identifier is invalid."));
    }
    Ok(())
}
fn load_records(root: &Path, name: &str) -> Result<Vec<RecordEntry>> {
    let path = root.join("trash").join(name);
    safe(&path)?;
    let data = match fs::read(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    if data.len() > 16 * 1024 * 1024 {
        return Err(fault("Trash index is too large."));
    }
    let entries: Vec<RecordEntry> = serde_json::from_slice(&data)
        .map_err(|_| fault("Trash index cannot be read. Keep the file for recovery."))?;
    for entry in &entries {
        check_record(&entry.kind, &entry.id)?;
    }
    Ok(entries)
}
impl RecordTrash {
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.to_path_buf(),
            entries: load_records(root, "records.json")?,
            expired: load_records(root, "records-expired.json")?,
        })
    }
    /// Records in their Trash window, oldest first.
    pub fn entries(&self) -> &[RecordEntry] {
        &self.entries
    }
    /// Records past the window, kept for Recovery.
    pub fn expired(&self) -> &[RecordEntry] {
        &self.expired
    }
    fn payload_path(&self, kind: &str, id: &str) -> Result<PathBuf> {
        check_record(kind, id)?;
        let path = self
            .root
            .join("trash")
            .join("records")
            .join(kind)
            .join(format!("{id}.bin"));
        safe(&path)?;
        Ok(path)
    }
    /// Keeps the record's bytes, then lists it. Called before the record
    /// leaves its own store, so a removal never loses it.
    pub fn put(&mut self, kind: &str, id: &str, title: &str, bytes: &[u8], now: i64) -> Result<()> {
        let path = self.payload_path(kind, id)?;
        if self.entries.iter().any(|e| e.kind == kind && e.id == id) {
            return Err(fault("This item is already in Trash."));
        }
        let dir = path.parent().expect("payload has a folder");
        fs::create_dir_all(dir)?;
        // An older removal of the same record, past its window, keeps its
        // bytes on disk under its deletion time; the newer removal is listed.
        if let Some(older) = self.expired.iter().find(|e| e.kind == kind && e.id == id) {
            fs::rename(&path, dir.join(format!("{id}.{}.bin", older.deleted_at)))?;
            let expired: Vec<_> = self
                .expired
                .iter()
                .filter(|e| !(e.kind == kind && e.id == id))
                .cloned()
                .collect();
            write(&self.root, "records-expired.json", &expired)?;
            self.expired = expired;
        }
        let temp = dir.join(format!("{}.pending", ObjectId::mint("document")?));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        File::open(dir)?.sync_all()?;
        let mut next = self.entries.clone();
        next.push(RecordEntry {
            kind: kind.into(),
            id: id.into(),
            title: title.into(),
            deleted_at: now,
        });
        write(&self.root, "records.json", &next)?;
        self.entries = next;
        Ok(())
    }
    /// The entry, in Trash or in Recovery.
    pub fn find(&self, kind: &str, id: &str) -> Option<&RecordEntry> {
        self.entries
            .iter()
            .chain(&self.expired)
            .find(|e| e.kind == kind && e.id == id)
    }
    /// The kept bytes of a listed record.
    pub fn bytes(&self, kind: &str, id: &str) -> Result<Vec<u8>> {
        if self.find(kind, id).is_none() {
            return Err(fault("This item is not in Trash or Recovery."));
        }
        Ok(fs::read(self.payload_path(kind, id)?)?)
    }
    /// Unlists a record once its own store holds it again. The kept bytes
    /// stay until the same record is removed again.
    pub fn restored(&mut self, kind: &str, id: &str) -> Result<()> {
        let keep = |list: &[RecordEntry]| {
            list.iter()
                .filter(|e| !(e.kind == kind && e.id == id))
                .cloned()
                .collect::<Vec<_>>()
        };
        let (entries, expired) = (keep(&self.entries), keep(&self.expired));
        if expired != self.expired {
            write(&self.root, "records-expired.json", &expired)?;
            self.expired = expired;
        }
        if entries != self.entries {
            write(&self.root, "records.json", &entries)?;
            self.entries = entries;
        }
        Ok(())
    }
    /// Moves records past the retention window from Trash to Recovery.
    pub fn expire(&mut self, now: i64) -> Result<()> {
        let (old, young): (Vec<_>, Vec<_>) = self
            .entries
            .iter()
            .cloned()
            .partition(|e| now.saturating_sub(e.deleted_at) >= RETENTION);
        if old.is_empty() {
            return Ok(());
        }
        let mut expired = self.expired.clone();
        expired.extend(old);
        // Recovery is written first, so an interruption lists an item twice
        // rather than losing it.
        write(&self.root, "records-expired.json", &expired)?;
        self.expired = expired;
        write(&self.root, "records.json", &young)?;
        self.entries = young;
        Ok(())
    }
}
