use std::collections::BTreeSet;

use crate::error::{KernelError, Result};
use crate::hash::ContentHash;
use crate::id::ObjectId;
use crate::model::DataClass;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsequentialAction {
    InstallPackage,
    EnableService,
    ChangeConfiguration,
    MutateGovernedObject,
    LaunchRun,
    RemoveResource,
}

impl ConsequentialAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InstallPackage => "install_package",
            Self::EnableService => "enable_service",
            Self::ChangeConfiguration => "change_configuration",
            Self::MutateGovernedObject => "mutate_governed_object",
            Self::LaunchRun => "launch_run",
            Self::RemoveResource => "remove_resource",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsequentialIntent {
    pub id: ObjectId,
    pub actor: String,
    pub action: ConsequentialAction,
    pub target: String,
    pub required_capabilities: BTreeSet<ObjectId>,
    pub payload_hash: ContentHash,
    pub data_class: DataClass,
}

impl ConsequentialIntent {
    pub fn new(
        actor: impl Into<String>,
        action: ConsequentialAction,
        target: impl Into<String>,
        required_capabilities: BTreeSet<ObjectId>,
        payload_hash: ContentHash,
        data_class: DataClass,
    ) -> Result<Self> {
        let actor = actor.into();
        let target = target.into();
        if actor.trim().is_empty() || target.trim().is_empty() {
            return Err(KernelError::Authorization(
                "intent actor and target must be non-empty".into(),
            ));
        }
        if required_capabilities.is_empty() {
            return Err(KernelError::Authorization(
                "consequential intent must name at least one required capability".into(),
            ));
        }
        if required_capabilities
            .iter()
            .any(|capability| capability.kind() != "capability")
        {
            return Err(KernelError::Authorization(
                "intent contains a non-capability identifier".into(),
            ));
        }
        let seed = canonical_intent_material(
            &actor,
            action,
            &target,
            &required_capabilities,
            payload_hash,
            data_class,
        );
        Ok(Self {
            id: ObjectId::derive("intent", &seed)?,
            actor,
            action,
            target,
            required_capabilities,
            payload_hash,
            data_class,
        })
    }

