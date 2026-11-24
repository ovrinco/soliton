#![forbid(unsafe_code)]

//! A Tauri-independent kernel for Soliton's governed object spine.
//!
//! The crate deliberately owns no UI, process execution, or network
//! integration. Its one database is the observation ledger (`ledger`), a
//! SQLite file beside the object files that the planner never reads to
//! function. It provides authoritative object contracts, deterministic
//! recurrence and projections, a crash-recoverable disk repository, an inert
//! app registry, and a fail-closed QREAM authorization boundary.

pub mod date;
pub mod error;
pub mod hash;
pub mod id;
pub mod ledger;
pub mod model;
pub mod now;
pub mod projection;
pub mod qream;
pub mod recurrence;
pub mod registry;
pub mod repository;

pub use date::{CivilDate, Weekday};
pub use error::{KernelError, Result};
pub use hash::ContentHash;
pub use id::{ObjectId, OccurrenceId};
pub use ledger::{Answer, KindDef, Ledger, MarkScope, NewObservation, Question, Table};
pub use model::{
    Aim, AimState, Category, DataClass, Entity, EntityKind, ObjectMeta, Priority, Reminder,
    ReminderState, Routine, RoutineState, Task, TaskStatus,
};
pub use now::{ClockTime, NowItem, NowKind, NowList, NowSettings, NowSource, NowWhy, SourceKind};
pub use projection::{
    aim_rollup, calendar_month, calendar_week, calendar_year, kanban_projection, list_projection,
    AimRollup, CalendarProjection, KanbanProjection, ProjectionItem,
};
pub use qream::{
    AuthorizedCommand, ConsequentialAction, ConsequentialIntent, Grant, QreamGate, Verdict,
    VerdictDecision,
};
pub use recurrence::{
    materialize_occurrences, MonthlyPattern, Occurrence, RecurrenceCadence, RecurrenceCursor,
    RecurrenceCursorStatus, RecurrenceRule, WeekOrdinal,
};
pub use registry::{
    AppCard, AppRegistry, AppRegistryRow, AppState, CapabilityRow, PackageAction,
    PackageGreetingProposal,
};
pub use repository::{
    ExportAudience, GovernedRepository, JournalEntry, MutationPrecondition, PreparedMutation,
    RecoveryReport, Snapshot, StoredEntity,
};

pub mod details;
pub use model::Document;

pub mod trash;

#[cfg(all(test, feature = "bundled-sqlite"))]
mod bundled_sqlite_security {
    #[test]
    fn bundled_sqlite_contains_wal_page_size_fix() {
        assert!(
            rusqlite::version_number() >= 3_054_000,
            "The bundled feature must include SQLite 3.54.0 or newer"
        );
    }
}
