//! Bounded, explicit single-file checkpoints and recovery.
//!
//! Checkpoints are local evidence, not a backup.  A caller must name the
//! regular file explicitly, review a preview, and supply the current hash
//! again before restore.  No directory traversal, database repair, or
//! symlink following is performed here.
use crate::{Error, Result};
use chrono::Utc;
use serde_json::{Value, json};
use soliton_kernel::ContentHash;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TEXT_DIFF: usize = 64 * 1024;
const MAX_RECORDS: usize = 4096;
const MAX_RECORD_BYTES: u64 = 1024 * 1024;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn private_dir(path: &Path) -> Result<()> {
    reject_symlink_components(path, false)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_dir())
    {
        return Err(invalid("Recovery archive path is not a private directory"));
    }
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid("Recovery archive path is not a private directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("Recovery archive entry exceeds its size limit"));
    }
    Ok(bytes)
}

fn private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn safe_component(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(invalid(format!("Invalid {label}")));
    }
    Ok(())
}

fn reject_path_syntax(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(invalid("Choose an absolute file path"));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(invalid("File path must not contain . or .. components"));
        }
    }
    Ok(())
}

fn reject_symlink_components(path: &Path, include_final: bool) -> Result<()> {
    reject_path_syntax(path)?;
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let is_final = current == path;
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() && (include_final || !is_final) => {
                return Err(invalid("Symlink targets are not accepted for recovery"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && is_final => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn ensure_target_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("File path has no parent directory"))?;
    reject_symlink_components(parent, true)?;
    if !parent.is_dir() {
        return Err(invalid("File parent is unavailable"));
    }
    Ok(())
}

fn read_file(path: &Path) -> Result<(Vec<u8>, fs::Metadata)> {
    reject_symlink_components(path, true)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid("File is missing"));
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(invalid("Symlink targets are not accepted for recovery"));
    }
    if !metadata.is_file() {
        return Err(invalid("Recovery supports regular files only"));
    }
    if metadata.len() > MAX_BYTES {
        return Err(invalid("File exceeds the 16 MiB checkpoint limit"));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_BYTES) as usize);
    file.take(MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("File exceeds the 16 MiB checkpoint limit"));
    }
    let after = fs::symlink_metadata(path)?;
    if after.file_type().is_symlink()
        || !after.is_file()
        || after.len() != bytes.len() as u64
        || after.modified().ok() != metadata.modified().ok()
    {
        return Err(invalid("File changed while it was being checkpointed"));
    }
    Ok((bytes, after))
}

fn hash(bytes: &[u8]) -> String {
    ContentHash::digest(bytes).to_string()
}

fn stable_operation_id(checkpoint_id: &str, expected_current_hash: &str) -> String {
    let seed = format!("{checkpoint_id}\0{expected_current_hash}");
    format!(
        "restore-{}",
        ContentHash::digest(seed.as_bytes()).short_hex(32)
    )
}

