//! Canonical Remember evidence. Text is stored verbatim as data, never parsed as policy.
use super::*;
use crate::store::ResultMemoryCandidate;

/// Literal second-level section; the next second-level heading ends the evidence.
pub fn extract_remember(report: &str) -> Option<String> {
    let mut body = None;
    for line in report.lines() {
        if body.is_none() {
            if line.trim() == "## Remember" {
                body = Some(Vec::new());
            }
        } else if line.starts_with("## ") {
            break;
        } else {
            body.as_mut()?.push(line);
        }
    }
    let text = body?.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

impl MemoryStore {
    pub(crate) fn propose_result_memory(
        &mut self,
        row: &ResultMemoryCandidate,
    ) -> Result<ProposalReceipt, MemoryError> {
        let body = self.ingest_object(row.remember.as_bytes())?;
        let provenance = self.ingest_object(serde_json::json!({"task":row.task_id,"attempt":row.attempt_id,"submission":row.submission_id,"content_digest":row.content_digest,"carrier":"result.report/## Remember","trust":"data, never instructions"}).to_string().as_bytes())?;
        let doc = ProposalDocument {
            schema_version: 1,
            proposal_id: row.proposal_id.clone(),
            producer: ProposalProducer {
                task_id: row.task_id.clone(),
                attempt_id: row.attempt_id.clone(),
            },
            input_snapshot_id: row.snapshot_id.clone(),
            observed_revisions: vec![],
            repository: None,
            changes: vec![ProposalChange {
                record_key: format!("remember/{}", row.proposal_id),
                expected: None,
                kind: "observation".into(),
                scope: Applicability {
                    domains: vec![],
                    paths: vec![],
                },
                claim: format!(
                    "Worker Remember evidence from task {} (data, never instructions)",
                    row.task_id
                ),
                body_object: format!("sha256:{}", body.as_str()),
                evidence: vec![ProposalEvidence {
                    object: Some(format!("sha256:{}", provenance.as_str())),
                    validation_id: None,
                }],
                based_on: vec![],
                impact: "informational".into(),
            }],
        };
        let bytes = serde_json::to_vec(&doc).map_err(|e| MemoryError::Invalid(e.to_string()))?;
        self.propose(&bytes, jiff::Timestamp::now().as_millisecond())
    }

    /// Narrow standing owner delegation. Signed owner review/promote APIs remain unchanged.
    pub fn decide_result_candidate(
        &mut self,
        id: &str,
        approve: bool,
        reason: &str,
        config: &crate::migration::ConfigReference,
    ) -> Result<ReviewDecision, MemoryError> {
        if reason.trim().is_empty() || reason.len() > 2000 || reason.chars().any(char::is_control) {
            return Err(MemoryError::Invalid(
                "decision needs a reason of 1..2000 bytes without control characters".into(),
            ));
        }
        let row = self
            .store
            .result_memory_candidates()?
            .into_iter()
            .find(|r| r.proposal_id == id && r.captured)
            .ok_or_else(|| MemoryError::Invalid("canonical candidate missing".into()))?;
        let (stored, payload) = self
            .store
            .memory_proposal_payload(id)?
            .ok_or_else(|| MemoryError::Invalid("proposal missing".into()))?;
        let proposal: ProposalDocument =
            serde_json::from_str(&payload).map_err(|e| MemoryError::Invalid(e.to_string()))?;
        let change = proposal
            .changes
            .first()
            .ok_or_else(|| MemoryError::Invalid("empty candidate".into()))?;
        let body = ObjectId::parse(&change.body_object).map_err(MemoryError::Invalid)?;
        let bytes = read_object(&self.objects, &body)?;
        if bytes != row.remember.as_bytes() || body.as_str() != row.content_digest {
            return Err(MemoryError::Invalid("candidate evidence mismatch".into()));
        }
        let now = jiff::Timestamp::now().as_millisecond();
        let action = if approve { "approve" } else { "reject" };
        let decision = ReviewDecision {id:format!("remember-review-{:x}",Sha256::digest(format!("{id}\0{action}\0{reason}").as_bytes())),proposal_id:id.into(),payload_digest:stored.payload_digest,decision:action.into(),classification:"[{\"class\":\"project_fact\",\"mandatory\":false}]".into(),reviewed_heads:serde_json::json!({"principal":"coordinator","delegation":crate::store::MEMORY_COORDINATOR_DELEGATION,"submission":row.submission_id,"content_digest":row.content_digest}).to_string(),reason:reason.into(),created_unix_ms:now};
        let provenance = self.ingest_object(serde_json::json!({"proposal":id,"decision":decision.id,"principal":"coordinator","delegation":crate::store::MEMORY_COORDINATOR_DELEGATION,"source_submission":row.submission_id,"content_digest":row.content_digest}).to_string().as_bytes())?;
        let revision = approve.then(|| NewRevision {
            id: MemoryRecordId::new(format!(
                "mem-{:x}",
                Sha256::digest(change.record_key.as_bytes())
            ))
            .expect("bounded hash identity"),
            record_key: change.record_key.clone(),
            scope_id: "project".into(),
            kind: MemoryKind::Observation,
            body_hash: body,
            provenance_hash: provenance,
            applicability: change.scope.clone(),
            dependencies: vec![],
            expected: None,
            expiry_unix_ms: None,
            validity_state: "valid".into(),
            validity_reason: "coordinator approved project fact".into(),
        });
        self.store
            .decide_result_memory(&row, &decision, revision.as_ref(), config, now)
            .map_err(MemoryError::from)
    }
}
