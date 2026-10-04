//! Exact, expiring owner actions; cap authority is consumed by reservation.
use super::*;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OwnerRequest {
    pub id: String,
    pub action: String,
    pub task: String,
    pub contract_digest: String,
    pub repository: String,
    pub summary: String,
    pub expires: i64,
    pub status: String,
    pub decision_by: Option<String>,
    pub decided: Option<i64>,
}

/// Hash exact contract decisions, excluding the mutable installation envelope.
pub fn contract_digest(mut document: serde_json::Value) -> String {
    if let Some(object) = document.as_object_mut() {
        for field in ["expected_head", "contract_revision"] {
            object.remove(field);
        }
    }
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&document).expect("JSON value"))
    )
}
fn read(db: &Connection, id: &str) -> Result<Option<OwnerRequest>> {
    Ok(db.query_row("SELECT id,action,task,contract_digest,repository,summary,expires,status,decision_by,decided FROM owner_requests WHERE id=?1", [id], |r| Ok(OwnerRequest {
        id:r.get(0)?, action:r.get(1)?, task:r.get(2)?, contract_digest:r.get(3)?, repository:r.get(4)?, summary:r.get(5)?, expires:r.get(6)?, status:r.get(7)?, decision_by:r.get(8)?, decided:r.get(9)?,
    })).optional()?)
}
impl SqliteStore {
    pub fn owner_request(&self, id: &str) -> Result<Option<OwnerRequest>> {
        read(&self.connection, id)
    }
    /// Idempotent delivery of one exact action and its canonical inbox item.
    pub fn request_owner(
        &mut self,
        action: &str,
        task: &str,
        digest: &str,
        repository: &str,
        slug: &str,
        now: i64,
    ) -> Result<OwnerRequest> {
        super::delivery::now_check(now)?;
        if !matches!(action, "cap" | "repository")
            || digest.len() != 64
            || !digest.bytes().all(|c| c.is_ascii_hexdigit())
            || task.is_empty()
            || [task, repository, slug]
                .iter()
                .any(|s| s.chars().any(char::is_control))
            || !Path::new(repository).is_absolute()
        {
            return Err(StoreError::Invalid("invalid owner action".into()));
        }
        let summary = if action == "cap" {
            format!(
                "Allow this one launch of task {task} (contract digest {digest}) above the worker cap"
            )
        } else {
            format!(
                "Add repository {repository} to PROJECT.md for project {slug} (task {task}, contract digest {digest})"
            )
        };
        let mut id = format!("owner-request-{:x}", Sha256::digest(summary.as_bytes()));
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(old) = read(&tx, &id)? {
            if matches!(old.status.as_str(), "pending" | "approved") && old.expires > now {
                return Ok(old);
            }
            let count:u64=tx.query_row("SELECT count(*) FROM owner_requests WHERE action=?1 AND task=?2 AND contract_digest=?3 AND repository=?4",params![action,task,digest,repository],|r|r.get(0))?;
            // A new request after expiry/rejection/consumption has a new identity;
            // repeated refusals reuse the currently pending request.
            id = format!("{id}-{count}");
            let current:Option<String>=tx.query_row("SELECT id FROM owner_requests WHERE action=?1 AND task=?2 AND contract_digest=?3 AND repository=?4 AND status IN ('pending','approved') AND expires>?5",params![action,task,digest,repository,now],|r|r.get(0)).optional()?;
            if let Some(current) = current {
                return read(&tx, &current)?.ok_or(StoreError::Conflict);
            }
        }
        let expires = now
            .checked_add(86_400_000)
            .ok_or_else(|| StoreError::Invalid("expiry overflow".into()))?;
        tx.execute("INSERT INTO owner_requests(id,action,task,contract_digest,repository,summary,expires,status) VALUES(?1,?2,?3,?4,?5,?6,?7,'pending')",params![id,action,task,digest,repository,summary,expires])?;
        let request = read(&tx, &id)?.ok_or(StoreError::Conflict)?;
        super::inbox::insert(
            &tx,
            &InboxItem {
                revision: 1,
                seen: false,
                done: false,
                content: InboxContent {
                    id: id.clone(),
                    kind: "owner-request".into(),
                    subject: task.into(),
                    summary: summary.clone(),
                    body: serde_json::to_string(&request)
                        .map_err(|e| StoreError::Invalid(e.to_string()))?,
                    created: now.to_string(),
                },
            },
        )?;
        tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('owner.requested',?1,1,1,?2)",params![id,serde_json::to_string(&request).map_err(|e|StoreError::Invalid(e.to_string()))?])?;
        tx.commit()?;
        Ok(request)
    }
    pub fn decide_owner_request(
        &mut self,
        id: &str,
        summary: &str,
        approve: bool,
        now: i64,
    ) -> Result<()> {
        super::delivery::now_check(now)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request =
            read(&tx, id)?.ok_or_else(|| StoreError::Invalid("owner request missing".into()))?;
        if request.summary != summary
            || request.status != "pending"
            || (approve && now >= request.expires)
        {
            return Err(StoreError::Invalid(
                "summary mismatch, request expired, or request already decided".into(),
            ));
        }
        let status = if approve { "approved" } else { "rejected" };
        tx.execute("UPDATE owner_requests SET status=?2,decision_by='owner:claude-code-ask',decided=?3 WHERE id=?1",params![id,status,now])?;
        tx.execute(
            "UPDATE inbox_items SET done=1,seen=1,revision=revision+1 WHERE id=?1",
            [id],
        )?;
        let revision: u64 =
            tx.query_row("SELECT revision FROM inbox_items WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
        tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('inbox.done',?1,?2,1,'{\"seen\":true,\"done\":true}')",params![id,revision])?;
        tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('owner.decided',?1,2,1,?2)",params![id,serde_json::json!({"status":status,"decision_by":"owner:claude-code-ask","decided":now}).to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn owner_cap_exemption(&self, task: &str, digest: &str, now: i64) -> Result<bool> {
        Ok(cap_request(&self.connection, task, digest, now)?.is_some())
    }
}
fn cap_request(db: &Connection, task: &str, digest: &str, now: i64) -> Result<Option<String>> {
    Ok(db.query_row("SELECT id FROM owner_requests WHERE action='cap' AND task=?1 AND contract_digest=?2 AND status='approved' AND expires>?3",params![task,digest,now],|r|r.get(0)).optional()?)
}
pub(super) fn cap_for_inputs(
    db: &Connection,
    inputs: &LaunchInputs,
    now: i64,
) -> Result<Option<String>> {
    let raw: Option<Vec<u8>> = db
        .query_row(
            "SELECT raw_bytes FROM task_contracts WHERE task_id=?1 AND contract_revision=?2",
            params![
                inputs.task.as_str(),
                inputs.task_contract.as_ref().map(|r| r.revision)
            ],
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = raw else { return Ok(None) };
    let document = serde_json::from_slice(&raw).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    cap_request(db, inputs.task.as_str(), &contract_digest(document), now)
}
