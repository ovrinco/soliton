//! Reviewed transfers publish a separate, fully validated workspace in one rename.
use crate::{Error, Result, service::Service, time};
use serde_json::{Value, json};
use soliton_kernel::{ContentHash, Entity, ObjectId};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
fn bad(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}
fn packet(path: &Path) -> Result<(Vec<Entity>, String)> {
    if !path.is_absolute() {
        return Err(bad("Choose an absolute export path"));
    }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > 64 * 1024 * 1024 {
        return Err(bad("Choose a Soliton export under 64 MB"));
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(bad("Export exceeds 64 MB"));
    }
    let hash = ContentHash::digest(&bytes).to_string();
    let v: Value = serde_json::from_slice(&bytes)?;
    if v["format"] != "soliton-export/2" {
        return Err(bad(
            "Unsupported transfer format. The source has not been changed.",
        ));
    }
    let list = v["objects"]
        .as_array()
        .ok_or_else(|| bad("Export has no objects array"))?;
    if list.len() > 100_000 {
        return Err(bad("Too many objects in one transfer"));
    }
    let mut objects = Vec::new();
    let mut ids = BTreeSet::new();
    for item in list {
        let e = Entity::from_canonical_bytes(
            item.as_str()
                .ok_or_else(|| bad("Export object must be canonical text"))?
                .as_bytes(),
        )?;
        e.validate()?;
        if !ids.insert(e.id().to_string()) {
            return Err(bad("Export contains duplicate identities"));
        }
        if let Entity::Document(d) = &e
            && !["note", "strategy", "learning", "proposal"].contains(&d.document_type.as_str())
        {
            return Err(bad("Workspace settings cannot be imported as content"));
        }
        if let Entity::Task(t) = &e
            && let Some(z) = &t.meta.details.schedule
        {
            let day = t
                .scheduled_for
                .ok_or_else(|| bad("Timed task has no date"))?;
            let resolved = time::scheduled(
                &day.to_string(),
                &z.time,
                z.duration_minutes,
                &z.timezone,
                &z.fold,
            )?;
            if resolved.start_utc != z.start_utc {
                return Err(bad(
                    "Schedule instant differs from its civil date and timezone",
                ));
            }
        }
        for ov in e.meta().details.overrides.values() {
            time::date(&ov.origin_date)?;
            if let Some(day) = &ov.date {
                time::date(day)?;
            }
            if let Some(z) = &ov.schedule {
                time::scheduled(
                    ov.date.as_ref().unwrap_or(&ov.origin_date),
                    &z.time,
                    z.duration_minutes,
                    &z.timezone,
                    &z.fold,
                )?;
            }
        }
        objects.push(e);
    }
    for e in &objects {
        for id in &e.meta().details.links {
            if !ids.contains(id) {
                return Err(bad(format!(
                    "Linked object {id} is missing from this export"
                )));
            }
        }
    }
    soliton_kernel::Snapshot::from_entities(objects.clone())?;
    // Relationships are checked again by the repository before publication.
    objects.sort_by_key(|e| match e {
        Entity::Category(_) => 0,
        Entity::Aim(_) => 1,
        Entity::Task(_) | Entity::Routine(_) => 2,
        Entity::Document(_) => 3,
        Entity::Reminder(_) => 4,
    });
    Ok((objects, hash))
}
pub fn preview(path: &Path) -> Result<Value> {
    let (objects, hash) = packet(path)?;
    Ok(
        json!({"source":path,"hash":hash,"count":objects.len(),"titles":objects.iter().map(|e|json!({"title":e.meta().title,"kind":e.kind().as_str(),"id":e.id().as_str()})).collect::<Vec<_>>()}),
    )
}
pub fn import(path: &Path, expected: &str, target: &Path) -> Result<Value> {
    let (objects, hash) = packet(path)?;
    if hash != expected {
        return Err(bad(
            "Export changed after preview. Review it again before importing.",
        ));
    }
    if !target.is_absolute() || target.file_name().is_none() {
        return Err(bad("Choose a new absolute workspace directory"));
    }
    if target.try_exists()? {
        return Err(bad(
            "The destination already exists. Choose a new directory.",
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| bad("Destination has no parent"))?;
    let stage = parent.join(format!(".soliton-import-{}", ObjectId::mint("import")?));
    fs::create_dir(&stage)?;
    let result = (|| {
        let service = Service::open(&stage)?;
        for e in &objects {
            service.repo.create(e.clone())?;
        }
        service.repo.rebuild()?;
        let manifest = json!({"format":"soliton-transfer/1","source":path,"source_hash":hash,"objects":objects.iter().map(|e|e.id().as_str()).collect::<Vec<_>>(),"published_to":target});
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.join("transfer.json"))?;
        file.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        file.sync_all()?;
        File::open(&stage)?.sync_all()?;
        drop(service);
        publish(&stage, target)?;
        File::open(parent)?.sync_all()?;
        Ok(json!({"path":target,"count":objects.len(),"source_hash":hash}))
    })();
    // Failed or interrupted staging directories are never treated as live data.
    // Preserve a failed stage for inspection, with its exact location in the error.
    result.map_err(|e: Error| {
        bad(format!(
            "{e}. Import staging, if present: {}",
            stage.display()
        ))
    })
}
fn publish(source: &Path, target: &Path) -> Result<()> {
    // GNU mv uses renameat2(RENAME_NOREPLACE) for this no-copy, no-clobber move.
    // Arguments are passed directly without a shell. A skipped target is an error.
    let result = std::process::Command::new("/usr/bin/mv")
        .args(["--no-copy", "--no-clobber", "--no-target-directory", "--"])
        .arg(source)
        .arg(target)
        .output()?;
    if !result.status.success() || source.exists() {
        return Err(bad(format!(
            "Destination was not published: {}",
            String::from_utf8_lossy(&result.stderr)
        )));
    }
    Ok(())
}
