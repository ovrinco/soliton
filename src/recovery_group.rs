// Multi-file recovery composes the same reviewed single-file transaction.
// Filesystem-wide atomic replacement is not promised: every partial outcome is
// durable and retryable, with conflicts stopping further writes.
impl RecoveryStore {
    fn group_path(&self, area: &str, id: &str) -> Result<PathBuf> {
        safe_component(id, "recovery group identifier")?;
        let dir = self.root.join(area);
        private_dir(&dir)?;
        Ok(dir.join(format!("{id}.json")))
    }
    fn group_load(&self, id: &str) -> Result<Value> {
        let path = self.group_path("groups", id)?;
        let value = self
            .load_optional_json(&path)?
            .ok_or_else(|| invalid("Recovery group not found"))?;
        if value["schema"] != 1 || value["id"] != id || value["kind"] != "checkpoint_group" {
            return Err(invalid("Invalid recovery group manifest"));
        }
        let entries = value["entries"]
            .as_array()
            .ok_or_else(|| invalid("Recovery group has no file manifest"))?;
        if entries.is_empty() || entries.len() > 256 {
            return Err(invalid("Invalid recovery group size"));
        }
        Ok(value)
    }
    fn group_checkpoint(&mut self, args: &Value) -> Result<Value> {
        let paths = args["paths"]
            .as_array()
            .ok_or_else(|| invalid("Choose explicit files for the checkpoint group"))?;
        if paths.is_empty() || paths.len() > 256 {
            return Err(invalid("A checkpoint group accepts 1–256 files"));
        }
        let mut admitted = std::collections::BTreeSet::new();
        let mut total = 0_u64;
        // Preflight the entire selection before archiving. Each actual read is
        // bounded again; source mutation remains visible in captured hashes.
        for path in paths {
            let path = path
                .as_str()
                .ok_or_else(|| invalid("Checkpoint paths must be strings"))?;
            reject_symlink_components(Path::new(path), true)?;
            let metadata = fs::metadata(path)?;
            if !metadata.is_file() || metadata.len() > MAX_BYTES {
                return Err(invalid(
                    "Each checkpoint target must be a regular file of at most 16 MiB",
                ));
            }
            if !admitted.insert(path.to_owned()) {
                return Err(invalid("Duplicate checkpoint target"));
            }
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| invalid("Checkpoint size overflow"))?;
            if total > 256 * 1024 * 1024 {
                return Err(invalid("Checkpoint group exceeds 256 MiB"));
            }
        }
        let mut entries = Vec::new();
        let mut captured = 0_u64;
        for path in admitted {
            let item = self.checkpoint(&json!({"path":path}))?;
            captured += item["bytes"].as_u64().unwrap_or(0);
            if captured > 256 * 1024 * 1024 {
                return Err(invalid(
                    "Files grew beyond the group size limit; individual checkpoints retained",
                ));
            }
            entries.push(json!({"id":item["id"],"path":item["path"],"hash":item["hash"],"bytes":item["bytes"]}));
        }
        let id = format!("group-{}", timestamp_nanos());
        let title = args["title"].as_str().unwrap_or("Selected project files");
        if title.len() > 1000 {
            return Err(invalid("Checkpoint title is too long"));
        }
        let group = json!({"schema":1,"id":id,"kind":"checkpoint_group","title":title,"created_at":Utc::now().to_rfc3339(),"entries":entries,"bytes":captured,"coverage":"Explicit selected files only; new files, deletions outside this selection and external effects are not restored"});
        atomic_json(&self.group_path("groups", &id)?, &group)?;
        Ok(group)
    }
    fn group_list(&self) -> Result<Value> {
        let dir = self.root.join("groups");
        private_dir(&dir)?;
        let mut items = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() || entry.file_type()?.is_symlink() {
                return Err(invalid("Unexpected recovery group entry"));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name
                .strip_suffix(".json")
                .ok_or_else(|| invalid("Invalid group file"))?;
            let mut value = self.group_load(id)?;
            value["file_count"] = json!(value["entries"].as_array().map_or(0, Vec::len));
            value.as_object_mut().unwrap().remove("entries");
            items.push(value);
            if items.len() > 4096 {
                return Err(invalid("Too many recovery groups"));
            }
        }
        items.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
        Ok(json!({"items":items}))
    }
    fn group_preview(&self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Group id required"))?;
        let group = self.group_load(id)?;
        let mut entries = Vec::new();
        for entry in group["entries"].as_array().unwrap() {
            let mut preview = self.preview(&json!({"id":entry["id"]}))?;
            // Bounded group preview carries change facts, not 256 large diffs.
            preview.as_object_mut().unwrap().remove("diff");
            preview.as_object_mut().unwrap().remove("checkpoint");
            entries.push(preview);
        }
        let revision = hash(&serde_json::to_vec(&entries)?);
        let op_id = stable_operation_id(id, &revision);
        Ok(
            json!({"id":id,"title":group["title"],"revision":revision,"op_id":op_id,"entries":entries,"coverage":group["coverage"],"notice":"Review every selected file. Restore preserves current bytes first and stops at conflicts. A multi-file restore can be partial after interruption."}),
        )
    }
    fn group_restore(&mut self, args: &Value) -> Result<Value> {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("Group id required"))?;
        let revision = args["expected_revision"]
            .as_str()
            .ok_or_else(|| invalid("Preview the group before restoring"))?;
        let op = args["op_id"]
            .as_str()
            .ok_or_else(|| invalid("Group operation id required"))?;
        if op != stable_operation_id(id, revision) {
            return Err(invalid(
                "Recovery group operation does not match its preview",
            ));
        }
        let plan_path = self.group_path("group-intents", op)?;
        let receipt_path = self.group_path("group-receipts", op)?;
        if let Some(receipt) = self.load_optional_json(&receipt_path)? {
            if receipt["id"] != id || receipt["expected_revision"] != revision {
                return Err(invalid("Recovery group operation mismatch"));
            }
            return Ok(receipt);
        }
        let mut plan = if let Some(plan) = self.load_optional_json(&plan_path)? {
            if plan["id"] != id || plan["expected_revision"] != revision || plan["op_id"] != op {
                return Err(invalid("Recovery group intent mismatch"));
            }
            plan
        } else {
            let preview = self.group_preview(&json!({"id":id}))?;
            if preview["revision"] != revision {
                return Err(invalid(
                    "Recovery group conflict: one or more files changed after preview",
                ));
            }
            let plan = json!({"schema":1,"id":id,"op_id":op,"expected_revision":revision,"status":"prepared","entries":preview["entries"],"outcomes":[]});
            atomic_json(&plan_path, &plan)?;
            plan
        };
        let entries = plan["entries"]
            .as_array()
            .ok_or_else(|| invalid("Invalid restore intent"))?
            .clone();
        let mut outcomes = plan["outcomes"].as_array().cloned().unwrap_or_default();
        for (index, entry) in entries.iter().enumerate() {
            let subop = format!("{}-{}", op, index);
            if index < outcomes.len() && outcomes[index]["status"] == "applied" {
                continue;
            }
            let result = self.restore(&json!({"id":entry["id"],"op_id":subop,"expected_current_hash":entry["current_hash"].as_str().unwrap_or("missing")}));
            match result {
                Ok(receipt) => {
                    if index < outcomes.len() {
                        outcomes[index] = receipt;
                    } else {
                        outcomes.push(receipt);
                    }
                }
                Err(error) => {
                    plan["status"] = json!("partial");
                    plan["outcomes"] = json!(outcomes);
                    plan["blocked_file"] = entry["path"].clone();
                    plan["error"] = json!(error.to_string());
                    atomic_json(&plan_path, &plan)?;
                    return Ok(plan);
                }
            }
            plan["outcomes"] = json!(outcomes);
            plan["status"] = json!("applying");
            atomic_json(&plan_path, &plan)?;
        }
        plan["status"] = json!("applied");
        plan["completed_at"] = json!(Utc::now().to_rfc3339());
        plan.as_object_mut().unwrap().remove("error");
        plan.as_object_mut().unwrap().remove("blocked_file");
        atomic_json(&receipt_path, &plan)?;
        atomic_json(&plan_path, &plan)?;
        Ok(plan)
    }
}
