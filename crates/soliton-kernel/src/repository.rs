use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::error::{KernelError, Result};
use crate::hash::ContentHash;
use crate::id::ObjectId;
use crate::model::{DataClass, Entity, EntityKind};
use crate::recurrence::{RecurrenceCursor, RecurrenceCursorStatus, RecurrenceRule};

const OBJECTS_DIR: &str = "objects";
const JOURNAL_DIR: &str = "journal";
const JOURNAL_FILE: &str = "operations.log";
const FILE_EXTENSION: &str = "soliton";
const RECURRENCE_DIR: &str = "recurrence";

static WRITER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn writer_lock() -> MutexGuard<'static, ()> {
    WRITER_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportAudience {
    LocalOperator,
    Guest,
    ExternalExecutor,
    KnowledgeBundle,
}

impl ExportAudience {
    #[must_use]
    pub const fn permits(self, data_class: DataClass) -> bool {
        match self {
            Self::LocalOperator => true,
            Self::Guest | Self::ExternalExecutor | Self::KnowledgeBundle => {
                data_class.exportable_to_guest()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEntity {
    pub entity: Entity,
    pub content_hash: ContentHash,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    entities: BTreeMap<ObjectId, StoredEntity>,
}

impl Snapshot {
    pub fn from_entities(entities: impl IntoIterator<Item = Entity>) -> Result<Self> {
        let stored = entities
            .into_iter()
            .map(|entity| {
                let content_hash = entity.content_hash()?;
                Ok(StoredEntity {
                    entity,
                    content_hash,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::from_stored(stored)
    }

    fn from_stored(entities: impl IntoIterator<Item = StoredEntity>) -> Result<Self> {
        let mut by_id = BTreeMap::new();
        for stored in entities {
            stored.entity.validate()?;
            let id = stored.entity.id().clone();
            if by_id.insert(id.clone(), stored).is_some() {
                return Err(KernelError::Integrity(format!("duplicate object id {id}")));
            }
        }
        validate_relationships(&by_id)?;
        Ok(Self { entities: by_id })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values().map(|stored| &stored.entity)
    }

    #[must_use]
    pub fn get(&self, id: &ObjectId) -> Option<&StoredEntity> {
        self.entities.get(id)
    }

    /// Read projection only; authority relationships remain in the full snapshot.
    pub fn without_trash(&self, trash: &crate::trash::TrashIndex) -> Self {
        Self {
            entities: self
                .entities
                .iter()
                .filter(|(_, stored)| !trash.contains(&stored.entity, self))
                .map(|(id, stored)| (id.clone(), stored.clone()))
                .collect(),
        }
    }

    pub fn exportable_entities(&self, audience: ExportAudience) -> impl Iterator<Item = &Entity> {
        self.entities()
            .filter(move |entity| audience.permits(entity.meta().data_class))
    }

    pub fn filtered_for_export(&self, audience: ExportAudience) -> Result<Self> {
        let entities = self
            .entities
            .values()
            .filter(|stored| audience.permits(stored.entity.meta().data_class))
            .cloned()
            .collect::<Vec<_>>();
        // Export packets can intentionally omit relationship targets with a
        // stronger data class, so they are not revalidated as an authority graph.
        let by_id = entities
            .into_iter()
            .map(|stored| (stored.entity.id().clone(), stored))
            .collect();
        Ok(Self { entities: by_id })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationPrecondition {
    MustNotExist,
    Matches(ContentHash),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalEntry {
    Prepared {
        operation_id: ObjectId,
        kind: EntityKind,
        object_id: ObjectId,
        before: Option<ContentHash>,
        after: ContentHash,
    },
    Committed {
        operation_id: ObjectId,
        recovered: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    pub committed: u32,
    pub already_committed: u32,
}

#[derive(Clone, Debug)]
struct PrepareRecord {
    operation_id: ObjectId,
    kind: EntityKind,
    object_id: ObjectId,
    before: Option<ContentHash>,
    after: ContentHash,
}

#[derive(Clone, Debug)]
pub struct GovernedRepository {
    root: PathBuf,
}

impl GovernedRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let requested = absolute_path(path.as_ref())?;
        deny_symlink_components(&requested)?;
        fs::create_dir_all(&requested)?;
        deny_symlink_components(&requested)?;
        let root = requested.canonicalize()?;
        let repository = Self { root };
        repository.ensure_layout()?;
        Ok(repository)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn prepare(
        &self,
        entity: Entity,
        precondition: MutationPrecondition,
    ) -> Result<PreparedMutation> {
        let _writer = writer_lock();
        self.prepare_unlocked(entity, precondition)
    }

    fn prepare_unlocked(
        &self,
        entity: Entity,
        precondition: MutationPrecondition,
    ) -> Result<PreparedMutation> {
        self.ensure_layout()?;
        entity.validate()?;
        self.validate_entity_relationships(&entity)?;
        let bytes = entity.canonical_bytes()?;
        let after = ContentHash::digest(&bytes);
        let target = self.object_path(entity.kind(), entity.id());
        let current = self.hash_if_present(&target)?;
        check_precondition(precondition, current, entity.id())?;
        let before = current;
        // Immutable versions are committed before the mutation intent. The
        // journal still decides which version is authoritative.
        let history = self.root.join("versions").join(entity.id().as_str());
        deny_symlink_components(&history)?;
        fs::create_dir_all(&history)?;
        sync_directory(&self.root)?;
        if let Some(old) = before {
            let old_bytes = read_file(&target)?;
            self.preserve_version(&history, old, &old_bytes)?;
        }
        self.preserve_version(&history, after, &bytes)?;
        let operation_id = ObjectId::mint("op")?;
        let record = PrepareRecord {
            operation_id,
            kind: entity.kind(),
            object_id: entity.id().clone(),
            before,
            after,
        };
        let stage = self.stage_path(&record);
        self.write_stage(&stage, &bytes)?;
        self.append_journal(&JournalEntry::Prepared {
            operation_id: record.operation_id.clone(),
            kind: record.kind,
            object_id: record.object_id.clone(),
            before: record.before,
            after: record.after,
        })?;
        Ok(PreparedMutation {
            repository: self.clone(),
            record,
        })
    }

    pub fn create(&self, entity: Entity) -> Result<StoredEntity> {
        // Keep the precondition and commit in one writer turn. Otherwise two
        // handles can both prepare a create before either makes it durable.
        let _writer = writer_lock();
        let mutation = self.prepare_unlocked(entity, MutationPrecondition::MustNotExist)?;
        self.install_prepared_unlocked(&mutation.record, false)
    }

    pub fn update(&self, entity: Entity, expected: ContentHash) -> Result<StoredEntity> {
        let _writer = writer_lock();
        let mutation = self.prepare_unlocked(entity, MutationPrecondition::Matches(expected))?;
        self.install_prepared_unlocked(&mutation.record, false)
    }

    pub fn recover(&self) -> Result<RecoveryReport> {
        let _writer = writer_lock();
        self.ensure_layout()?;
        self.complete_trash_restore()?;
        let purged = crate::trash::TrashIndex::purged_ids(&self.root)?;
        if !purged.is_empty() {
            self.complete_trash_purge(&purged)?;
        }
        let entries = self.journal_entries()?;
        let mut prepared = BTreeMap::<ObjectId, PrepareRecord>::new();
        let mut prepare_order = Vec::new();
        let mut committed = BTreeSet::new();
        let mut latest_committed = BTreeMap::<ObjectId, ObjectId>::new();
        for entry in entries {
            match entry {
                JournalEntry::Prepared {
                    operation_id,
                    kind,
                    object_id,
                    before,
                    after,
                } => {
                    let record = PrepareRecord {
                        operation_id: operation_id.clone(),
                        kind,
                        object_id,
                        before,
                        after,
                    };
                    if prepared.insert(operation_id.clone(), record).is_some() {
                        return Err(KernelError::Integrity(format!(
                            "operation {operation_id} has duplicate prepare records"
                        )));
                    }
                    prepare_order.push(operation_id);
                }
                JournalEntry::Committed { operation_id, .. } => {
                    if !prepared.contains_key(&operation_id) {
                        return Err(KernelError::Integrity(format!(
                            "operation {operation_id} commits before it is prepared"
                        )));
                    }
                    if !committed.insert(operation_id.clone()) {
                        return Err(KernelError::Integrity(format!(
                            "operation {operation_id} has duplicate commit records"
                        )));
                    }
                    let record = prepared.get(&operation_id).expect("checked above");
                    latest_committed.insert(record.object_id.clone(), operation_id);
                }
            }
        }

        let mut report = RecoveryReport {
            already_committed: u32::try_from(committed.len()).unwrap_or(u32::MAX),
            ..RecoveryReport::default()
        };
        // Replay in journal order. A rename can make a pending update durable
        // before its COMMIT line; recovering that operation must happen before
        // checking the older committed target as the object's final state.
        for operation_id in prepare_order {
            if committed.contains(&operation_id) {
                continue;
            }
            let record = prepared
                .get(&operation_id)
                .expect("prepare order is complete");
            self.install_prepared_unlocked(record, true)?;
            committed.insert(operation_id.clone());
            latest_committed.insert(record.object_id.clone(), operation_id);
            report.committed = report.committed.saturating_add(1);
        }
        for (object_id, operation_id) in latest_committed {
            let record = prepared
                .get(&operation_id)
                .expect("latest committed operation has a prepare record");
            let target = self.object_path(record.kind, &object_id);
            if self.hash_if_present(&target)? != Some(record.after) {
                return Err(KernelError::Integrity(format!(
                    "committed operation {operation_id} has no matching durable target"
                )));
            }
        }
        Ok(report)
    }

    // The durable Trash purge plan makes interruption recoverable. Retire
    // only the expired objects' journal pairs so unchanged r8 can also reopen
    // after a completed purge. Object history and other operations remain.
    fn complete_trash_purge(&self, ids: &BTreeSet<ObjectId>) -> Result<()> {
        let entries = self.journal_entries()?;
        let retired: BTreeSet<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                JournalEntry::Prepared {
                    operation_id,
                    object_id,
                    ..
                } if ids.contains(object_id) => Some(operation_id.clone()),
                _ => None,
            })
            .collect();
        self.archive_trash_versions(ids, &entries)?;
        for id in ids {
            let path = self.trash_purge_path(id)?;
            match fs::remove_file(&path) {
                Ok(()) => {
                    File::open(path.parent().expect("object path has a parent"))?.sync_all()?
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
        if !retired.is_empty() {
            let remaining = entries
                .iter()
                .filter(|entry| {
                    let operation_id = match entry {
                        JournalEntry::Prepared { operation_id, .. }
                        | JournalEntry::Committed { operation_id, .. } => operation_id,
                    };
                    !retired.contains(operation_id)
                })
                .map(render_journal_entry)
                .collect::<String>();
            let path = self.root.join(JOURNAL_DIR).join(JOURNAL_FILE);
            let stage = self
                .root
                .join(JOURNAL_DIR)
                .join(format!("{}.pending", ObjectId::mint("document")?));
            deny_symlink_components(&path)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)?;
            file.write_all(remaining.as_bytes())?;
            file.sync_all()?;
            fs::rename(&stage, &path)?;
            File::open(self.root.join(JOURNAL_DIR))?.sync_all()?;
        }
        Ok(())
    }

    pub(crate) fn retain_trash_versions(&self, ids: &BTreeSet<ObjectId>) -> Result<()> {
        let _writer = writer_lock();
        self.archive_trash_versions(ids, &self.journal_entries()?)
    }

    fn archive_trash_versions(
        &self,
        ids: &BTreeSet<ObjectId>,
        entries: &[JournalEntry],
    ) -> Result<()> {
        // Persist the last authoritative bytes/hash before retiring either
        // current objects or their journal. No codec or journal format changes.
        let mut archive = self.trash_archive()?;
        let committed: BTreeSet<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                JournalEntry::Committed { operation_id, .. } => Some(operation_id),
                _ => None,
            })
            .collect();
        for id in ids {
            let path = self.trash_purge_path(id)?;
            if archive.contains_key(id.as_str()) && !path.exists() {
                continue;
            }
            let stored = if path.exists() {
                let bytes = read_file(&path)?;
                self.load_stored(&path, ContentHash::digest(&bytes))?
            } else {
                let Some(hash) = entries.iter().rev().find_map(|entry| match entry {
                    JournalEntry::Prepared {
                        operation_id,
                        object_id,
                        after,
                        ..
                    } if object_id == id && committed.contains(operation_id) => Some(*after),
                    _ => None,
                }) else {
                    // Older completed purges retired their journal before
                    // this recovery index existed. Preserve that readable
                    // workspace without inventing an authoritative version.
                    continue;
                };
                self.load_stored(
                    &self
                        .root
                        .join("versions")
                        .join(id.as_str())
                        .join(format!("{hash}.soliton")),
                    hash,
                )?
            };
            if stored.entity.id() != id {
                return Err(KernelError::Integrity(
                    "Trash recovery identity mismatch".into(),
                ));
            }
            let history = self.root.join("versions").join(id.as_str());
            deny_symlink_components(&history)?;
            fs::create_dir_all(&history)?;
            self.preserve_version(
                &history,
                stored.content_hash,
                &stored.entity.canonical_bytes()?,
            )?;
            archive.insert(id.to_string(), stored.content_hash.to_string());
        }
        crate::trash::write(&self.root, "recovery-versions.json", &archive)
    }

    fn trash_archive(&self) -> Result<BTreeMap<String, String>> {
        let path = self.root.join("trash/recovery-versions.json");
        crate::trash::safe(&path)?;
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_slice(&read_file(&path)?)
            .map_err(|_| KernelError::Recovery("Trash recovery index cannot be read".into()))
    }

    /// Expired Trash uses the existing Recovery surface, independently of the
    /// thirty-day Trash list. Only versions recorded before purge are admitted.
    pub fn expired_trash(&self) -> Result<Vec<StoredEntity>> {
        let purged = crate::trash::TrashIndex::purged_ids(&self.root)?;
        let archive = self.trash_archive()?;
        let mut result = Vec::new();
        for id in purged {
            let Some(hash) = archive.get(id.as_str()) else {
                continue;
            };
            let hash: ContentHash = hash.parse()?;
            let path = self
                .root
                .join("versions")
                .join(id.as_str())
                .join(format!("{hash}.soliton"));
            let stored = self.load_stored(&path, hash)?;
            if stored.entity.id() != &id {
                return Err(KernelError::Integrity(
                    "Trash recovery identity mismatch".into(),
                ));
            }
            result.push(stored);
        }
        Ok(result)
    }

    pub fn trash_restore_plan(&self, id: &ObjectId) -> Result<Vec<StoredEntity>> {
        let archived = self.expired_trash()?;
        if !archived.iter().any(|v| v.entity.id() == id) {
            return Err(KernelError::Recovery(
                "Expired Trash item is unavailable".into(),
            ));
        }
        let current = self.rebuild()?;
        let snapshot = Snapshot::from_entities(
            current
                .entities()
                .cloned()
                .chain(archived.iter().map(|v| v.entity.clone())),
        )?;
        let roots = crate::trash::load(&self.root, "recovery-roots.json")?;
        let mut selected: BTreeSet<_> = archived
            .iter()
            .filter(|v| {
                crate::trash::follows(&v.entity, id.as_str(), &snapshot)
                    && !roots.iter().any(|root| {
                        root.id != id.as_str()
                            && crate::trash::follows(&v.entity, &root.id, &snapshot)
                            && snapshot
                                .get(&root.id.parse().expect("validated Trash id"))
                                .is_some_and(|root| {
                                    crate::trash::follows(&root.entity, id.as_str(), &snapshot)
                                })
                    })
            })
            .map(|v| v.entity.id().clone())
            .collect();
        // Restoring an independently deleted child also needs its missing
        // relationship parents, but never its siblings or an unrelated branch.
        loop {
            let old_len = selected.len();
            let current_selection = selected.clone();
            for v in archived
                .iter()
                .filter(|v| current_selection.contains(v.entity.id()))
            {
                let parents: Vec<_> = match &v.entity {
                    Entity::Aim(a) => a.category_id.iter().collect(),
                    Entity::Task(t) => t.category_id.iter().chain(t.aim_id.iter()).collect(),
                    _ => Vec::new(),
                };
                for parent in parents {
                    if snapshot.get(parent).is_some() {
                        selected.insert(parent.clone());
                    }
                }
            }
            if selected.len() == old_len {
                break;
            }
        }
        let mut plan: Vec<_> = archived
            .into_iter()
            .filter(|v| selected.contains(v.entity.id()))
            .collect();
        plan.sort_by_key(|v| match v.entity.kind() {
            EntityKind::Category => 0,
            EntityKind::Aim => 1,
            _ => 2,
        });
        Ok(plan)
    }

    pub fn restore_expired_trash(&self, id: &ObjectId) -> Result<usize> {
        let plan = self.trash_restore_plan(id)?;
        let _writer = writer_lock();
        let ids: Vec<_> = plan
            .iter()
            .map(|v| crate::trash::TrashEntry {
                kind: match v.entity.kind() {
                    EntityKind::Aim => "goal",
                    EntityKind::Task => "task",
                    _ => "category",
                }
                .into(),
                id: v.entity.id().to_string(),
                deleted_at: 0,
            })
            .collect();
        // Durable admission precedes any target write. Repository recovery
        // completes this exact plan before processing remaining purges.
        crate::trash::write(&self.root, "restoring.json", &ids)?;
        self.complete_trash_restore()?;
        Ok(plan.len())
    }

    fn complete_trash_restore(&self) -> Result<()> {
        let restoring = crate::trash::load(&self.root, "restoring.json")?;
        if restoring.is_empty() {
            return Ok(());
        }
        let archive = self.trash_archive()?;
        let ids: BTreeSet<_> = restoring.iter().map(|v| v.id.as_str()).collect();
        let mut stored = Vec::new();
        for entry in &restoring {
            let hash: ContentHash = archive
                .get(&entry.id)
                .ok_or_else(|| KernelError::Recovery("Trash recovery version is missing".into()))?
                .parse()?;
            let path = self
                .root
                .join("versions")
                .join(&entry.id)
                .join(format!("{hash}.soliton"));
            let value = self.load_stored(&path, hash)?;
            if value.entity.id().as_str() != entry.id {
                return Err(KernelError::Integrity(
                    "Trash recovery identity mismatch".into(),
                ));
            }
            let current = self.hash_if_present(&self.trash_purge_path(value.entity.id())?)?;
            if current.is_some_and(|current| current != hash) {
                return Err(KernelError::Conflict(
                    "Recovered Trash object changed".into(),
                ));
            }
            stored.push(value);
        }
        // All retained versions are validated before making the plan visible.
        for name in ["purged.json", "index.json", "recovery-roots.json"] {
            let entries: Vec<_> = crate::trash::load(&self.root, name)?
                .into_iter()
                .filter(|v| !ids.contains(v.id.as_str()))
                .collect();
            crate::trash::write(&self.root, name, &entries)?;
        }
        stored.sort_by_key(|v| match v.entity.kind() {
            EntityKind::Category => 0,
            EntityKind::Aim => 1,
            _ => 2,
        });
        for value in stored {
            let target = self.trash_purge_path(value.entity.id())?;
            if self.hash_if_present(&target)?.is_none() {
                let mutation =
                    self.prepare_unlocked(value.entity, MutationPrecondition::MustNotExist)?;
                self.install_prepared_unlocked(&mutation.record, true)?;
            }
        }
        crate::trash::write(
            &self.root,
            "restoring.json",
            &Vec::<crate::trash::TrashEntry>::new(),
        )?;
        Ok(())
    }

    pub fn rebuild(&self) -> Result<Snapshot> {
        self.recover()?;
        let committed_object_ids = self.committed_object_ids()?;
        let mut stored = Vec::new();
        for kind in all_entity_kinds() {
            let directory = self.kind_dir(kind);
            deny_symlink_components(&directory)?;
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() {
                    return Err(KernelError::PathDenied(format!(
                        "symlink inside governed object directory: {}",
                        path.display()
                    )));
                }
                if !metadata.is_file()
                    || path.extension().and_then(|value| value.to_str()) != Some(FILE_EXTENSION)
                {
                    continue;
                }
                let bytes = read_file(&path)?;
                let entity = Entity::from_canonical_bytes(&bytes)?;
                if entity.kind() != kind {
                    return Err(KernelError::Integrity(format!(
                        "{} contains a {} object",
                        path.display(),
                        entity.kind().as_str()
                    )));
                }
                let expected_name = format!("{}.{}", entity.id(), FILE_EXTENSION);
                if path.file_name().and_then(|value| value.to_str()) != Some(&expected_name) {
                    return Err(KernelError::Integrity(format!(
                        "object path does not match embedded id {}: {}",
                        entity.id(),
                        path.display()
                    )));
                }
                if !committed_object_ids.contains(entity.id()) {
                    return Err(KernelError::Integrity(format!(
                        "object {} has no committed journal operation",
                        entity.id()
                    )));
                }
                stored.push(StoredEntity {
                    entity,
                    content_hash: ContentHash::digest(&bytes),
                });
            }
        }
        Snapshot::from_stored(stored)
    }

    /// Persist a recurrence cursor as a sidecar owned by this repository.
    /// The cursor is bound to the exact series and rule before any bytes are
    /// staged. A global in-process writer gate serializes all repository
    /// handles; cross-process locking remains an operational deployment
    /// responsibility and is not claimed here.
    pub fn persist_recurrence_cursor(
        &self,
        cursor: &RecurrenceCursor,
        rule: &RecurrenceRule,
    ) -> Result<()> {
        let _writer = writer_lock();
        self.persist_recurrence_cursor_unlocked(cursor, rule)
    }

    fn persist_recurrence_cursor_unlocked(
        &self,
        cursor: &RecurrenceCursor,
        rule: &RecurrenceRule,
    ) -> Result<()> {
        cursor.validate_against_rule(&cursor.series_id, rule)?;
        self.ensure_recurrence_layout()?;
        let target = self.recurrence_cursor_path(&cursor.series_id);
        let stage = target.with_extension("cursor.pending");
        deny_symlink_components(&stage)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stage)?;
        file.write_all(&cursor.to_bytes())?;
        file.sync_all()?;
        sync_directory(target.parent().ok_or_else(|| {
            KernelError::Integrity("recurrence cursor has no parent directory".into())
        })?)?;
        fs::rename(&stage, &target)?;
        sync_directory(target.parent().ok_or_else(|| {
            KernelError::Integrity("recurrence cursor has no parent directory".into())
        })?)?;
        Ok(())
    }

    pub fn load_recurrence_cursor(&self, series_id: &ObjectId) -> Result<RecurrenceCursor> {
        self.ensure_recurrence_layout()?;
        let path = self.recurrence_cursor_path(series_id);
        deny_symlink_components(&path)?;
        let cursor = RecurrenceCursor::from_bytes(&read_file(&path)?)?;
        if cursor.series_id != *series_id {
            return Err(KernelError::Integrity(
                "recurrence cursor series does not match its filename".into(),
            ));
        }
        Ok(cursor)
    }

    /// Apply one materialized occurrence through the repository-owned cursor
    /// seam. The cursor is validated before mutation and is persisted only
    /// after the in-memory apply succeeds. The supplied provenance is bound
    /// into the durable cursor and later applications must use the same value.
    pub fn apply_recurrence_occurrence(
        &self,
        series_id: &ObjectId,
        rule: &RecurrenceRule,
        occurrence: &crate::recurrence::Occurrence,
        rule_provenance: impl Into<String>,
    ) -> Result<bool> {
        let _writer = writer_lock();
        self.ensure_recurrence_layout()?;
        let rule_provenance = rule_provenance.into();
        if rule_provenance.trim().is_empty() {
            return Err(KernelError::Integrity(
                "recurrence rule provenance is empty".into(),
            ));
        }
        if occurrence.id.series_id != *series_id {
            return Err(KernelError::Integrity(
                "recurrence occurrence series does not match requested series".into(),
            ));
        }
        let target = self.recurrence_cursor_path(series_id);
        let mut cursor = if target.exists() {
            let loaded = RecurrenceCursor::from_bytes(&read_file(&target)?)?;
            if loaded.series_id != *series_id {
                return Err(KernelError::Integrity(
                    "recurrence cursor series does not match requested series".into(),
                ));
            }
            loaded
        } else {
            RecurrenceCursor::initial(series_id.clone(), rule)?
        };
        cursor.bind_rule_provenance(rule_provenance)?;
        let applied = cursor.apply_occurrence(occurrence, rule)?;
        if applied {
            self.persist_recurrence_cursor_unlocked(&cursor, rule)?;
        }
        Ok(applied)
    }

    pub fn recurrence_cursor_status<'a>(
        &self,
        series_id: &ObjectId,
        rule: impl Into<Option<&'a RecurrenceRule>>,
    ) -> Result<RecurrenceCursorStatus> {
        let path = self.recurrence_cursor_path(series_id);
        if !path.exists() {
            return Ok(RecurrenceCursorStatus::Missing);
        }
        let Some(rule) = rule.into() else {
            return Ok(RecurrenceCursorStatus::Hold(
                "recurrence source rule is missing".into(),
            ));
        };
        match self
            .load_recurrence_cursor(series_id)?
            .validate_against_rule(series_id, rule)
        {
            Ok(()) => Ok(RecurrenceCursorStatus::Ready(
                self.load_recurrence_cursor(series_id)?,
            )),
            Err(error) => Ok(RecurrenceCursorStatus::Hold(error.to_string())),
        }
    }

    fn committed_object_ids(&self) -> Result<BTreeSet<ObjectId>> {
        let mut prepared = BTreeMap::<ObjectId, ObjectId>::new();
        let mut committed = BTreeSet::new();
        for entry in self.journal_entries()? {
            match entry {
                JournalEntry::Prepared {
                    operation_id,
                    object_id,
                    ..
                } => {
                    if prepared.insert(operation_id, object_id).is_some() {
                        return Err(KernelError::Integrity(
                            "journal contains duplicate prepare records".into(),
                        ));
                    }
                }
                JournalEntry::Committed { operation_id, .. } => {
                    let object_id = prepared.get(&operation_id).ok_or_else(|| {
                        KernelError::Integrity(format!(
                            "operation {operation_id} commits before it is prepared"
                        ))
                    })?;
                    committed.insert(object_id.clone());
                }
            }
        }
        Ok(committed)
    }

    pub fn journal_entries(&self) -> Result<Vec<JournalEntry>> {
        let path = self.journal_path();
        deny_symlink_components(&path)?;
        let text = String::from_utf8(read_file(&path)?)
            .map_err(|_| KernelError::Integrity("journal is not UTF-8".into()))?;
        text.lines().map(parse_journal_line).collect()
    }

    /// Reads a caller-supplied relative path only after traversal and symlink checks.
    pub fn read_relative(&self, relative: impl AsRef<Path>) -> Result<Vec<u8>> {
        let relative = relative.as_ref();
        validate_relative_path(relative)?;
        let path = self.root.join(relative);
        deny_symlink_components(&path)?;
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(&self.root) {
            return Err(KernelError::PathDenied(format!(
                "path escapes repository root: {}",
                relative.display()
            )));
        }
        read_file(&canonical)
    }

    fn install_prepared(&self, record: &PrepareRecord, recovered: bool) -> Result<StoredEntity> {
        let _writer = writer_lock();
        self.install_prepared_unlocked(record, recovered)
    }

    fn install_prepared_unlocked(
        &self,
        record: &PrepareRecord,
        recovered: bool,
    ) -> Result<StoredEntity> {
        self.validate_prepare_record(record)?;
        let target = self.object_path(record.kind, &record.object_id);
        let stage = self.stage_path(record);
        let current = self.hash_if_present(&target)?;
        if current == Some(record.after) {
            self.append_journal(&JournalEntry::Committed {
                operation_id: record.operation_id.clone(),
                recovered,
            })?;
            return self.load_stored(&target, record.after);
        }
        if current != record.before {
            return Err(KernelError::Conflict(format!(
                "{} changed after operation {} was prepared",
                record.object_id, record.operation_id
            )));
        }
        deny_symlink_components(&stage)?;
        if !stage.is_file() {
            return Err(KernelError::Recovery(format!(
                "staged bytes for operation {} are missing",
                record.operation_id
            )));
        }
        let staged = read_file(&stage)?;
        if ContentHash::digest(&staged) != record.after {
            return Err(KernelError::Integrity(format!(
                "staged bytes for operation {} do not match journal hash",
                record.operation_id
            )));
        }
        let entity = Entity::from_canonical_bytes(&staged)?;
        if entity.id() != &record.object_id || entity.kind() != record.kind {
            return Err(KernelError::Integrity(format!(
                "staged object does not match operation {}",
                record.operation_id
            )));
        }
        self.validate_entity_relationships(&entity)?;
        fs::rename(&stage, &target)?;
        sync_directory(target.parent().ok_or_else(|| {
            KernelError::Integrity("object target has no parent directory".into())
        })?)?;
        self.append_journal(&JournalEntry::Committed {
            operation_id: record.operation_id.clone(),
            recovered,
        })?;
        Ok(StoredEntity {
            entity,
            content_hash: record.after,
        })
    }

    fn validate_prepare_record(&self, record: &PrepareRecord) -> Result<()> {
        if record.object_id.kind() != record.kind.as_str() {
            return Err(KernelError::Integrity(format!(
                "journal kind does not match id {}",
                record.object_id
            )));
        }
        if record.operation_id.kind() != "op" {
            return Err(KernelError::Integrity(format!(
                "journal operation has non-operation id {}",
                record.operation_id
            )));
        }
        Ok(())
    }

    fn load_stored(&self, path: &Path, expected: ContentHash) -> Result<StoredEntity> {
        let bytes = read_file(path)?;
        let actual = ContentHash::digest(&bytes);
        if actual != expected {
            return Err(KernelError::Integrity(format!(
                "{} changed while completing a mutation",
                path.display()
            )));
        }
        Ok(StoredEntity {
            entity: Entity::from_canonical_bytes(&bytes)?,
            content_hash: actual,
        })
    }

    fn ensure_layout(&self) -> Result<()> {
        deny_symlink_components(&self.root)?;
        let objects = self.root.join(OBJECTS_DIR);
        let journal = self.root.join(JOURNAL_DIR);
        for directory in [&objects, &journal] {
            deny_symlink_components(directory)?;
            fs::create_dir_all(directory)?;
            deny_symlink_components(directory)?;
        }
        for kind in all_entity_kinds() {
            let directory = objects.join(kind.as_str());
            deny_symlink_components(&directory)?;
            fs::create_dir_all(&directory)?;
            deny_symlink_components(&directory)?;
        }
        let journal_path = self.journal_path();
        deny_symlink_components(&journal_path)?;
        if !journal_path.exists() {
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&journal_path)?;
            file.sync_all()?;
            sync_directory(&journal)?;
        }
        Ok(())
    }

    fn ensure_recurrence_layout(&self) -> Result<()> {
        self.ensure_layout()?;
        let directory = self.root.join(RECURRENCE_DIR);
        deny_symlink_components(&directory)?;
        fs::create_dir_all(&directory)?;
        deny_symlink_components(&directory)?;
        Ok(())
    }

    fn append_journal(&self, entry: &JournalEntry) -> Result<()> {
        let path = self.journal_path();
        deny_symlink_components(&path)?;
        let mut file = OpenOptions::new().append(true).open(path)?;
        file.write_all(render_journal_entry(entry).as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    fn write_stage(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        deny_symlink_components(path)?;
        let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        sync_directory(path.parent().ok_or_else(|| {
            KernelError::Integrity("staging path has no parent directory".into())
        })?)?;
        Ok(())
    }

    fn preserve_version(&self, directory: &Path, hash: ContentHash, bytes: &[u8]) -> Result<()> {
        let path = directory.join(format!("{hash}.soliton"));
        deny_symlink_components(&path)?;
        if path.exists() {
            if ContentHash::digest(&read_file(&path)?) != hash {
                return Err(KernelError::Integrity(
                    "immutable version hash mismatch".into(),
                ));
            }
        } else {
            self.write_stage(&path, bytes)?;
        }
        sync_directory(directory.parent().expect("version directory has parent"))?;
        Ok(())
    }

    /// Only versions mentioned in a completed operation are exposed as history.
    pub fn versions(&self, id: &ObjectId) -> Result<Vec<StoredEntity>> {
        let entries = self.journal_entries()?;
        let committed: BTreeSet<_> = entries
            .iter()
            .filter_map(|e| match e {
                JournalEntry::Committed { operation_id, .. } => Some(operation_id),
                _ => None,
            })
            .collect();
        let mut hashes = BTreeSet::new();
        for entry in &entries {
            if let JournalEntry::Prepared {
                operation_id,
                object_id,
                before,
                after,
                ..
            } = entry
            {
                if object_id == id && committed.contains(operation_id) {
                    hashes.insert(*after);
                    if let Some(hash) = before {
                        hashes.insert(*hash);
                    }
                }
            }
        }
        let mut result = Vec::new();
        for hash in hashes {
            let path = self
                .root
                .join("versions")
                .join(id.as_str())
                .join(format!("{hash}.soliton"));
            if !path.exists() {
                continue;
            } // v1 repositories did not preserve versions.
            deny_symlink_components(&path)?;
            let stored = self.load_stored(&path, hash)?;
            if stored.entity.id() != id {
                return Err(KernelError::Integrity("version identity mismatch".into()));
            }
            result.push(stored);
        }
        result.sort_by_key(|v| v.entity.meta().revision);
        Ok(result)
    }

    fn hash_if_present(&self, path: &Path) -> Result<Option<ContentHash>> {
        deny_symlink_components(path)?;
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => Ok(Some(ContentHash::digest(&read_file(path)?))),
            Ok(_) => Err(KernelError::PathDenied(format!(
                "governed object path is not a regular file: {}",
                path.display()
            ))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn journal_path(&self) -> PathBuf {
        self.root.join(JOURNAL_DIR).join(JOURNAL_FILE)
    }

    fn kind_dir(&self, kind: EntityKind) -> PathBuf {
        self.root.join(OBJECTS_DIR).join(kind.as_str())
    }

    fn object_path(&self, kind: EntityKind, id: &ObjectId) -> PathBuf {
        self.kind_dir(kind).join(format!("{id}.{FILE_EXTENSION}"))
    }

    fn stage_path(&self, record: &PrepareRecord) -> PathBuf {
        self.kind_dir(record.kind).join(format!(
            ".pending-{}-{}",
            record.object_id, record.operation_id
        ))
    }

    fn recurrence_cursor_path(&self, series_id: &ObjectId) -> PathBuf {
        self.root
            .join(RECURRENCE_DIR)
            .join(format!("{series_id}.cursor"))
    }

    fn validate_entity_relationships(&self, entity: &Entity) -> Result<()> {
        let mut targets = Vec::new();
        match entity {
            Entity::Category(_) | Entity::Document(_) => {}
            Entity::Aim(value) => {
                targets.push((value.category_id.as_ref(), Some(EntityKind::Category)))
            }
            Entity::Task(value) => {
                targets.push((value.category_id.as_ref(), Some(EntityKind::Category)));
                targets.push((value.aim_id.as_ref(), Some(EntityKind::Aim)));
            }
            Entity::Reminder(value) => {
                targets.push((value.related_to.as_ref(), None));
            }
            Entity::Routine(value) => {
                targets.push((value.category_id.as_ref(), Some(EntityKind::Category)));
            }
        }
        for (target, expected_kind) in targets {
            let Some(target) = target else { continue };
            let target_kind = all_entity_kinds()
                .into_iter()
                .find(|kind| kind.as_str() == target.kind())
                .ok_or_else(|| {
                    KernelError::Integrity(format!("relationship target {target} has unknown kind"))
                })?;
            let path = self.object_path(target_kind, target);
            deny_symlink_components(&path)?;
            if !path.is_file() {
                return Err(KernelError::Integrity(format!(
                    "relationship target {target} does not exist before mutation"
                )));
            }
            let referenced = Entity::from_canonical_bytes(&read_file(&path)?)?;
            if expected_kind.is_some_and(|kind| referenced.kind() != kind) {
                let expected_kind = expected_kind.expect("checked above");
                return Err(KernelError::Integrity(format!(
                    "relationship target {target} is {}, expected {}",
                    referenced.kind().as_str(),
                    expected_kind.as_str()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
#[must_use = "dropping a prepared mutation leaves it for repository recovery"]
pub struct PreparedMutation {
    repository: GovernedRepository,
    record: PrepareRecord,
}

impl PreparedMutation {
    pub fn commit(self) -> Result<StoredEntity> {
        self.repository.install_prepared(&self.record, false)
    }

    #[must_use]
    pub fn operation_id(&self) -> &ObjectId {
        &self.record.operation_id
    }
}

fn check_precondition(
    precondition: MutationPrecondition,
    actual: Option<ContentHash>,
    id: &ObjectId,
) -> Result<()> {
    match precondition {
        MutationPrecondition::MustNotExist if actual.is_some() => {
            Err(KernelError::Conflict(format!("object {id} already exists")))
        }
        MutationPrecondition::Matches(expected) if actual != Some(expected) => Err(
            KernelError::Conflict(format!("object {id} does not match expected content hash")),
        ),
        _ => Ok(()),
    }
}

fn validate_relationships(entities: &BTreeMap<ObjectId, StoredEntity>) -> Result<()> {
    for stored in entities.values() {
        match &stored.entity {
            Entity::Category(_) | Entity::Document(_) => {}
            Entity::Aim(aim) => {
                validate_optional_target(entities, aim.category_id.as_ref(), EntityKind::Category)?;
            }
            Entity::Task(task) => {
                validate_optional_target(
                    entities,
                    task.category_id.as_ref(),
                    EntityKind::Category,
                )?;
                validate_optional_target(entities, task.aim_id.as_ref(), EntityKind::Aim)?;
            }
            Entity::Reminder(reminder) => {
                if let Some(target) = &reminder.related_to {
                    if !entities.contains_key(target) {
                        return Err(KernelError::Integrity(format!(
                            "reminder {} refers to missing object {target}",
                            reminder.meta.id
                        )));
                    }
                }
            }
            Entity::Routine(routine) => {
                validate_optional_target(
                    entities,
                    routine.category_id.as_ref(),
                    EntityKind::Category,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_optional_target(
    entities: &BTreeMap<ObjectId, StoredEntity>,
    target: Option<&ObjectId>,
    expected_kind: EntityKind,
) -> Result<()> {
    let Some(target) = target else {
        return Ok(());
    };
    let stored = entities.get(target).ok_or_else(|| {
        KernelError::Integrity(format!("relationship target {target} does not exist"))
    })?;
    if stored.entity.kind() != expected_kind {
        return Err(KernelError::Integrity(format!(
            "relationship target {target} is {}, expected {}",
            stored.entity.kind().as_str(),
            expected_kind.as_str()
        )));
    }
    Ok(())
}

fn all_entity_kinds() -> [EntityKind; 6] {
    [
        EntityKind::Document,
        EntityKind::Category,
        EntityKind::Aim,
        EntityKind::Task,
        EntityKind::Reminder,
        EntityKind::Routine,
    ]
}

fn render_journal_entry(entry: &JournalEntry) -> String {
    match entry {
        JournalEntry::Prepared {
            operation_id,
            kind,
            object_id,
            before,
            after,
        } => format!(
            "PREPARE\t{operation_id}\t{}\t{object_id}\t{}\t{after}\n",
            kind.as_str(),
            before.map_or_else(|| "-".into(), |hash| hash.to_string())
        ),
        JournalEntry::Committed {
            operation_id,
            recovered,
        } => format!(
            "COMMIT\t{operation_id}\t{}\n",
            if *recovered { "recovery" } else { "normal" }
        ),
    }
}

fn parse_journal_line(line: &str) -> Result<JournalEntry> {
    let parts = line.split('\t').collect::<Vec<_>>();
    match parts.as_slice() {
        ["PREPARE", operation_id, kind, object_id, before, after] => Ok(JournalEntry::Prepared {
            operation_id: operation_id.parse()?,
            kind: EntityKind::parse(kind)?,
            object_id: object_id.parse()?,
            before: if *before == "-" {
                None
            } else {
                Some(before.parse()?)
            },
            after: after.parse()?,
        }),
        ["COMMIT", operation_id, mode] if matches!(*mode, "normal" | "recovery") => {
            Ok(JournalEntry::Committed {
                operation_id: operation_id.parse()?,
                recovered: *mode == "recovery",
            })
        }
        _ => Err(KernelError::Integrity(format!(
            "invalid journal line {line:?}"
        ))),
    }
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(KernelError::PathDenied(
            "path must be a non-empty relative path".into(),
        ));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(KernelError::PathDenied(format!(
            "path contains traversal or non-normal components: {}",
            path.display()
        )));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn deny_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(KernelError::PathDenied(format!(
                    "symlink component is forbidden: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    deny_symlink_components(path)?;
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::date::CivilDate;
    use crate::model::{Category, ObjectMeta, Priority, Task, TaskStatus};
    use crate::recurrence::{
        materialize_occurrences, RecurrenceCadence, RecurrenceCursor, RecurrenceRule,
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let count = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "soliton-kernel-{label}-{}-{count}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn category(title: &str, data_class: DataClass) -> Entity {
        Entity::Category(Category {
            meta: ObjectMeta {
                details: Default::default(),
                id: ObjectId::derive("category", title.as_bytes()).unwrap(),
                title: title.into(),
                data_class,
                revision: 1,
            },
            color: None,
            sort_order: 0,
        })
    }

    fn task(title: &str) -> Entity {
        Entity::Task(Task {
            meta: ObjectMeta {
                details: Default::default(),
                id: ObjectId::derive("task", title.as_bytes()).unwrap(),
                title: title.into(),
                data_class: DataClass::Internal,
                revision: 1,
            },
            category_id: None,
            aim_id: None,
            priority: Priority::Medium,
            status: TaskStatus::Todo,
            scheduled_for: Some(CivilDate::new(2026, 8, 1).unwrap()),
            due_on: None,
            recurrence: None,
            completed_occurrences: BTreeSet::new(),
        })
    }

    #[test]
    fn create_journals_and_rebuilds() {
        let root = TestDir::new("create");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let entity = category("Work", DataClass::Internal);
        let id = entity.id().clone();
        let stored = repository.create(entity.clone()).unwrap();
        assert_eq!(stored.entity, entity);
        let entries = repository.journal_entries().unwrap();
        assert_eq!(entries.len(), 2);
        let snapshot = repository.rebuild().unwrap();
        assert_eq!(snapshot.get(&id).unwrap().entity, entity);
    }

    #[test]
    fn prepared_write_recovers_after_restart() {
        let root = TestDir::new("recover");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let entity = task("Resume me");
        let id = entity.id().clone();
        let operation = repository
            .prepare(entity.clone(), MutationPrecondition::MustNotExist)
            .unwrap();
        let operation_id = operation.operation_id().clone();
        drop(operation);

        let reopened = GovernedRepository::open(&root.0).unwrap();
        let report = reopened.recover().unwrap();
        assert_eq!(report.committed, 1);
        let entries = reopened.journal_entries().unwrap();
        assert!(entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Committed { operation_id: id, recovered: true } if id == &operation_id
        )));
        assert_eq!(reopened.rebuild().unwrap().get(&id).unwrap().entity, entity);
        assert_eq!(reopened.recover().unwrap().committed, 0);
    }

    #[test]
    fn recovery_closes_a_transaction_renamed_before_commit_record() {
        let root = TestDir::new("recover-after-rename");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let entity = task("Rename landed");
        let id = entity.id().clone();
        let operation = repository
            .prepare(entity.clone(), MutationPrecondition::MustNotExist)
            .unwrap();
        let record = operation.record.clone();
        let stage = repository.stage_path(&record);
        let target = repository.object_path(record.kind, &record.object_id);
        fs::rename(stage, &target).unwrap();
        sync_directory(target.parent().unwrap()).unwrap();
        drop(operation);

        let reopened = GovernedRepository::open(&root.0).unwrap();
        assert_eq!(reopened.recover().unwrap().committed, 1);
        assert_eq!(reopened.rebuild().unwrap().get(&id).unwrap().entity, entity);
    }

    #[test]
    fn orphan_commit_record_fails_closed() {
        let root = TestDir::new("orphan-commit");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let orphan = ObjectId::derive("op", b"orphan").unwrap();
        let mut journal = OpenOptions::new()
            .append(true)
            .open(repository.journal_path())
            .unwrap();
        writeln!(journal, "COMMIT\t{orphan}\tnormal").unwrap();
        journal.sync_all().unwrap();
        assert!(matches!(
            repository.recover(),
            Err(KernelError::Integrity(_))
        ));
    }

    #[test]
    fn duplicate_commit_record_fails_closed() {
        let root = TestDir::new("duplicate-commit");
        let repository = GovernedRepository::open(&root.0).unwrap();
        repository
            .create(category("Duplicate commit", DataClass::Internal))
            .unwrap();
        let operation_id = repository
            .journal_entries()
            .unwrap()
            .into_iter()
            .find_map(|entry| match entry {
                JournalEntry::Committed { operation_id, .. } => Some(operation_id),
                JournalEntry::Prepared { .. } => None,
            })
            .unwrap();
        let mut journal = OpenOptions::new()
            .append(true)
            .open(repository.journal_path())
            .unwrap();
        writeln!(journal, "COMMIT\t{operation_id}\tnormal").unwrap();
        journal.sync_all().unwrap();
        assert!(matches!(
            repository.recover(),
            Err(KernelError::Integrity(message)) if message.contains("duplicate commit")
        ));
    }

    #[test]
    fn stale_hash_cannot_overwrite_newer_content() {
        let root = TestDir::new("conflict");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let original = task("Versioned");
        let original_hash = repository.create(original.clone()).unwrap().content_hash;

        let mut second = original.clone();
        let Entity::Task(second_task) = &mut second else {
            unreachable!();
        };
        second_task.meta.revision = 2;
        second_task.status = TaskStatus::InProgress;
        let second_hash = repository
            .update(second.clone(), original_hash)
            .unwrap()
            .content_hash;

        let mut stale = original;
        let Entity::Task(stale_task) = &mut stale else {
            unreachable!();
        };
        stale_task.meta.revision = 3;
        assert!(matches!(
            repository.update(stale, original_hash),
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(
            repository
                .rebuild()
                .unwrap()
                .get(second.id())
                .unwrap()
                .content_hash,
            second_hash
        );
    }

    #[test]
    fn traversal_paths_are_denied() {
        let root = TestDir::new("traversal");
        let repository = GovernedRepository::open(&root.0).unwrap();
        assert!(matches!(
            repository.read_relative("../outside"),
            Err(KernelError::PathDenied(_))
        ));
        assert!(matches!(
            repository.read_relative("/etc/passwd"),
            Err(KernelError::PathDenied(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_denied_for_reads_and_rebuilds() {
        use std::os::unix::fs::symlink;

        let root = TestDir::new("symlink");
        let outside = root.0.parent().unwrap().join(format!(
            "soliton-kernel-outside-{}",
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&outside, b"outside").unwrap();
        let repository = GovernedRepository::open(&root.0).unwrap();
        let link = root.0.join("link");
        symlink(&outside, &link).unwrap();
        assert!(matches!(
            repository.read_relative("link"),
            Err(KernelError::PathDenied(_))
        ));
        let object_link = root.0.join("objects/task/foreign.soliton");
        symlink(&outside, object_link).unwrap();
        assert!(matches!(
            repository.rebuild(),
            Err(KernelError::PathDenied(_))
        ));
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn sensitive_objects_are_excluded_from_every_nonlocal_export() {
        let public = category("Public", DataClass::Public);
        let internal = category("Internal", DataClass::Internal);
        let sensitive = category("Sensitive", DataClass::PersonalSensitive);
        let restricted = category("Restricted", DataClass::Restricted);
        let snapshot = Snapshot::from_entities([
            public.clone(),
            internal.clone(),
            sensitive.clone(),
            restricted.clone(),
        ])
        .unwrap();
        assert_eq!(
            snapshot
                .exportable_entities(ExportAudience::LocalOperator)
                .count(),
            4
        );
        for audience in [
            ExportAudience::Guest,
            ExportAudience::ExternalExecutor,
            ExportAudience::KnowledgeBundle,
        ] {
            let ids = snapshot
                .exportable_entities(audience)
                .map(|entity| entity.id())
                .collect::<BTreeSet<_>>();
            assert!(ids.contains(public.id()));
            assert!(ids.contains(internal.id()));
            assert!(!ids.contains(sensitive.id()));
            assert!(!ids.contains(restricted.id()));
        }
    }

    #[test]
    fn relationship_targets_are_validated_on_rebuild() {
        let root = TestDir::new("relations");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let mut linked = task("Linked");
        let Entity::Task(task) = &mut linked else {
            unreachable!();
        };
        task.category_id = Some(ObjectId::derive("category", b"missing").unwrap());
        assert!(matches!(
            repository.create(linked),
            Err(KernelError::Integrity(message)) if message.contains("does not exist before mutation")
        ));
        assert_eq!(repository.journal_entries().unwrap(), Vec::new());
    }

    #[test]
    fn recurrence_cursor_is_durable_and_corruption_is_a_hold_without_apply() {
        let root = TestDir::new("cursor-durable");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let series = ObjectId::derive("routine", b"cursor-series").unwrap();
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2026, 8, 1).unwrap(),
            cadence: RecurrenceCadence::Daily,
            interval: 1,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        };
        let mut cursor = RecurrenceCursor::initial(series.clone(), &rule).unwrap();
        let occurrence = materialize_occurrences(&series, &rule, rule.starts_on, rule.starts_on)
            .unwrap()
            .pop()
            .unwrap();
        cursor.apply_occurrence(&occurrence, &rule).unwrap();
        repository
            .persist_recurrence_cursor(&cursor, &rule)
            .unwrap();
        assert!(matches!(
            repository.recurrence_cursor_status(&series, &rule).unwrap(),
            RecurrenceCursorStatus::Ready(_)
        ));
        let path = repository.recurrence_cursor_path(&series);
        let before = fs::read(&path).unwrap();
        let corrupted = String::from_utf8(before.clone()).unwrap().replace(
            "next_window_start=2026-08-02",
            "next_window_start=2026-08-09",
        );
        fs::write(&path, corrupted.as_bytes()).unwrap();
        assert!(matches!(
            repository.recurrence_cursor_status(&series, &rule).unwrap(),
            RecurrenceCursorStatus::Hold(_)
        ));
        let mut loaded = repository.load_recurrence_cursor(&series).unwrap();
        assert!(loaded.apply_occurrence(&occurrence, &rule).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupted.as_bytes());
    }

    #[test]
    fn recurrence_application_retains_and_rejects_provenance() {
        let root = TestDir::new("cursor-provenance");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let series = ObjectId::derive("routine", b"provenance-series").unwrap();
        let rule = RecurrenceRule {
            starts_on: CivilDate::new(2026, 8, 1).unwrap(),
            cadence: RecurrenceCadence::Daily,
            interval: 1,
            ends_on: None,
            max_occurrences: None,
            exclusions: BTreeSet::new(),
        };
        let occurrences = materialize_occurrences(
            &series,
            &rule,
            rule.starts_on,
            CivilDate::new(2026, 8, 2).unwrap(),
        )
        .unwrap();
        repository
            .apply_recurrence_occurrence(&series, &rule, &occurrences[0], "review")
            .unwrap();
        let path = repository.recurrence_cursor_path(&series);
        let loaded = repository.load_recurrence_cursor(&series).unwrap();
        assert_eq!(loaded.rule_provenance, "review");
        let before = fs::read(&path).unwrap();
        assert!(matches!(
            repository.apply_recurrence_occurrence(&series, &rule, &occurrences[1], "other"),
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn conflicting_handles_are_serialized_and_leave_one_valid_commit() {
        let root = TestDir::new("conflicting-handles");
        let first = GovernedRepository::open(&root.0).unwrap();
        let second = GovernedRepository::open(&root.0).unwrap();
        let entity = category("single writer", DataClass::Internal);
        let left = entity.clone();
        let right = entity;
        let (left_result, right_result) = std::thread::scope(|scope| {
            let left_handle = scope.spawn(|| first.create(left));
            let right_handle = scope.spawn(|| second.create(right));
            (left_handle.join().unwrap(), right_handle.join().unwrap())
        });
        let successes = u8::from(left_result.is_ok()) + u8::from(right_result.is_ok());
        assert_eq!(successes, 1);
        assert!(left_result.is_err() || right_result.is_err());
        assert_eq!(first.rebuild().unwrap().len(), 1);
    }

    #[test]
    fn unjournaled_object_is_rejected_during_rebuild() {
        let root = TestDir::new("unjournaled");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let entity = category("Unjournaled", DataClass::Internal);
        let bytes = entity.canonical_bytes().unwrap();
        let path = repository.object_path(EntityKind::Category, entity.id());
        fs::write(path, bytes).unwrap();
        assert!(matches!(
            repository.rebuild(),
            Err(KernelError::Integrity(message)) if message.contains("no committed journal")
        ));
    }

    #[test]
    fn committed_journal_with_missing_target_holds_recovery() {
        let root = TestDir::new("missing-target");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let entity = category("Missing target", DataClass::Internal);
        let id = entity.id().clone();
        repository.create(entity).unwrap();
        fs::remove_file(repository.object_path(EntityKind::Category, &id)).unwrap();
        assert!(matches!(
            repository.recover(),
            Err(KernelError::Integrity(message)) if message.contains("durable target")
        ));
    }

    #[test]
    fn corrupted_stage_is_held_without_installing_an_object() {
        let root = TestDir::new("corrupt-stage");
        let repository = GovernedRepository::open(&root.0).unwrap();
        let entity = category("Corrupt stage", DataClass::Internal);
        let operation = repository
            .prepare(entity, MutationPrecondition::MustNotExist)
            .unwrap();
        let stage = repository.stage_path(&operation.record);
        fs::write(&stage, b"tampered staged bytes").unwrap();
        drop(operation);
        assert!(matches!(
            repository.recover(),
            Err(KernelError::Integrity(message)) if message.contains("staged bytes")
        ));
        assert!(!repository
            .object_path(
                EntityKind::Category,
                &ObjectId::derive("category", b"Corrupt stage").unwrap()
            )
            .exists());
    }
}
