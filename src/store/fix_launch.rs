//! Owner-session fix launch and native result lifecycle linking (schema 70).
use super::*;
use rusqlite::OptionalExtension;
use std::collections::BTreeSet;

pub(super) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!("../../migrations/0070_fix_launch.sql"))?;
    if db.prepare("PRAGMA foreign_key_check")?.query([])?.next()?.is_some() {
        return Err(StoreError::Corrupt("fix launch migration left a foreign key violation".into()));
    }
    Ok(())
}
fn invalid(message: impl Into<String>) -> StoreError { StoreError::Invalid(message.into()) }
pub(super) fn prior_findings_available(db: &Connection) -> Result<bool> { present(db) }
fn present(db: &Connection) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='fix_launches' AND type='table')", [], |r| r.get(0))?)
}
fn selection(review: Option<&str>, refs: &[String]) -> String {
    let refs: BTreeSet<_> = refs.iter().collect();
    serde_json::json!({"review":review,"findings":refs}).to_string()
}
struct Selected { claim: Option<i64>, finding: Option<String>, evidence: String }
fn select(db: &Connection, review: Option<&str>, refs: &[String]) -> Result<Vec<Selected>> {
    let state = finding_state(db, None)?.ok_or(StoreError::UnsupportedSchema(55))?;
    let mut submissions = BTreeSet::new();
    if let Some(review) = review {
        let session: String = db.query_row("SELECT c.session_id FROM review_completions c JOIN review_sessions s USING(session_id) JOIN attempts a ON a.id=s.attempt_id JOIN review_session_launches l USING(session_id)
            WHERE a.task_id=?1 AND c.outcome='completed' ORDER BY c.completed_unix_ms DESC,c.session_id DESC LIMIT 1", [review], |r| r.get(0)).optional()?
            .ok_or_else(|| invalid(format!("review task {review} has no completed receipt")))?;
        submissions.extend(state.submissions.iter().filter(|s| s.session_id == session).map(|s| s.submission_id));
    }
    let mut findings = BTreeSet::new();
    for reference in refs {
        if let Some(finding) = state.findings.iter().find(|f| &f.finding_id == reference) {
            if finding.status != "validated" { return Err(invalid(format!("finding {reference} is not validated"))); }
            findings.insert(reference.clone());
        } else {
            let matches: Vec<_> = state.submissions.iter().filter(|s| &s.finding_ref == reference).collect();
            if matches.len() != 1 { return Err(invalid(format!("finding reference {reference} must name exactly one receipt submission or canonical finding"))); }
            submissions.insert(matches[0].submission_id);
        }
    }
    let mut selected = Vec::new();
    for submission in state.submissions.iter().filter(|s| submissions.contains(&s.submission_id)) {
        let digest: String = db.query_row("SELECT receipt_digest FROM review_completions WHERE session_id=?1", [&submission.session_id], |r| r.get(0))?;
        for claim in &submission.claims {
            match claim.outcome.as_str() {
                "pending" => selected.push(Selected { claim: Some(claim.claim_id), finding: None, evidence: digest.clone() }),
                "validated" => { findings.insert(claim.canonical_finding.clone().ok_or_else(|| invalid("validated claim has no canonical finding"))?); }
                _ => return Err(invalid(format!("claim {} is neither pending nor validated", claim.claim_id))),
            }
        }
    }
    let fixes = fix_state(db, None)?.ok_or(StoreError::UnsupportedSchema(56))?;
    for finding in findings {
        let f = fixes.findings.iter().find(|f| f.finding_id == finding).ok_or_else(|| invalid("missing canonical finding"))?;
        if f.status != "validated" || f.currently_resolved || fixes.repairs.iter().any(|r| r.finding_id == finding && r.closure.is_none()) {
            return Err(invalid(format!("finding {finding} is not available for a new repair")));
        }
        selected.push(Selected { claim: None, finding: Some(finding), evidence: String::new() });
    }
    if selected.len() > 64 { return Err(invalid("a fix launch names at most 64 findings")); }
    if selected.is_empty() { return Err(invalid("a fix launch must name at least one finding")); }
    Ok(selected)
}
impl SqliteStore {
    /// Read-only preflight, before any launch mutations. A retained launch freezes
    /// the selectors and profile, even after its claims have been repaired.
    pub fn check_fix_run(&mut self, task: &str, review: Option<&str>, refs: &[String], profile: &str) -> Result<()> {
        if !present(&self.connection)? {
            if review.is_some() || !refs.is_empty() { return Err(StoreError::UnsupportedSchema(69)); }
            return Ok(());
        }
        let existing: Option<(String, String)> = self.connection.query_row("SELECT selection_json,profile FROM fix_launches WHERE task_id=?1", [task], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((frozen, assigned)) = existing {
            if frozen != selection(review, refs) || assigned != profile { return Err(invalid("the fix task is already bound to another selection or profile")); }
        } else if review.is_some() || !refs.is_empty() {
            if review == Some(task) { return Err(invalid("a fix runs as its own task")); }
            if self.connection.query_row("SELECT EXISTS(SELECT 1 FROM attempts WHERE task_id=?1)", [task], |r| r.get::<_,bool>(0))? { return Err(invalid("fix findings must be bound before the task's first attempt")); }
            select(&self.connection, review, refs)?;
        }
        Ok(())
    }
    /// Atomically validate selected pending claims, open assigned opportunities
    /// and freeze the task binding. Other claims remain pending.
    pub fn prepare_fix_run(&mut self, task: &str, review: Option<&str>, refs: &[String], profile: &str, now: i64) -> anyhow::Result<()> {
        self.atomic_replay(|store| {
            store.check_fix_run(task, review, refs, profile)?;
            if store.connection.query_row("SELECT EXISTS(SELECT 1 FROM fix_launches WHERE task_id=?1)", [task], |r| r.get::<_,bool>(0))? { return Ok(()); }
            let selected = select(&store.connection, review, refs)?;
            store.connection.execute("INSERT INTO fix_launches(task_id,selection_json,profile) VALUES(?1,?2,?3)", params![task, selection(review,refs),profile])?;
            for selected in selected {
                let finding = if let Some(claim) = selected.claim {
                    let event = store.triage_finding_claim(claim, &TriageRequest { outcome: TriageOutcome::Validated { target: FindingTarget::New { title: None }, severity: "medium".into() }, evidence: vec![selected.evidence], expected_seq: None }, "operator:cli", now)?;
                    event.subject["finding_id"].as_str().ok_or_else(|| invalid("validation did not mint a finding"))?.to_owned()
                } else { selected.finding.ok_or_else(|| invalid("missing fix finding"))? };
                let opened = store.open_repair(&finding, &RepairAssignment::Profile(profile.into()), DEFAULT_REPAIR_HORIZON_MS, None, "operator:cli", now)?;
                store.connection.execute("INSERT INTO fix_launch_findings(task_id,repair_seq) VALUES(?1,?2)", params![task,opened.seq])?;
            }
            Ok(())
        })
    }
}
fn log(db: &Connection, kind: &str, now: i64) -> Result<i64> {
    super::fix_attribution::log(db, kind, "operator:cli", None, now)
}
/// Bind before any outcome, inside the reservation transaction.
pub(super) fn start(db: &Connection, inputs: &LaunchInputs, attempt: &AttemptId, now: i64) -> Result<()> {
    if !present(db)? { return Ok(()); }
    let repairs: Vec<(i64,String)> = db.prepare("SELECT r.seq,r.configuration_id FROM fix_launch_findings f JOIN repair_opportunities r ON r.seq=f.repair_seq WHERE f.task_id=?1")?
        .query_map([inputs.task.as_str()], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (repair, configuration) in repairs {
        let profile = inputs.effective_profile.as_ref().ok_or_else(|| invalid("a fix launch needs an effective profile"))?;
        if crate::domain::agent_configuration(profile).id != configuration { return Err(invalid("a fix launches only with its assigned configuration")); }
        let seq = log(db,"attempt_bound",now)?;
        db.execute("INSERT INTO repair_attempts(seq,repair_seq,attempt_id,ordinal,configuration_id) SELECT ?1,?2,?3,count(*)+1,?4 FROM repair_attempts WHERE repair_seq=?2", params![seq,repair,attempt.as_str(),configuration])?;
    }
    Ok(())
}
/// Submission and all per-finding proposals commit together.
pub(super) fn submitted(db: &Connection, submission: &str, now: i64) -> Result<()> {
    if !present(db)? { return Ok(()); }
    let repairs: Vec<i64> = db.prepare("SELECT a.repair_seq FROM result_submissions s JOIN repair_attempts a ON a.attempt_id=s.attempt_id JOIN fix_launch_findings f ON f.repair_seq=a.repair_seq WHERE s.submission_id=?1 AND NOT EXISTS(SELECT 1 FROM repair_closures c WHERE c.repair_seq=a.repair_seq)")?
        .query_map([submission], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for repair in repairs {
        let seq = log(db,"proposed",now)?;
        db.execute("INSERT INTO fix_proposals(seq,repair_seq,submission_id,attempt_id,candidate_oid) SELECT ?1,?2,submission_id,attempt_id,candidate_oid FROM result_submissions WHERE submission_id=?3", params![seq,repair,submission])?;
    }
    Ok(())
}
/// Accepted native verification of the exact proposal is the delegated repair
/// decision; approved_alternative does not claim a reproduced regression.
pub(super) fn verified(db: &Connection, run: &str, now: i64) -> Result<()> {
    if !present(db)? { return Ok(()); }
    let proposals: Vec<(i64,i64)> = db.prepare("SELECT p.seq,p.repair_seq FROM verification_runs v JOIN verified_results r USING(run_id) JOIN fix_proposals p ON p.submission_id=v.submission_id AND p.candidate_oid=v.commit_oid JOIN fix_launch_findings f ON f.repair_seq=p.repair_seq WHERE v.run_id=?1 AND v.state='accepted' AND NOT EXISTS(SELECT 1 FROM fix_verifications fv WHERE fv.proposal_seq=p.seq) AND NOT EXISTS(SELECT 1 FROM repair_closures c WHERE c.repair_seq=p.repair_seq)")?
        .query_map([run], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (proposal,repair) in proposals {
        let seq = log(db,"verified",now)?;
        db.execute("INSERT INTO fix_verifications(seq,proposal_seq,run_id,result_id,commit_oid,assurance,evidence_refs) SELECT ?1,?2,v.run_id,r.result_id,v.commit_oid,'approved_alternative',?4 FROM verification_runs v JOIN verified_results r USING(run_id) WHERE v.run_id=?3", params![seq,proposal,run,serde_json::json!([format!("verification_run:{run}")]).to_string()])?;
        let seq = log(db,"repair_closed",now)?;
        db.execute("INSERT INTO repair_closures(seq,repair_seq,outcome) VALUES(?1,?2,'fixed')",params![seq,repair])?;
    }
    Ok(())
}
/// Publication links every verified proposal of the exact integrated candidate.
pub(super) fn integrated(db: &Connection, integrated: &str, now: i64) -> Result<()> {
    if !present(db)? { return Ok(()); }
    let proposals: Vec<(i64,i64)> = db.prepare("SELECT p.seq,fv.seq FROM integrated_commits ic JOIN integration_operations o USING(operation_id) JOIN verified_results v ON v.result_id=o.verified_result_id JOIN integration_candidates c USING(candidate_id) JOIN fix_proposals p ON p.submission_id=v.submission_id AND p.candidate_oid=v.commit_oid AND p.candidate_oid=c.parent_verified JOIN fix_verifications fv ON fv.proposal_seq=p.seq JOIN fix_launch_findings f ON f.repair_seq=p.repair_seq WHERE ic.integrated_id=?1 AND NOT EXISTS(SELECT 1 FROM fix_integrations fi WHERE fi.proposal_seq=p.seq)")?
        .query_map([integrated], |r| Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    for (proposal,verification) in proposals {
        let seq = log(db,"integrated",now)?;
        db.execute("INSERT INTO fix_integrations(seq,proposal_seq,verification_seq,integrated_id,commit_oid,integrated_unix_ms) SELECT ?1,?2,?3,integrated_id,commit_oid,created_unix_ms FROM integrated_commits WHERE integrated_id=?4",params![seq,proposal,verification,integrated])?;
    }
    Ok(())
}