    #[must_use]
    pub fn semantic_hash(&self) -> ContentHash {
        ContentHash::digest(&canonical_intent_material(
            &self.actor,
            self.action,
            &self.target,
            &self.required_capabilities,
            self.payload_hash,
            self.data_class,
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerdictDecision {
    Approved,
    Denied,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verdict {
    pub id: ObjectId,
    pub intent_id: ObjectId,
    pub intent_hash: ContentHash,
    pub decision: VerdictDecision,
    pub evidence_hash: ContentHash,
}

impl Verdict {
    pub fn approve(intent: &ConsequentialIntent, evidence_hash: ContentHash) -> Result<Self> {
        Self::for_intent(intent, VerdictDecision::Approved, evidence_hash)
    }

    pub fn deny(intent: &ConsequentialIntent, evidence_hash: ContentHash) -> Result<Self> {
        Self::for_intent(intent, VerdictDecision::Denied, evidence_hash)
    }

    fn for_intent(
        intent: &ConsequentialIntent,
        decision: VerdictDecision,
        evidence_hash: ContentHash,
    ) -> Result<Self> {
        let seed = format!(
            "{}\0{}\0{}\0{}",
            intent.id,
            intent.semantic_hash(),
            match decision {
                VerdictDecision::Approved => "approved",
                VerdictDecision::Denied => "denied",
            },
            evidence_hash
        );
        Ok(Self {
            id: ObjectId::derive("verdict", seed.as_bytes())?,
            intent_id: intent.id.clone(),
            intent_hash: intent.semantic_hash(),
            decision,
            evidence_hash,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Grant {
    pub id: ObjectId,
    pub intent_id: ObjectId,
    pub intent_hash: ContentHash,
    pub verdict_id: ObjectId,
    pub capabilities: BTreeSet<ObjectId>,
    pub revoked: bool,
}

impl Grant {
    pub fn from_approved_verdict(
        intent: &ConsequentialIntent,
        verdict: &Verdict,
        capabilities: BTreeSet<ObjectId>,
    ) -> Result<Self> {
        if verdict.decision != VerdictDecision::Approved
            || verdict.intent_id != intent.id
            || verdict.intent_hash != intent.semantic_hash()
        {
            return Err(KernelError::Authorization(
                "grant can only be derived from the matching approved verdict".into(),
            ));
        }
        let seed = format!(
            "{}\0{}\0{}",
            intent.id,
            verdict.id,
            capabilities
                .iter()
                .map(ObjectId::as_str)
                .collect::<Vec<_>>()
                .join(",")
        );
        Ok(Self {
            id: ObjectId::derive("grant", seed.as_bytes())?,
            intent_id: intent.id.clone(),
            intent_hash: intent.semantic_hash(),
            verdict_id: verdict.id.clone(),
            capabilities,
            revoked: false,
        })
    }
}

/// A command token can only be constructed by `QreamGate::authorize`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedCommand {
    intent_id: ObjectId,
    grant_id: ObjectId,
    action: ConsequentialAction,
    target: String,
    payload_hash: ContentHash,
}

impl AuthorizedCommand {
    #[must_use]
    pub fn intent_id(&self) -> &ObjectId {
        &self.intent_id
    }

    #[must_use]
    pub fn grant_id(&self) -> &ObjectId {
        &self.grant_id
    }

    #[must_use]
    pub const fn action(&self) -> ConsequentialAction {
        self.action
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    #[must_use]
    pub const fn payload_hash(&self) -> ContentHash {
        self.payload_hash
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct QreamGate;

impl QreamGate {
    pub fn authorize(
        intent: &ConsequentialIntent,
        verdict: Option<&Verdict>,
        grant: Option<&Grant>,
    ) -> Result<AuthorizedCommand> {
        let verdict = verdict.ok_or_else(|| {
            KernelError::Authorization("no QREAM verdict accompanied the intent".into())
        })?;
        let grant = grant.ok_or_else(|| {
            KernelError::Authorization("no capability grant accompanied the intent".into())
        })?;
        let intent_hash = intent.semantic_hash();
        if verdict.decision != VerdictDecision::Approved {
            return Err(KernelError::Authorization(
                "QREAM verdict denied the intent".into(),
            ));
        }
        if verdict.intent_id != intent.id || verdict.intent_hash != intent_hash {
            return Err(KernelError::Authorization(
                "QREAM verdict does not bind this exact intent".into(),
            ));
        }
        if grant.revoked {
            return Err(KernelError::Authorization(
                "capability grant has been revoked".into(),
            ));
        }
        if grant.intent_id != intent.id
            || grant.intent_hash != intent_hash
            || grant.verdict_id != verdict.id
        {
            return Err(KernelError::Authorization(
                "capability grant does not bind the intent and verdict".into(),
            ));
        }
        if !intent.required_capabilities.is_subset(&grant.capabilities) {
            return Err(KernelError::Authorization(
                "capability grant does not cover every requested capability".into(),
            ));
        }
        Ok(AuthorizedCommand {
            intent_id: intent.id.clone(),
            grant_id: grant.id.clone(),
            action: intent.action,
            target: intent.target.clone(),
            payload_hash: intent.payload_hash,
        })
    }
}

fn canonical_intent_material(
    actor: &str,
    action: ConsequentialAction,
    target: &str,
    capabilities: &BTreeSet<ObjectId>,
    payload_hash: ContentHash,
    data_class: DataClass,
) -> Vec<u8> {
    format!(
        "soliton-intent/1\nactor={actor}\naction={}\ntarget={target}\ncapabilities={}\npayload_hash={payload_hash}\ndata_class={}\n",
        action.as_str(),
        capabilities
            .iter()
            .map(ObjectId::as_str)
            .collect::<Vec<_>>()
            .join(","),
        data_class.as_str()
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (ConsequentialIntent, Verdict, Grant) {
        let capability = ObjectId::derive("capability", b"package.install").unwrap();
        let intent = ConsequentialIntent::new(
            "operator",
            ConsequentialAction::InstallPackage,
            "ripgrep",
            [capability.clone()].into_iter().collect(),
            ContentHash::digest(b"ripgrep@known-version"),
            DataClass::Internal,
        )
        .unwrap();
        let verdict = Verdict::approve(&intent, ContentHash::digest(b"policy evidence")).unwrap();
        let grant =
            Grant::from_approved_verdict(&intent, &verdict, [capability].into_iter().collect())
                .unwrap();
        (intent, verdict, grant)
    }

    #[test]
    fn matching_verdict_and_grant_authorize_exact_command() {
        let (intent, verdict, grant) = fixture();
        let command = QreamGate::authorize(&intent, Some(&verdict), Some(&grant)).unwrap();
        assert_eq!(command.intent_id(), &intent.id);
        assert_eq!(command.grant_id(), &grant.id);
        assert_eq!(command.target(), "ripgrep");
    }

    #[test]
    fn missing_verdict_fails_closed() {
        let (intent, _, grant) = fixture();
        assert!(QreamGate::authorize(&intent, None, Some(&grant)).is_err());
    }

    #[test]
    fn missing_grant_fails_closed() {
        let (intent, verdict, _) = fixture();
        assert!(QreamGate::authorize(&intent, Some(&verdict), None).is_err());
    }

    #[test]
    fn denied_verdict_fails_closed() {
        let (intent, _, grant) = fixture();
        let denial = Verdict::deny(&intent, ContentHash::digest(b"denial")).unwrap();
        assert!(QreamGate::authorize(&intent, Some(&denial), Some(&grant)).is_err());
    }

    #[test]
    fn revoked_grant_fails_closed() {
        let (intent, verdict, mut grant) = fixture();
        grant.revoked = true;
        assert!(QreamGate::authorize(&intent, Some(&verdict), Some(&grant)).is_err());
    }

    #[test]
    fn payload_tampering_invalidates_prior_authority() {
        let (mut intent, verdict, grant) = fixture();
        intent.payload_hash = ContentHash::digest(b"different package bytes");
        assert!(QreamGate::authorize(&intent, Some(&verdict), Some(&grant)).is_err());
    }

    #[test]
    fn insufficient_capabilities_fail_closed() {
        let (intent, verdict, mut grant) = fixture();
        grant.capabilities.clear();
        assert!(QreamGate::authorize(&intent, Some(&verdict), Some(&grant)).is_err());
    }

    #[test]
    fn foreign_verdict_and_grant_fail_closed() {
        let (intent, _, _) = fixture();
        let foreign_capability = ObjectId::derive("capability", b"service.enable").unwrap();
        let foreign = ConsequentialIntent::new(
            "operator",
            ConsequentialAction::EnableService,
            "indexer",
            [foreign_capability.clone()].into_iter().collect(),
            ContentHash::digest(b"service"),
            DataClass::Internal,
        )
        .unwrap();
        let verdict = Verdict::approve(&foreign, ContentHash::digest(b"evidence")).unwrap();
        let grant = Grant::from_approved_verdict(
            &foreign,
            &verdict,
            [foreign_capability].into_iter().collect(),
        )
        .unwrap();
        assert!(QreamGate::authorize(&intent, Some(&verdict), Some(&grant)).is_err());
    }
}