fn timestamp_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn mode(metadata: &fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn modified_ms(metadata: &fs::Metadata) -> Option<String> {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis().to_string())
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Recovery record has no parent"))?;
    private_dir(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        timestamp_nanos()
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    private_file(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    sync_directory(parent)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn atomic_bytes(path: &Path, bytes: &[u8], permissions: Option<u32>) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Recovery target has no parent"))?;
    ensure_target_parent(path)?;
    let temp = parent.join(format!(".soliton-recovery-{}.tmp", timestamp_nanos()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    private_file(&temp)?;
    file.write_all(bytes)?;
    if let Some(mode) = permissions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
    }
    file.sync_all()?;
    // Recheck immediately before rename so a symlink cannot be replaced.
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&temp);
        return Err(invalid("Symlink targets are not accepted for recovery"));
    }
    fs::rename(&temp, path)?;
    sync_directory(parent)?;
    Ok(())
}

fn text_diff(before: &[u8], current: Option<&[u8]>) -> Value {
    let Some(current) = current else {
        return Value::Null;
    };
    if before.len() > MAX_TEXT_DIFF || current.len() > MAX_TEXT_DIFF {
        return json!({"kind":"unavailable","reason":"Files are too large for an inline text diff"});
    }
    let Ok(before) = std::str::from_utf8(before) else {
        return json!({"kind":"binary","reason":"Checkpoint is not UTF-8 text"});
    };
    let Ok(current) = std::str::from_utf8(current) else {
        return json!({"kind":"binary","reason":"Current file is not UTF-8 text"});
    };
    if before == current {
        json!({"kind":"text","changed":false,"before":before,"current":current})
    } else {
        json!({"kind":"text","changed":true,"before":before,"current":current})
    }
}

pub struct RecoveryStore {
    root: PathBuf,
    records: BTreeMap<String, Value>,
}

impl RecoveryStore {
    pub fn open(root: &Path) -> Result<Self> {
        reject_path_syntax(root)?;
        if root.exists() && fs::symlink_metadata(root)?.file_type().is_symlink() {
            return Err(invalid("Recovery root cannot be a symlink"));
        }
        private_dir(root)?;
        private_dir(&root.join("records"))?;
        private_dir(&root.join("blobs"))?;
        private_dir(&root.join("receipts"))?;
        private_dir(&root.join("intents"))?;
        let mut records = BTreeMap::new();
        for entry in fs::read_dir(root.join("records"))? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() || !entry.file_type()?.is_file() {
                return Err(invalid(
                    "Recovery archive contains an unexpected record entry",
                ));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name
                .strip_suffix(".json")
                .ok_or_else(|| invalid("Recovery archive contains an unexpected record file"))?;
            safe_component(id, "checkpoint id")?;
            let bytes = read_bounded(&entry.path(), MAX_RECORD_BYTES)?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|error| invalid(format!("Corrupt recovery record {id}: {error}")))?;
            if value["id"].as_str() != Some(id)
                || value["kind"] != "checkpoint"
                || value["schema"] != 1
            {
                return Err(invalid(format!("Corrupt recovery record {id}")));
            }
            let blob = value["blob"]
                .as_str()
                .ok_or_else(|| invalid("Recovery record has no blob"))?;
            safe_component(blob, "recovery blob")?;
            let blob_path = root.join("blobs").join(format!("{blob}.bin"));
            let blob_bytes = read_bounded(&blob_path, MAX_BYTES)
                .map_err(|_| invalid(format!("Recovery blob missing for {id}")))?;
            if hash(&blob_bytes) != value["hash"].as_str().unwrap_or("") {
                return Err(invalid(format!("Corrupt recovery blob for {id}")));
            }
            records.insert(id.to_owned(), value);
            if records.len() > MAX_RECORDS {
                return Err(invalid("Recovery archive contains too many checkpoints"));
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            records,
        })
    }

    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        if args["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("expired-trash-"))
            && matches!(command, "recovery-preview" | "recovery-restore")
        {
            return self.expired_trash(command, args);
        }
        match command {
            "recovery-group-checkpoint" => self.group_checkpoint(args),
            "recovery-group-list" => self.group_list(),
            "recovery-group-preview" => self.group_preview(args),
            "recovery-group-restore" => self.group_restore(args),
            "recovery-checkpoint" => self.checkpoint(args),
            "recovery-list" => self.list(args),
            "recovery-preview" => self.preview(args),
            "recovery-restore" => self.restore(args),
            _ => Err(invalid(format!("Unknown recovery command: {command}"))),
        }
    }

    fn checkpoint(&mut self, args: &Value) -> Result<Value> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| invalid("Checkpoint path is required"))?;
        let path = PathBuf::from(path);
        let (bytes, metadata) = read_file(&path)?;
        self.archive_checkpoint(&path, &bytes, &metadata, "explicit")
    }

    fn archive_checkpoint(
        &mut self,
        path: &Path,
        bytes: &[u8],
        metadata: &fs::Metadata,
        reason: &str,
    ) -> Result<Value> {
        if self.records.len() >= MAX_RECORDS {
            return Err(invalid(
                "Recovery checkpoint limit reached; existing checkpoints are preserved",
            ));
        }
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("File exceeds the 16 MiB checkpoint limit"));
        }
        let content_hash = hash(bytes);
        let blob = ContentHash::digest(bytes).short_hex(32);
        let blob_path = self.root.join("blobs").join(format!("{blob}.bin"));
        if blob_path.exists() {
            let existing = read_bounded(&blob_path, MAX_BYTES)?;
            if hash(&existing) != content_hash {
                return Err(invalid("Recovery blob hash mismatch"));
            }
        } else {
            atomic_bytes(&blob_path, bytes, None)?;
        }
        let mut id = format!("checkpoint-{}-{}", &blob[..16], timestamp_nanos());
        let mut suffix = 0_u32;
        while self.records.contains_key(&id) {
            suffix += 1;
            id = format!("checkpoint-{}-{}-{suffix}", &blob[..16], timestamp_nanos());
        }
        let record = json!({
            "schema":1,
            "kind":"checkpoint",
            "id":id,
            "path":path,
            "hash":content_hash,
            "blob":blob,
            "bytes":bytes.len(),
            "mode":mode(metadata),
            "modified":modified_ms(metadata),
            "captured_at":Utc::now().to_rfc3339(),
            "reason":reason
        });
        atomic_json(
            &self.root.join("records").join(format!("{id}.json")),
            &record,
        )?;
        self.records.insert(id, record.clone());
        Ok(record)
    }

    fn list(&self, args: &Value) -> Result<Value> {
        let filter = args["path"].as_str();
        let mut items = self
            .records
            .values()
            .filter(|record| filter.is_none() || record["path"].as_str() == filter)
            .cloned()
            .collect::<Vec<_>>();
        if let Some(workspace) = self
            .root
            .parent()
            .filter(|root| root.join("trash/recovery-versions.json").is_file())
        {
            let repo = soliton_kernel::GovernedRepository::open(workspace)?;
            for stored in repo.expired_trash()? {
                let path = repo.trash_purge_path(stored.entity.id())?;
                if filter.is_some_and(|filter| Some(filter) != path.to_str()) {
                    continue;
                }
                items.push(json!({"id":format!("expired-trash-{}", stored.entity.id()),
                    "kind":"checkpoint", "path":path, "hash":stored.content_hash.to_string(),
                    "bytes":stored.entity.canonical_bytes()?.len(), "reason":"expired-trash",
                    "title":stored.entity.meta().title}));
            }
        }
        items.sort_by(|a, b| b["captured_at"].as_str().cmp(&a["captured_at"].as_str()));
        Ok(
            json!({"items":items,"local_only":true,"notice":"Local checkpoints are not an independent backup"}),
        )
    }

    fn expired_trash(&self, command: &str, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Recovery id is required"))?;
        let object: soliton_kernel::ObjectId = id
            .strip_prefix("expired-trash-")
            .ok_or_else(|| invalid("Invalid expired Trash id"))?
            .parse()?;
        let workspace = self
            .root
            .parent()
            .ok_or_else(|| invalid("Recovery workspace is unavailable"))?;
        let repo = soliton_kernel::GovernedRepository::open(workspace)?;
        let plan = repo.trash_restore_plan(&object)?;
        let root = plan
            .iter()
            .find(|v| v.entity.id() == &object)
            .ok_or_else(|| invalid("Expired Trash item is unavailable"))?;
        // The reviewed token covers every restored identity/hash and every
        // current target, rather than just the root file in the preview.
        let mut fingerprint = Vec::new();
        let mut affected = Vec::new();
        for stored in &plan {
            let path = repo.trash_purge_path(stored.entity.id())?;
            let current = Self::current(&path)?;
            let current_hash = current.as_ref().map(|(bytes, _)| hash(bytes));
            let row = json!({"id":stored.entity.id().to_string(),"path":path,
                "hash":stored.content_hash.to_string(),"current_hash":current_hash});
            fingerprint.extend(serde_json::to_vec(&row)?);
            affected.push(row);
        }
        let token = hash(&fingerprint);
        let path = repo.trash_purge_path(&object)?;
        if command == "recovery-preview" {
            return Ok(json!({"id":id,"op_id":stable_operation_id(id,&token),
                "path":path,"current_hash":token,"checkpoint_hash":root.content_hash.to_string(),
                "current_state":"missing","missing":true,"conflict":false,
                "affected":affected,"diff":{"kind":"text","before":String::from_utf8(root.entity.canonical_bytes()?).map_err(|_| invalid("Invalid object text"))?,"current":""},
                "local_only":true}));
        }
        if args["expected_current_hash"].as_str() != Some(&token) {
            return Err(invalid(
                "Recovery conflict: preview the expired Trash group again",
            ));
        }
        if args["op_id"]
            .as_str()
            .is_some_and(|op| op != stable_operation_id(id, &token))
        {
            return Err(invalid("Recovery operation does not match its preview"));
        }
        let count = repo.restore_expired_trash(&object)?;
        Ok(
            json!({"id":id,"status":"applied","path":path,"restored_hash":root.content_hash.to_string(),"restored_count":count}),
        )
    }

    fn load_record(&self, args: &Value) -> Result<(String, Value, Vec<u8>)> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Checkpoint id is required"))?;
        safe_component(id, "checkpoint id")?;
        let record = self
            .records
            .get(id)
            .ok_or_else(|| invalid("Checkpoint not found"))?
            .clone();
        let blob = record["blob"]
            .as_str()
            .ok_or_else(|| invalid("Recovery record has no blob"))?;
        let bytes = read_bounded(
            &self.root.join("blobs").join(format!("{blob}.bin")),
            MAX_BYTES,
        )
        .map_err(|_| invalid("Recovery checkpoint bytes are unavailable"))?;
        if hash(&bytes) != record["hash"].as_str().unwrap_or("") {
            return Err(invalid(
                "Recovery checkpoint bytes failed integrity verification",
            ));
        }
        Ok((id.to_owned(), record, bytes))
    }

    fn load_optional_json(&self, path: &Path) -> Result<Option<Value>> {
        let bytes = match read_bounded(path, MAX_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| invalid(format!("Corrupt recovery operation record: {error}")))
    }

    fn persist_intent(&self, intent: &Value) -> Result<()> {
        let op_id = intent["op_id"]
            .as_str()
            .ok_or_else(|| invalid("Recovery intent has no operation id"))?;
        safe_component(op_id, "recovery operation id")?;
        atomic_json(
            &self.root.join("intents").join(format!("{op_id}.json")),
            intent,
        )
    }

    fn publish_receipt(&self, receipt: &Value) -> Result<Value> {
        let op_id = receipt["op_id"]
            .as_str()
            .ok_or_else(|| invalid("Recovery receipt has no operation id"))?;
        safe_component(op_id, "recovery operation id")?;
        let path = self.root.join("receipts").join(format!("{op_id}.json"));
        if let Some(existing) = self.load_optional_json(&path)? {
            if existing["op_id"] != op_id || existing["status"] != "applied" {
                return Err(invalid("Recovery receipt conflicts with its operation id"));
            }
            return Ok(existing);
        }
        // Append first so a crash between journal and projection can be
        // replayed. Duplicate journal lines on retry are harmless; the
        // immutable receipt projection remains the operation identity.
        let journal = self.root.join("receipts").join("journal.ndjson");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&journal)?;
        private_file(&journal)?;
        file.write_all(serde_json::to_string(receipt)?.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        atomic_json(&path, receipt)?;
        Ok(receipt.clone())
    }

    fn current(path: &Path) -> Result<Option<(Vec<u8>, fs::Metadata)>> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(invalid("Symlink targets are not accepted for recovery"))
            }
            Ok(_) => Ok(Some(read_file(path)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn preview(&self, args: &Value) -> Result<Value> {
        let (id, record, bytes) = self.load_record(args)?;
        let path = PathBuf::from(
            record["path"]
                .as_str()
                .ok_or_else(|| invalid("Recovery record has no path"))?,
        );
        ensure_target_parent(&path)?;
        let current = Self::current(&path)?;
        let (current_bytes, current_hash, state) = match &current {
            Some((bytes, _)) => (Some(bytes.as_slice()), Some(hash(bytes)), "available"),
            None => (None, None, "missing"),
        };
        let expected = current_hash.as_deref().unwrap_or("missing");
        let op_id = stable_operation_id(&id, expected);
        Ok(json!({
            "id":id,
            "op_id":op_id,
            "path":path,
            "checkpoint_hash":record["hash"],
            "current_hash":current_hash,
            "current_state":state,
            "missing":current.is_none(),
            "conflict":current_hash.as_deref().is_some_and(|value| Some(value) != record["hash"].as_str()),
            "diff":text_diff(&bytes,current_bytes),
            "checkpoint":record,
            "local_only":true
        }))
    }

    fn restore(&mut self, args: &Value) -> Result<Value> {
        let (id, record, bytes) = self.load_record(args)?;
        let expected = args["expected_current_hash"]
            .as_str()
            .ok_or_else(|| invalid("Preview the checkpoint and supply expected_current_hash"))?;
        let op_id = args["op_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| stable_operation_id(&id, expected));
        safe_component(&op_id, "recovery operation id")?;
        let path = PathBuf::from(
            record["path"]
                .as_str()
                .ok_or_else(|| invalid("Recovery record has no path"))?,
        );
        ensure_target_parent(&path)?;
        if let Some(existing) =
            self.load_optional_json(&self.root.join("receipts").join(format!("{op_id}.json")))?
        {
            if existing["checkpoint_id"] != id
                || existing["path"].as_str() != path.to_str()
                || existing
                    .get("expected_current_hash")
                    .is_some_and(|v| v != expected)
            {
                return Err(invalid(
                    "Recovery operation id is already bound to another file",
                ));
            }
            return Ok(existing);
        }
        let current = Self::current(&path)?;
        let actual = current
            .as_ref()
            .map(|(bytes, _)| hash(bytes))
            .unwrap_or_else(|| "missing".into());
        // A retry after a crash may observe the restored bytes before the
        // receipt projection was published. Reconcile that exact operation
        // without taking another before checkpoint or replacing twice.
        if actual == record["hash"].as_str().unwrap_or("") && actual != expected {
            let intent = self
                .load_optional_json(&self.root.join("intents").join(format!("{op_id}.json")))?
                .ok_or_else(|| {
                    invalid("Recovery conflict: no admitted restore matches this retry")
                })?;
            if intent["checkpoint_id"] != id || intent["expected_current_hash"] != expected {
                return Err(invalid("Recovery retry does not match its admitted intent"));
            }
            let receipt = json!({
                "schema":1,
                "kind":"recovery_receipt",
                "id":op_id,
                "op_id":op_id,
                "status":"applied",
                "checkpoint_id":id,
                "path":path,
                "restored_hash":record["hash"],
                "expected_current_hash":expected,
                "completed_at":Utc::now().to_rfc3339(),
                "reconciled":true
            });
            let result = self.publish_receipt(&receipt)?;
            self.persist_intent(&json!({"schema":1,"op_id":op_id,"status":"applied","checkpoint_id":id,"path":path}))?;
            return Ok(result);
        }
        if actual != expected {
            return Err(invalid(format!(
                "Recovery conflict: current file hash is {actual}"
            )));
        }
        let intent_path = self.root.join("intents").join(format!("{op_id}.json"));
        let intent = self.load_optional_json(&intent_path)?;
        let before_checkpoint_id = if let Some(existing) = intent.as_ref() {
            if existing["checkpoint_id"] != id
                || existing["path"].as_str() != path.to_str()
                || existing
                    .get("expected_current_hash")
                    .is_some_and(|v| v != expected)
            {
                return Err(invalid(
                    "Recovery operation id is already bound to another file",
                ));
            }
            existing["before_checkpoint_id"].as_str().map(str::to_owned)
        } else {
            None
        };
        let before_checkpoint_id = if let Some(before_checkpoint_id) = before_checkpoint_id {
            Some(before_checkpoint_id)
        } else if let Some((current_bytes, metadata)) = current {
            let before =
                self.archive_checkpoint(&path, &current_bytes, &metadata, "before-restore")?;
            let before_id = before["id"].as_str().unwrap_or("").to_owned();
            let next = json!({
                "schema":1,
                "op_id":op_id,
                "status":"prepared",
                "checkpoint_id":id,
                "path":path,
                "expected_current_hash":expected,
                "before_checkpoint_id":before_id
            });
            self.persist_intent(&next)?;
            Some(before_id)
        } else {
            let next = json!({
                "schema":1,
                "op_id":op_id,
                "status":"prepared",
                "checkpoint_id":id,
                "path":path,
                "expected_current_hash":expected,
                "before_checkpoint_id":Value::Null
            });
            self.persist_intent(&next)?;
            None
        };
        // The pre-restore checkpoint can take time. Re-read at the commit
        // boundary so a later edit cannot be silently replaced.
        let latest = Self::current(&path)?;
        let latest_hash = latest
            .as_ref()
            .map(|(bytes, _)| hash(bytes))
            .unwrap_or_else(|| "missing".into());
        if latest_hash != expected {
            return Err(invalid(format!(
                "Recovery conflict: current file hash is {latest_hash}"
            )));
        }
        atomic_bytes(
            &path,
            &bytes,
            record["mode"].as_u64().map(|value| value as u32),
        )?;
        let restored = read_file(&path)?.0;
        if hash(&restored) != record["hash"].as_str().unwrap_or("") {
            return Err(invalid("Restored file failed integrity verification"));
        }
        let receipt = json!({
            "schema":1,
            "kind":"recovery_receipt",
            "id":op_id,
            "op_id":op_id,
            "status":"applied",
            "checkpoint_id":id,
            "path":path,
            "restored_hash":record["hash"],
            "expected_current_hash":expected,
            "completed_at":Utc::now().to_rfc3339(),
            "before_checkpoint_id":before_checkpoint_id
        });
        let result = self.publish_receipt(&receipt)?;
        self.persist_intent(&json!({
            "schema":1,
            "op_id":op_id,
            "status":"applied",
            "checkpoint_id":id,
            "path":path,
            "before_checkpoint_id":before_checkpoint_id
        }))?;
        Ok(result)
    }
}

include!("recovery_group.rs");
