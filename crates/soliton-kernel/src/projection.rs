use std::collections::BTreeMap;

use crate::date::CivilDate;
use crate::error::Result;
use crate::id::{ObjectId, OccurrenceId};
use crate::model::{DataClass, Entity, EntityKind, ReminderState, RoutineState, TaskStatus};
use crate::recurrence::materialize_occurrences;
use crate::repository::Snapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionItem {
    pub source_id: ObjectId,
    pub occurrence_id: Option<OccurrenceId>,
    pub kind: EntityKind,
    pub title: String,
    pub on: Option<CivilDate>,
    pub state: String,
    pub data_class: DataClass,
}

pub type KanbanProjection = BTreeMap<TaskStatus, Vec<ProjectionItem>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarProjection {
    pub from: CivilDate,
    pub through: CivilDate,
    pub items: Vec<ProjectionItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AimRollup {
    pub total_tasks: u32,
    pub completed_tasks: u32,
}

impl AimRollup {
    #[must_use]
    pub fn completion_basis_points(self) -> u16 {
        if self.total_tasks == 0 {
            return 0;
        }
        let value = u64::from(self.completed_tasks) * 10_000 / u64::from(self.total_tasks);
        u16::try_from(value).unwrap_or(10_000)
    }
}

#[must_use]
pub fn list_projection(snapshot: &Snapshot) -> Vec<ProjectionItem> {
    let mut items = snapshot
        .entities()
        .map(|entity| ProjectionItem {
            source_id: entity.id().clone(),
            occurrence_id: None,
            kind: entity.kind(),
            title: entity.meta().title.clone(),
            on: primary_date(entity),
            state: entity_state(entity),
            data_class: entity.meta().data_class,
        })
        .collect::<Vec<_>>();
    sort_projection_items(&mut items);
    items
}

#[must_use]
pub fn kanban_projection(snapshot: &Snapshot) -> KanbanProjection {
    let mut lanes = TaskStatus::all()
        .into_iter()
        .map(|status| (status, Vec::new()))
        .collect::<KanbanProjection>();
    for entity in snapshot.entities() {
        let Entity::Task(task) = entity else {
            continue;
        };
        lanes
            .get_mut(&task.status)
            .expect("all task states have a lane")
            .push(ProjectionItem {
                source_id: task.meta.id.clone(),
                occurrence_id: None,
                kind: EntityKind::Task,
                title: task.meta.title.clone(),
                on: task.scheduled_for.or(task.due_on),
                state: task.status.as_str().into(),
                data_class: task.meta.data_class,
            });
    }
    for items in lanes.values_mut() {
        sort_projection_items(items);
    }
    lanes
}

pub fn calendar_week(snapshot: &Snapshot, containing: CivilDate) -> Result<CalendarProjection> {
    calendar_between(
        snapshot,
        containing.start_of_week()?,
        containing.end_of_week()?,
    )
}

pub fn calendar_month(snapshot: &Snapshot, containing: CivilDate) -> Result<CalendarProjection> {
    calendar_between(
        snapshot,
        containing.start_of_month()?,
        containing.end_of_month()?,
    )
}

pub fn calendar_year(snapshot: &Snapshot, containing: CivilDate) -> Result<CalendarProjection> {
    calendar_between(
        snapshot,
        containing.start_of_year()?,
        containing.end_of_year()?,
    )
}

pub fn calendar_between(
    snapshot: &Snapshot,
    from: CivilDate,
    through: CivilDate,
) -> Result<CalendarProjection> {
    let mut items = Vec::new();
    for entity in snapshot.entities() {
        append_calendar_items(entity, from, through, &mut items)?;
    }
    sort_projection_items(&mut items);
    Ok(CalendarProjection {
        from,
        through,
        items,
    })
}

#[must_use]
pub fn aim_rollup(snapshot: &Snapshot, aim_id: &ObjectId) -> AimRollup {
    let mut total_tasks = 0_u32;
    let mut completed_tasks = 0_u32;
    for entity in snapshot.entities() {
        let Entity::Task(task) = entity else {
            continue;
        };
        if task.aim_id.as_ref() != Some(aim_id) || task.status == TaskStatus::Archived {
            continue;
        }
        total_tasks = total_tasks.saturating_add(1);
        if task.status == TaskStatus::Completed {
            completed_tasks = completed_tasks.saturating_add(1);
        }
    }
    AimRollup {
        total_tasks,
        completed_tasks,
    }
}

fn append_calendar_items(
    entity: &Entity,
    from: CivilDate,
    through: CivilDate,
    output: &mut Vec<ProjectionItem>,
) -> Result<()> {
    match entity {
        Entity::Category(_) | Entity::Document(_) => {}
        Entity::Aim(aim) => {
            if aim
                .target_on
                .is_some_and(|date| (from..=through).contains(&date))
            {
                output.push(item(entity, None, aim.target_on, aim.state.as_str()));
            }
        }
        Entity::Task(task) => {
            if let Some(rule) = &task.recurrence {
                for occurrence in materialize_occurrences(&task.meta.id, rule, from, through)? {
                    let state = if task.completed_occurrences.contains(&occurrence.id) {
                        TaskStatus::Completed.as_str()
                    } else {
                        task.status.as_str()
                    };
                    output.push(item(
                        entity,
                        Some(occurrence.id),
                        Some(occurrence.on),
                        state,
                    ));
                }
            } else if task
                .scheduled_for
                .or(task.due_on)
                .is_some_and(|date| (from..=through).contains(&date))
            {
                output.push(item(
                    entity,
                    None,
                    task.scheduled_for.or(task.due_on),
                    task.status.as_str(),
                ));
            }
        }
        Entity::Reminder(reminder) => {
            if let Some(rule) = &reminder.recurrence {
                for occurrence in materialize_occurrences(&reminder.meta.id, rule, from, through)? {
                    let state = if reminder.completed_occurrences.contains(&occurrence.id) {
                        ReminderState::Completed.as_str()
                    } else {
                        reminder.state.as_str()
                    };
                    output.push(item(
                        entity,
                        Some(occurrence.id),
                        Some(occurrence.on),
                        state,
                    ));
                }
            } else if (from..=through).contains(&reminder.scheduled_for) {
                output.push(item(
                    entity,
                    None,
                    Some(reminder.scheduled_for),
                    reminder.state.as_str(),
                ));
            }
        }
        Entity::Routine(routine) => {
            for occurrence in
                materialize_occurrences(&routine.meta.id, &routine.recurrence, from, through)?
            {
                let state = if routine.completed_occurrences.contains(&occurrence.id) {
                    "completed"
                } else {
                    routine.state.as_str()
                };
                output.push(item(
                    entity,
                    Some(occurrence.id),
                    Some(occurrence.on),
                    state,
                ));
            }
        }
    }
    Ok(())
}

fn item(
    entity: &Entity,
    occurrence_id: Option<OccurrenceId>,
    on: Option<CivilDate>,
    state: &str,
) -> ProjectionItem {
    ProjectionItem {
        source_id: entity.id().clone(),
        occurrence_id,
        kind: entity.kind(),
        title: entity.meta().title.clone(),
        on,
        state: state.into(),
        data_class: entity.meta().data_class,
    }
}

fn primary_date(entity: &Entity) -> Option<CivilDate> {
    match entity {
        Entity::Category(_) | Entity::Routine(_) | Entity::Document(_) => None,
        Entity::Aim(aim) => aim.target_on,
        Entity::Task(task) => task.scheduled_for.or(task.due_on),
        Entity::Reminder(reminder) => Some(reminder.scheduled_for),
    }
}

fn entity_state(entity: &Entity) -> String {
    match entity {
        Entity::Category(_) | Entity::Document(_) => "active",
        Entity::Aim(aim) => aim.state.as_str(),
        Entity::Task(task) => task.status.as_str(),
        Entity::Reminder(reminder) => reminder.state.as_str(),
        Entity::Routine(routine) => routine.state.as_str(),
    }
    .into()
}

fn sort_projection_items(items: &mut [ProjectionItem]) {
    items.sort_by(|left, right| {
        left.on
            .cmp(&right.on)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.occurrence_id.cmp(&right.occurrence_id))
    });
}

