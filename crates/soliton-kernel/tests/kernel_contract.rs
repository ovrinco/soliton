use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use soliton_kernel::{
    aim_rollup, calendar_month, kanban_projection, list_projection, Aim, AimState, AppRegistry,
    AppRegistryRow, AppState, CapabilityRow, Category, CivilDate, ContentHash, DataClass, Entity,
    ExportAudience, GovernedRepository, Grant, MutationPrecondition, ObjectId, ObjectMeta,
    PackageAction, Priority, QreamGate, RecurrenceCadence, RecurrenceRule, Routine, RoutineState,
    Snapshot, Task, TaskStatus, Verdict,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "soliton-kernel-integration-{label}-{}-{count}",
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

fn meta(kind: &str, title: &str, data_class: DataClass) -> ObjectMeta {
    ObjectMeta {
        details: Default::default(),
        id: ObjectId::derive(kind, title.as_bytes()).unwrap(),
        title: title.into(),
        data_class,
        revision: 1,
    }
}

#[test]
fn one_object_spine_drives_list_kanban_calendar_and_derived_rollup() {
    let category = Entity::Category(Category {
        meta: meta("category", "Work", DataClass::Internal),
        color: Some("#DAA520".into()),
        sort_order: 1,
    });
    let aim = Entity::Aim(Aim {
        folder: None,
        meta: meta("aim", "Ship", DataClass::Internal),
        category_id: Some(category.id().clone()),
        state: AimState::Active,
        target_on: Some(CivilDate::new(2026, 8, 31).unwrap()),
    });
    let task = Entity::Task(Task {
        meta: meta("task", "Integrate", DataClass::Internal),
        category_id: Some(category.id().clone()),
        aim_id: Some(aim.id().clone()),
        priority: Priority::High,
        status: TaskStatus::Completed,
        scheduled_for: Some(CivilDate::new(2026, 8, 3).unwrap()),
        due_on: None,
        recurrence: None,
        completed_occurrences: BTreeSet::new(),
    });
    let task_id = task.id().clone();
    let aim_id = aim.id().clone();
    let snapshot = Snapshot::from_entities([category, aim, task]).unwrap();

    assert!(list_projection(&snapshot)
        .iter()
        .any(|row| row.source_id == task_id));
    assert_eq!(
        kanban_projection(&snapshot)[&TaskStatus::Completed][0].source_id,
        task_id
    );
    assert!(
        calendar_month(&snapshot, CivilDate::new(2026, 8, 1).unwrap())
            .unwrap()
            .items
            .iter()
            .any(|row| row.source_id == task_id)
    );
    assert_eq!(
        aim_rollup(&snapshot, &aim_id).completion_basis_points(),
        10_000
    );
}

#[test]
fn recurrence_survives_repository_restart_with_identical_occurrence_ids() {
    let root = TestDir::new("recurrence-restart");
    let repository = GovernedRepository::open(&root.0).unwrap();
    let routine = Entity::Routine(Routine {
        meta: meta("routine", "Daily walk", DataClass::PersonalSensitive),
        category_id: None,
        state: RoutineState::Active,
        recurrence: RecurrenceRule {
            starts_on: CivilDate::new(2026, 8, 1).unwrap(),
            cadence: RecurrenceCadence::Daily,
            interval: 1,
            ends_on: None,
            max_occurrences: Some(5),
            exclusions: [CivilDate::new(2026, 8, 3).unwrap()].into_iter().collect(),
        },
        completed_occurrences: BTreeSet::new(),
    });
    repository.create(routine).unwrap();
    let before = calendar_month(
        &repository.rebuild().unwrap(),
        CivilDate::new(2026, 8, 1).unwrap(),
    )
    .unwrap();
    let reopened = GovernedRepository::open(&root.0).unwrap();
    let after = calendar_month(
        &reopened.rebuild().unwrap(),
        CivilDate::new(2026, 8, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(before, after);
    assert_eq!(before.items.len(), 4);
}

#[test]
fn prepared_atomic_mutation_is_completed_by_recovery() {
    let root = TestDir::new("atomic-recovery");
    let repository = GovernedRepository::open(&root.0).unwrap();
    let entity = Entity::Category(Category {
        meta: meta("category", "Recoverable", DataClass::Internal),
        color: None,
        sort_order: 0,
    });
    let id = entity.id().clone();
    let prepared = repository
        .prepare(entity, MutationPrecondition::MustNotExist)
        .unwrap();
    drop(prepared);
    let reopened = GovernedRepository::open(&root.0).unwrap();
    assert_eq!(reopened.recover().unwrap().committed, 1);
    assert!(reopened.rebuild().unwrap().get(&id).is_some());
    assert_eq!(reopened.recover().unwrap().committed, 0);
}

#[test]
fn guest_export_excludes_personal_and_restricted_records() {
    let entities = [
        ("Public", DataClass::Public),
        ("Internal", DataClass::Internal),
        ("Personal", DataClass::PersonalSensitive),
        ("Restricted", DataClass::Restricted),
    ]
    .map(|(title, data_class)| {
        Entity::Category(Category {
            meta: meta("category", title, data_class),
            color: None,
            sort_order: 0,
        })
    });
    let snapshot = Snapshot::from_entities(entities).unwrap();
    let exported = snapshot
        .exportable_entities(ExportAudience::Guest)
        .map(|entity| entity.meta().title.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(exported, BTreeSet::from(["Internal", "Public"]));
}

#[test]
fn registry_greeting_remains_an_intent_until_qream_grants_it() {
    let capability = CapabilityRow::new("Package install", "Install a registered package").unwrap();
    let app = AppRegistryRow::new(
        "Ripgrep",
        "ripgrep",
        capability.id.clone(),
        AppState::Missing,
        true,
        vec![PackageAction::Install],
    )
    .unwrap();
    let mut registry = AppRegistry::default();
    registry.register_capability(capability.clone()).unwrap();
    registry.register_app(app).unwrap();
    let proposal = registry.package_greeting("first-run").unwrap().remove(0);

    assert!(QreamGate::authorize(&proposal.intent, None, None).is_err());
    let verdict = Verdict::approve(&proposal.intent, ContentHash::digest(b"policy")).unwrap();
    assert!(QreamGate::authorize(&proposal.intent, Some(&verdict), None).is_err());
    let grant = Grant::from_approved_verdict(
        &proposal.intent,
        &verdict,
        [capability.id].into_iter().collect(),
    )
    .unwrap();
    let command = QreamGate::authorize(&proposal.intent, Some(&verdict), Some(&grant)).unwrap();
    assert_eq!(command.target(), "ripgrep");
}
