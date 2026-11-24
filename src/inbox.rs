//! Durable receiver for files delivered to the Soliton phone-file inbox.
//!
//! The transport adapter is intentionally outside this module.  It should
//! decode a bounded binary chunk into the `inbox-chunk` JSON envelope and call
//! [`InboxStore::command`].  This store owns only local durable bytes and
//! receipts; it never opens, unpacks, executes, or transcribes a received
//! file.

use crate::{Error, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Maximum accepted source file size.  The service may expose a smaller
/// configured limit, but it must never accept more than this bound.
pub const MAX_FILE_BYTES: u64 = 1_073_741_824;
/// The JSON-lines transport uses hex, so the decoded chunk remains bounded at
/// 64 KiB even though the envelope is larger on the wire.
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
const MAX_LIST: usize = 100;
const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(invalid(format!(
            "Inbox path is a symlink: {}",
            path.display()
        )));
    }
    Ok(())
}

fn private_dir(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    if path.exists() {
        if !path.is_dir() {
            return Err(invalid(format!(
                "Inbox path is not a directory: {}",
                path.display()
            )));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{}", std::process::id(), nanos)
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Inbox metadata path has no parent"))?;
    private_dir(parent)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        unique_suffix()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    private_file(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    reject_symlink(path)?;
    fs::rename(&temporary, path)?;
    private_file(path)?;
    sync_dir(parent)?;
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    reject_symlink(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_METADATA_BYTES {
        return Err(invalid("Inbox metadata exceeds the size limit"));
    }
    Ok(serde_json::from_reader(file)?)
}

fn required_string(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid(format!("Inbox {key} is required")))
}

fn required_u64(args: &Value, key: &str) -> Result<u64> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("Inbox {key} must be a non-negative integer")))
}

fn sha256_hex(value: &str, key: &str) -> Result<String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid(format!(
            "Inbox {key} must be lowercase SHA-256 hex"
        )));
    }
    Ok(value.to_owned())
}

fn validate_uuid4(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[18] != b'-'
        || bytes[23] != b'-'
        || bytes[14] != b'4'
        || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        || !bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index)
                || byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
        })
    {
        return Err(invalid("Inbox op_id must be a lowercase UUID4"));
    }
    Ok(())
}

fn validate_filename(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || value.as_bytes().contains(&0)
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(invalid(
            "Inbox filename is empty, too long, or contains an unsafe character",
        ));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::RootDir | Component::ParentDir | Component::CurDir
            )
        })
    {
        return Err(invalid("Inbox filename must be a single relative name"));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid("Inbox filename contains a control character"));
    }
    Ok(())
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) || value.len() / 2 > MAX_CHUNK_BYTES {
        return Err(invalid(format!(
            "Inbox chunk must be 1–{} bytes of hex",
            MAX_CHUNK_BYTES
        )));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.bytes();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let high = hex_digit(high).ok_or_else(|| invalid("Inbox chunk is not lowercase hex"))?;
        let low = hex_digit(low).ok_or_else(|| invalid("Inbox chunk is not lowercase hex"))?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_file(path: &Path) -> Result<(u64, String)> {
    reject_symlink(path)?;
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("Inbox payload is not a regular file"));
    }
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| invalid("Inbox payload size overflow"))?;
        if size > MAX_FILE_BYTES {
            return Err(invalid("Inbox payload exceeds the size limit"));
        }
        hasher.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