impl TaskStatus {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::Todo,
            Self::InProgress,
            Self::Completed,
            Self::Archived,
        ]
    }
}

impl RoutineState {
    #[must_use]
    pub const fn is_visible(self) -> bool {
        !matches!(self, Self::Archived)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::model::{Aim, AimState, ObjectMeta, Priority, Task};
    use crate::recurrence::{RecurrenceCadence, RecurrenceRule};

    fn meta(kind: &str, title: &str) -> ObjectMeta {
        ObjectMeta {
            details: Default::default(),
            id: ObjectId::derive(kind, title.as_bytes()).unwrap(),
            title: title.into(),
            data_class: DataClass::Internal,
            revision: 1,
        }
    }

    fn task(title: &str, status: TaskStatus, on: CivilDate) -> Entity {
        Entity::Task(Task {
            meta: meta("task", title),
            category_id: None,
            aim_id: None,
            priority: Priority::Medium,
            status,
            scheduled_for: Some(on),
            due_on: None,
            recurrence: None,
            completed_occurrences: BTreeSet::new(),
        })
    }

    #[test]
    fn list_kanban_and_week_retain_the_same_source_id() {
        let on = CivilDate::new(2026, 8, 3).unwrap();
        let entity = task("One body", TaskStatus::Todo, on);
        let id = entity.id().clone();
        let snapshot = Snapshot::from_entities([entity]).unwrap();
        assert_eq!(list_projection(&snapshot)[0].source_id, id);
        assert_eq!(
            kanban_projection(&snapshot)[&TaskStatus::Todo][0].source_id,
            id
        );
        assert_eq!(calendar_week(&snapshot, on).unwrap().items[0].source_id, id);
    }

    #[test]
    fn recurring_items_materialize_in_calendar_without_cloning_authority() {
        let start = CivilDate::new(2026, 8, 3).unwrap();
        let mut entity = task("Daily", TaskStatus::Todo, start);
        let Entity::Task(task) = &mut entity else {
            unreachable!();
        };
        task.recurrence = Some(RecurrenceRule {
            starts_on: start,
            cadence: RecurrenceCadence::Daily,
            interval: 1,
            ends_on: None,
            max_occurrences: Some(3),
            exclusions: BTreeSet::new(),
        });
        let id = task.meta.id.clone();
        let snapshot = Snapshot::from_entities([entity]).unwrap();
        let week = calendar_week(&snapshot, start).unwrap();
        assert_eq!(week.items.len(), 3);
        assert!(week.items.iter().all(|item| item.source_id == id));
        assert_eq!(snapshot.len(), 1);
    }

    #[test]
    fn aim_progress_is_derived_from_tasks() {
        let aim = Entity::Aim(Aim {
            folder: None,
            meta: meta("aim", "Deliver"),
            category_id: None,
            state: AimState::Active,
            target_on: None,
        });
        let aim_id = aim.id().clone();
        let mut first = task(
            "Done",
            TaskStatus::Completed,
            CivilDate::new(2026, 8, 1).unwrap(),
        );
        let mut second = task(
            "Next",
            TaskStatus::Todo,
            CivilDate::new(2026, 8, 2).unwrap(),
        );
        for entity in [&mut first, &mut second] {
            let Entity::Task(task) = entity else {
                unreachable!();
            };
            task.aim_id = Some(aim_id.clone());
        }
        let snapshot = Snapshot::from_entities([aim, first, second]).unwrap();
        let rollup = aim_rollup(&snapshot, &aim_id);
        assert_eq!(rollup.total_tasks, 2);
        assert_eq!(rollup.completed_tasks, 1);
        assert_eq!(rollup.completion_basis_points(), 5_000);
    }

    #[test]
    fn month_and_year_have_exact_calendar_bounds() {
        let snapshot = Snapshot::default();
        let date = CivilDate::new(2024, 2, 10).unwrap();
        let month = calendar_month(&snapshot, date).unwrap();
        assert_eq!(month.from.to_string(), "2024-02-01");
        assert_eq!(month.through.to_string(), "2024-02-29");
        let year = calendar_year(&snapshot, date).unwrap();
        assert_eq!(year.from.to_string(), "2024-01-01");
        assert_eq!(year.through.to_string(), "2024-12-31");
    }
}
