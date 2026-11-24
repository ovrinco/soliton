use std::collections::BTreeMap;

use crate::error::{KernelError, Result};
use crate::hash::ContentHash;
use crate::id::ObjectId;
use crate::model::DataClass;
use crate::qream::{ConsequentialAction, ConsequentialIntent};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityRow {
    pub id: ObjectId,
    pub label: String,
    pub description: String,
    pub data_class: DataClass,
}

impl CapabilityRow {
    pub fn new(label: impl Into<String>, description: impl Into<String>) -> Result<Self> {
        let label = label.into();
        let description = description.into();
        if label.trim().is_empty() {
            return Err(KernelError::Registry(
                "capability label cannot be empty".into(),
            ));
        }
        Ok(Self {
            id: ObjectId::derive("capability", label.as_bytes())?,
            label,
            description,
            data_class: DataClass::Internal,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppState {
    Installed,
    Missing,
    Disabled,
    Deferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageAction {
    Install,
    Enable,
    Configure,
}

impl PackageAction {
    #[must_use]
    pub const fn consequential_action(self) -> ConsequentialAction {
        match self {
            Self::Install => ConsequentialAction::InstallPackage,
            Self::Enable => ConsequentialAction::EnableService,
            Self::Configure => ConsequentialAction::ChangeConfiguration,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppRegistryRow {
    pub id: ObjectId,
    pub display_name: String,
    pub package_name: String,
    pub capability_id: ObjectId,
    pub state: AppState,
    pub default_app: bool,
    pub greeting_actions: Vec<PackageAction>,
    pub data_class: DataClass,
}

impl AppRegistryRow {
    pub fn new(
        display_name: impl Into<String>,
        package_name: impl Into<String>,
        capability_id: ObjectId,
        state: AppState,
        default_app: bool,
        greeting_actions: Vec<PackageAction>,
    ) -> Result<Self> {
        let display_name = display_name.into();
        let package_name = package_name.into();
        if display_name.trim().is_empty() || package_name.trim().is_empty() {
            return Err(KernelError::Registry(
                "app display and package names must be non-empty".into(),
            ));
        }
        if capability_id.kind() != "capability" {
            return Err(KernelError::Registry(
                "app capability_id is not a capability identifier".into(),
            ));
        }
        let seed = format!("{display_name}\0{package_name}");
        Ok(Self {
            id: ObjectId::derive("app", seed.as_bytes())?,
            display_name,
            package_name,
            capability_id,
            state,
            default_app,
            greeting_actions,
            data_class: DataClass::Internal,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppCard {
    pub app_id: ObjectId,
    pub display_name: String,
    pub capability_label: String,
    pub state: AppState,
    pub default_app: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageGreetingProposal {
    pub app_id: ObjectId,
    pub action: PackageAction,
    pub intent: ConsequentialIntent,
}

#[derive(Clone, Debug, Default)]
pub struct AppRegistry {
    capabilities: BTreeMap<ObjectId, CapabilityRow>,
    apps: BTreeMap<ObjectId, AppRegistryRow>,
}

impl AppRegistry {
    pub fn register_capability(&mut self, row: CapabilityRow) -> Result<()> {
        if row.id.kind() != "capability" {
            return Err(KernelError::Registry(
                "capability row has the wrong identifier kind".into(),
            ));
        }
        if self.capabilities.insert(row.id.clone(), row).is_some() {
            return Err(KernelError::Registry(
                "capability row is already registered".into(),
            ));
        }
        Ok(())
    }

    pub fn register_app(&mut self, row: AppRegistryRow) -> Result<()> {
        if !self.capabilities.contains_key(&row.capability_id) {
            return Err(KernelError::Registry(format!(
                "app {} references an unregistered capability",
                row.display_name
            )));
        }
        if self.apps.insert(row.id.clone(), row).is_some() {
            return Err(KernelError::Registry(
                "app row is already registered".into(),
            ));
        }
        Ok(())
    }

    pub fn card(&self, app_id: &ObjectId) -> Result<AppCard> {
        let app = self.apps.get(app_id).ok_or_else(|| {
            KernelError::Registry(format!("no registry row exists for app {app_id}"))
        })?;
        let capability = self.capabilities.get(&app.capability_id).ok_or_else(|| {
            KernelError::Registry(format!(
                "registered app {app_id} has no registered capability"
            ))
        })?;
        Ok(AppCard {
            app_id: app.id.clone(),
            display_name: app.display_name.clone(),
            capability_label: capability.label.clone(),
            state: app.state,
            default_app: app.default_app,
        })
    }

    /// Produces inert QREAM intents. This API has no executor or command callback.
    pub fn package_greeting(&self, actor: &str) -> Result<Vec<PackageGreetingProposal>> {
        let mut proposals = Vec::new();
        for app in self.apps.values().filter(|app| app.default_app) {
            for action in &app.greeting_actions {
                if !action_applies(*action, app.state) {
                    continue;
                }
                let payload = format!("{}\0{}", app.package_name, action_name(*action));
                let intent = ConsequentialIntent::new(
                    actor,
                    action.consequential_action(),
                    app.package_name.clone(),
                    [app.capability_id.clone()].into_iter().collect(),
                    ContentHash::digest(payload.as_bytes()),
                    app.data_class,
                )?;
                proposals.push(PackageGreetingProposal {
                    app_id: app.id.clone(),
                    action: *action,
                    intent,
                });
            }
        }
        Ok(proposals)
    }

    #[must_use]
    pub fn app_count(&self) -> usize {
        self.apps.len()
    }
}

fn action_applies(action: PackageAction, state: AppState) -> bool {
    matches!(
        (action, state),
        (PackageAction::Install, AppState::Missing)
            | (PackageAction::Enable, AppState::Disabled)
            | (PackageAction::Configure, AppState::Installed)
    )
}

fn action_name(action: PackageAction) -> &'static str {
    match action {
        PackageAction::Install => "install",
        PackageAction::Enable => "enable",
        PackageAction::Configure => "configure",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn app_registration_requires_capability_first() {
        let capability = CapabilityRow::new("Package install", "Install a known package").unwrap();
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
        assert!(registry.register_app(app.clone()).is_err());
        registry.register_capability(capability).unwrap();
        registry.register_app(app).unwrap();
    }

    #[test]
    fn card_requires_a_registered_row() {
        let registry = AppRegistry::default();
        let unknown = ObjectId::derive("app", b"unknown").unwrap();
        assert!(registry.card(&unknown).is_err());
    }

    #[test]
    fn registered_row_can_render_a_card() {
        let capability = CapabilityRow::new("Editor", "Open the configured editor").unwrap();
        let app = AppRegistryRow::new(
            "Helix",
            "helix",
            capability.id.clone(),
            AppState::Installed,
            true,
            vec![],
        )
        .unwrap();
        let id = app.id.clone();
        let mut registry = AppRegistry::default();
        registry.register_capability(capability).unwrap();
        registry.register_app(app).unwrap();
        assert_eq!(registry.card(&id).unwrap().display_name, "Helix");
    }

    #[test]
    fn greeting_proposes_but_cannot_execute() {
        static EXECUTIONS: AtomicUsize = AtomicUsize::new(0);

        let capability = CapabilityRow::new("Package install", "Install a known package").unwrap();
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
        registry.register_capability(capability).unwrap();
        registry.register_app(app).unwrap();
        let proposals = registry.package_greeting("first-run").unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].intent.target, "ripgrep");
        assert_eq!(EXECUTIONS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn deferred_and_nondefault_apps_do_not_greet() {
        let capability = CapabilityRow::new("Package install", "Install a known package").unwrap();
        let deferred = AppRegistryRow::new(
            "Deferred",
            "deferred",
            capability.id.clone(),
            AppState::Deferred,
            true,
            vec![PackageAction::Install],
        )
        .unwrap();
        let optional = AppRegistryRow::new(
            "Optional",
            "optional",
            capability.id.clone(),
            AppState::Missing,
            false,
            vec![PackageAction::Install],
        )
        .unwrap();
        let mut registry = AppRegistry::default();
        registry.register_capability(capability).unwrap();
        registry.register_app(deferred).unwrap();
        registry.register_app(optional).unwrap();
        assert!(registry.package_greeting("first-run").unwrap().is_empty());
    }
}