fn safe_extension(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 16
                && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(|value| format!(".{value}"))
        .unwrap_or_default()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Operation {
    id: String,
    filename: String,
    size: u64,
    sha256: String,
    intent: String,
    state: String,
    offset: u64,
    disk_name: String,
    artifact_id: String,
    receipt_id: String,
    created_at: String,
    updated_at: String,
}

impl Operation {
    fn value(&self) -> Value {
        json!(self)
    }
}

pub struct InboxStore {
    staging: PathBuf,
    artifacts: PathBuf,
    receipts: PathBuf,
}

impl InboxStore {
    /// Open the already-approved private inbox root supplied by the service.
    /// This method does not accept a destination path from an inbox command.
    pub fn open(root: &Path) -> Result<Self> {
        private_dir(root)?;
        let store = Self {
            staging: root.join("staging"),
            artifacts: root.join("artifacts"),
            receipts: root.join("receipts"),
        };
        private_dir(&store.staging)?;
        private_dir(&store.artifacts)?;
        private_dir(&store.receipts)?;
        store.recover()
    }

    pub fn command(&mut self, command: &str, args: &Value) -> Result<Value> {
        match command {
            "inbox-destinations" => self.destinations(),
            "inbox-begin" => self.begin(args),
            "inbox-status" => self.status(args),
            "inbox-chunk" => self.chunk(args),
            "inbox-commit" => self.commit(args),
            "inbox-cancel" => self.cancel(args),
            "inbox-list" => self.list(args),
            _ => Err(invalid(format!("Unknown inbox command: {command}"))),
        }
    }

    fn destinations(&self) -> Result<Value> {
        let destination = json!({
            "id": "inbox",
            "label": "Soliton Inbox",
            "writable": true,
            "max_file_bytes": MAX_FILE_BYTES,
        });
        Ok(json!({"destinations":[destination]}))
    }

    fn operation_dir(&self, id: &str) -> Result<PathBuf> {
        validate_uuid4(id)?;
        Ok(self.staging.join(id))
    }

    fn metadata_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.operation_dir(id)?.join("metadata.json"))
    }

    fn payload_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.operation_dir(id)?.join("payload.part"))
    }

    fn receipt_path(&self, id: &str) -> Result<PathBuf> {
        validate_uuid4(id)?;
        Ok(self.receipts.join(format!("{id}.json")))
    }

    fn final_path(&self, operation: &Operation) -> PathBuf {
        self.artifacts.join(&operation.disk_name)
    }

    fn read_operation(&self, id: &str) -> Result<Operation> {
        let path = self.metadata_path(id)?;
        let value = read_json(&path)?;
        let operation: Operation = serde_json::from_value(value)?;
        if operation.id != id
            || operation.size > MAX_FILE_BYTES
            || operation.offset > operation.size
            || sha256_hex(&operation.sha256, "sha256").is_err()
            || !matches!(operation.intent.as_str(), "store" | "voice_note")
            || !matches!(
                operation.state.as_str(),
                "receiving" | "committing" | "committed" | "cancelled"
            )
            || validate_filename(&operation.filename).is_err()
            || operation.disk_name
                != format!("artifact-{}{}", id, safe_extension(&operation.filename))
            || operation.artifact_id != format!("artifact-{id}")
            || operation.receipt_id != format!("receipt-{id}")
        {
            return Err(invalid("Inbox metadata is corrupt or unsupported"));
        }
        Ok(operation)
    }

    fn read_receipt(&self, id: &str) -> Result<Option<Value>> {
        let path = self.receipt_path(id)?;
        reject_symlink(&path)?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json(&path)?))
    }

    fn persist_operation(&self, operation: &Operation) -> Result<()> {
        let path = self.metadata_path(&operation.id)?;
        write_json_atomic(&path, &operation.value())
    }

    fn reconcile_offset(&self, operation: &mut Operation) -> Result<bool> {
        let path = self.payload_path(&operation.id)?;
        reject_symlink(&path)?;
        if operation.state == "cancelled" || operation.state == "committed" {
            if path.exists() {
                reject_symlink(&path)?;
            }
            return Ok(false);
        }
        if !path.exists() {
            if operation.offset == 0 {
                return Ok(false);
            }
            return Err(invalid("Inbox metadata claims bytes that are missing"));
        }
        reject_symlink(&path)?;
        let length = fs::metadata(&path)?.len();
        if length > operation.size || length < operation.offset {
            return Err(invalid(
                "Inbox payload length disagrees with durable offset",
            ));
        }
        if length != operation.offset {
            operation.offset = length;
            operation.updated_at = now();
            self.persist_operation(operation)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn recover(&self) -> Result<Self> {
        for entry in fs::read_dir(&self.staging)? {
            let entry = entry?;
            let path = entry.path();
            reject_symlink(&path)?;
            if !entry.file_type()?.is_dir() {
                return Err(invalid("Inbox staging contains a non-directory entry"));
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let mut operation = self.read_operation(&id)?;
            if operation.state == "cancelled" {
                let payload = self.payload_path(&id)?;
                if payload.exists() {
                    reject_symlink(&payload)?;
                    fs::remove_file(payload)?;
                    sync_dir(&path)?;
                }
                continue;
            }
            self.reconcile_offset(&mut operation)?;
            if operation.state == "committing" || operation.state == "committed" {
                self.finalize(&mut operation)?;
            }
        }
        for entry in fs::read_dir(&self.receipts)? {
            let entry = entry?;
            let path = entry.path();
            reject_symlink(&path)?;
            if !entry.file_type()?.is_file() {
                return Err(invalid("Inbox receipts contains a non-file entry"));
            }
            let receipt_path = entry.path();
            let id = receipt_path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("Inbox receipt has an invalid filename"))?;
            validate_uuid4(id)?;
            let receipt = read_json(&receipt_path)?;
            if receipt["id"] != id || receipt["receipt_id"].as_str().is_none() {
                return Err(invalid("Inbox receipt is corrupt"));
            }
        }
        Ok(self.clone_paths())
    }

    fn clone_paths(&self) -> Self {
        Self {
            staging: self.staging.clone(),
            artifacts: self.artifacts.clone(),
            receipts: self.receipts.clone(),
        }
    }

    fn compare_begin(
        operation: &Operation,
        filename: &str,
        size: u64,
        sha256: &str,
        intent: &str,
    ) -> Result<()> {
        if operation.filename != filename
            || operation.size != size
            || operation.sha256 != sha256
            || operation.intent != intent
        {
            return Err(invalid(
                "Inbox op_id was reused with changed file content or intent",
            ));
        }
        Ok(())
    }

    fn begin(&mut self, args: &Value) -> Result<Value> {
        let id = required_string(args, "op_id")?;
        validate_uuid4(&id)?;
        let filename = required_string(args, "filename")?;
        validate_filename(&filename)?;
        let size = required_u64(args, "size")?;
        if size > MAX_FILE_BYTES {
            return Err(invalid("Inbox file exceeds the 1 GiB size limit"));
        }
        let sha256 = sha256_hex(&required_string(args, "sha256")?, "sha256")?;
        let intent = required_string(args, "intent")?;
        if !matches!(intent.as_str(), "store" | "voice_note") {
            return Err(invalid("Inbox intent must be store or voice_note"));
        }
        let directory = self.operation_dir(&id)?;
        reject_symlink(&directory)?;
        if directory.exists() {
            let mut operation = self.read_operation(&id)?;
            Self::compare_begin(&operation, &filename, size, &sha256, &intent)?;
            if operation.state == "committed" {
                return self.status_for(&operation);
            }
            if self.read_receipt(&id)?.is_some() {
                return self.status_for(&operation);
            }
            if operation.state == "cancelled" {
                operation.state = "receiving".into();
                operation.offset = 0;
                operation.updated_at = now();
                private_dir(&directory)?;
                let payload = self.payload_path(&id)?;
                if payload.exists() {
                    reject_symlink(&payload)?;
                    fs::remove_file(&payload)?;
                }
                let file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&payload)?;
                private_file(&payload)?;
                file.sync_all()?;
                self.persist_operation(&operation)?;
                sync_dir(&directory)?;
            } else {
                self.reconcile_offset(&mut operation)?;
            }
            return self.status_for(&operation);
        }
        private_dir(&directory)?;
        let payload = self.payload_path(&id)?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&payload)?;
        private_file(&payload)?;
        file.sync_all()?;
        let operation = Operation {
            id: id.clone(),
            filename: filename.clone(),
            size,
            sha256,
            intent,
            state: "receiving".into(),
            offset: 0,
            disk_name: format!("artifact-{id}{}", safe_extension(&filename)),
            artifact_id: format!("artifact-{id}"),
            receipt_id: format!("receipt-{id}"),
            created_at: now(),
            updated_at: now(),
        };
        self.persist_operation(&operation)?;
        sync_dir(&directory)?;
        sync_dir(&self.staging)?;
        self.status_for(&operation)
    }

    fn status_for(&self, operation: &Operation) -> Result<Value> {
        let receipt = self.read_receipt(&operation.id)?;
        let mut result = json!({
            "id": operation.id,
            "filename": operation.filename,
            "original_filename": operation.filename,
            "size": operation.size,
            "sha256": operation.sha256,
            "intent": operation.intent,
            "source_intent": operation.intent,
            "state": if receipt.is_some() { "committed" } else { operation.state.as_str() },
            "offset": if receipt.is_some() { operation.size } else { operation.offset },
            "max_chunk_bytes": MAX_CHUNK_BYTES,
            "artifact_id": operation.artifact_id,
            "stored_name": operation.disk_name,
        });
        if let Some(receipt) = receipt {
            result["receipt"] = receipt;
        }
        Ok(result)
    }

    fn status(&mut self, args: &Value) -> Result<Value> {
        let id = required_string(args, "id")?;
        let mut operation = self.read_operation(&id)?;
        self.reconcile_offset(&mut operation)?;
        self.status_for(&operation)
    }

    fn chunk(&mut self, args: &Value) -> Result<Value> {
        let id = required_string(args, "id")?;
        let offset = required_u64(args, "offset")?;
        let encoded = required_string(args, "hex")?;
        let expected_chunk_hash = sha256_hex(&required_string(args, "sha256")?, "sha256")?;
        let bytes = decode_hex(&encoded)?;
        let actual_chunk_hash = hash_bytes(&bytes);
        if actual_chunk_hash != expected_chunk_hash {
            return Err(invalid("Inbox chunk SHA-256 does not match its bytes"));
        }
        let mut operation = self.read_operation(&id)?;
        if operation.state == "committed" || self.read_receipt(&id)?.is_some() {
            return Err(invalid("Inbox transfer is already committed"));
        }
        if operation.state == "cancelled" {
            return Err(invalid("Inbox transfer was cancelled"));
        }
        self.reconcile_offset(&mut operation)?;
        let current = operation.offset;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("Inbox chunk offset overflow"))?;
        if end > operation.size {
            return Err(invalid("Inbox chunk exceeds the declared file size"));
        }
        let payload = self.payload_path(&id)?;
        if offset < current {
            if end > current {
                return Err(invalid("Inbox chunk overlaps the acknowledged offset"));
            }
            let mut file = File::open(&payload)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut existing = vec![0_u8; bytes.len()];
            file.read_exact(&mut existing)?;
            if existing != bytes {
                return Err(invalid("Inbox replay differs from acknowledged bytes"));
            }
            return Ok(json!({"id":id,"state":operation.state,"offset":current,"replayed":true}));
        }
        if offset != current {
            return Err(invalid(format!(
                "Inbox chunk has a gap: expected offset {current}"
            )));
        }
        if bytes.is_empty() {
            return Err(invalid("Inbox chunk cannot be empty"));
        }
        reject_symlink(&payload)?;
        let mut file = OpenOptions::new().write(true).open(&payload)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        operation.offset = end;
        operation.updated_at = now();
        self.persist_operation(&operation)?;
        sync_dir(&self.operation_dir(&id)?)?;
        Ok(json!({"id":id,"state":operation.state,"offset":operation.offset,"replayed":false}))
    }

    fn verify_payload(&self, operation: &Operation) -> Result<PathBuf> {
        let payload = self.payload_path(&operation.id)?;
        let (size, hash) = hash_file(&payload)?;
        if size != operation.size || size != operation.offset {
            return Err(invalid("Inbox payload is incomplete"));
        }
        if hash != operation.sha256 {
            return Err(invalid(
                "Inbox full-file SHA-256 does not match the declared hash",
            ));
        }
        let file = OpenOptions::new().read(true).open(&payload)?;
        file.sync_all()?;
        Ok(payload)
    }

    fn build_receipt(&self, operation: &Operation) -> Value {
        json!({
            "id": operation.id,
            "receipt_id": operation.receipt_id,
            "artifact_id": operation.artifact_id,
            "path": self.final_path(operation),
            "stored_name": operation.disk_name,
            "original_filename": operation.filename,
            "filename": operation.filename,
            "sha256": operation.sha256,
            "size": operation.size,
            "source_intent": operation.intent,
            "committed_at": now(),
        })
    }

    fn finalize(&self, operation: &mut Operation) -> Result<Value> {
        if let Some(receipt) = self.read_receipt(&operation.id)? {
            let final_path = self.final_path(operation);
            reject_symlink(&final_path)?;
            if !final_path.is_file() {
                return Err(invalid("Inbox receipt exists but its artifact is missing"));
            }
            let (size, hash) = hash_file(&final_path)?;
            if size != operation.size || hash != operation.sha256 {
                return Err(invalid(
                    "Inbox receipt points to an artifact with different bytes",
                ));
            }
            operation.state = "committed".into();
            operation.offset = operation.size;
            operation.updated_at = now();
            self.persist_operation(operation)?;
            return Ok(receipt);
        }
        let final_path = self.final_path(operation);
        reject_symlink(&final_path)?;
        if final_path.exists() {
            let (size, hash) = hash_file(&final_path)?;
            if size != operation.size || hash != operation.sha256 {
                return Err(invalid(
                    "Inbox artifact path already exists with different bytes",
                ));
            }
        } else {
            let payload = self.verify_payload(operation)?;
            fs::hard_link(&payload, &final_path).map_err(|error| {
                invalid(format!(
                    "Could not publish inbox artifact without overwrite: {error}"
                ))
            })?;
            private_file(&final_path)?;
            sync_dir(&self.artifacts)?;
        }
        let receipt = self.build_receipt(operation);
        write_json_atomic(&self.receipt_path(&operation.id)?, &receipt)?;
        operation.state = "committed".into();
        operation.offset = operation.size;
        operation.updated_at = now();
        self.persist_operation(operation)?;
        let payload = self.payload_path(&operation.id)?;
        if payload.exists() {
            reject_symlink(&payload)?;
            fs::remove_file(payload)?;
            sync_dir(&self.operation_dir(&operation.id)?)?;
        }
        Ok(receipt)
    }

    fn commit(&mut self, args: &Value) -> Result<Value> {
        let id = required_string(args, "id")?;
        let mut operation = self.read_operation(&id)?;
        if operation.state == "cancelled" {
            return Err(invalid("Inbox transfer was cancelled"));
        }
        if let Some(receipt) = self.read_receipt(&id)? {
            return Ok(receipt);
        }
        self.reconcile_offset(&mut operation)?;
        if operation.offset != operation.size {
            return Err(invalid("Inbox commit requires the complete file"));
        }
        operation.state = "committing".into();
        operation.updated_at = now();
        self.persist_operation(&operation)?;
        match self.finalize(&mut operation) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                // Keep the staged bytes recoverable after a checksum or
                // publication failure. A later retry can reconcile them.
                operation.state = "receiving".into();
                operation.updated_at = now();
                self.persist_operation(&operation)?;
                Err(error)
            }
        }
    }

    fn cancel(&mut self, args: &Value) -> Result<Value> {
        let id = required_string(args, "id")?;
        let mut operation = self.read_operation(&id)?;
        if operation.state == "committed" || self.read_receipt(&id)?.is_some() {
            return Err(invalid(
                "Cannot cancel a committed inbox transfer; its receipt remains valid",
            ));
        }
        if operation.state == "cancelled" {
            return self.status_for(&operation);
        }
        operation.state = "cancelled".into();
        operation.updated_at = now();
        self.persist_operation(&operation)?;
        let payload = self.payload_path(&id)?;
        if payload.exists() {
            reject_symlink(&payload)?;
            fs::remove_file(payload)?;
            sync_dir(&self.operation_dir(&id)?)?;
        }
        self.status_for(&operation)
    }

    fn list(&mut self, args: &Value) -> Result<Value> {
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_LIST as u64)
            .min(MAX_LIST as u64) as usize;
        let mut items = Vec::new();
        for entry in fs::read_dir(&self.staging)? {
            let entry = entry?;
            let path = entry.path();
            reject_symlink(&path)?;
            if !entry.file_type()?.is_dir() {
                return Err(invalid("Inbox staging contains a non-directory entry"));
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let mut operation = self.read_operation(&id)?;
            self.reconcile_offset(&mut operation)?;
            items.push(self.status_for(&operation)?);
        }
        items.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        items.truncate(limit);
        Ok(json!({"items":items,"limit":limit}))
    }
}
