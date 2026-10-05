//! Immutable launch identity, kept outside signed execution inputs (schema 72).
use super::*;
use rusqlite::OptionalExtension;
use std::collections::BTreeSet;

fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::Invalid(message.into())
}
fn present(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='task_lineage')",
        [],
        |r| r.get(0),
    )?)
}
impl SqliteStore {
    /// Explicit identity wins; otherwise inherit the reviewed/fixed work item.
    /// Historical targets without lineage use their task identity.
    pub fn resolve_launch_work_item(
        &mut self,
        task: &str,
        explicit: Option<&str>,
        target: Option<&str>,
        fixes: &[String],
    ) -> Result<String> {
        if let Some(work) = explicit {
            TaskId::new(work.to_owned()).map_err(invalid)?;
            return Ok(work.into());
        }
        if !present(&self.connection)? {
            return Ok(target.unwrap_or(task).into());
        }
        let work = |target: &str| -> Result<String> {
            Ok(self
                .connection
                .query_row(
                    "SELECT work_item FROM task_lineage WHERE task_id=?1",
                    [target],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_else(|| target.into()))
        };
        if let Some(target) = target {
            return work(target);
        }
        let mut inherited = BTreeSet::new();
        for reference in fixes {
            let targets: Vec<String> = self.connection.prepare("SELECT DISTINCT o.task_id FROM finding_submissions f JOIN review_sessions s USING(session_id) JOIN review_opportunities o USING(opportunity_id)
                WHERE f.finding_ref=?1 OR f.submission_id IN (SELECT c.submission_id FROM finding_claims c JOIN finding_decisions d USING(claim_id) WHERE d.finding_id=?1)")?
                .query_map([reference], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            for target in targets {
                inherited.insert(work(&target)?);
            }
        }
        if inherited.len() > 1 {
            return Err(invalid(
                "fix findings span work items; pass --work-item explicitly",
            ));
        }
        if let Some(work) = inherited.into_iter().next() {
            return Ok(work);
        }
        // A launch without a target starts a new work item.
        Ok(task.into())
    }
    /// Read-only preflight; retained lineage must match exactly.
    pub fn check_task_lineage(
        &mut self,
        task: &str,
        work: &str,
        role: &str,
        supersedes: Option<&str>,
    ) -> Result<()> {
        TaskId::new(work.to_owned()).map_err(invalid)?;
        if ![
            "build", "fix", "review", "skeptic", "recheck", "merge", "plan", "other",
        ]
        .contains(&role)
        {
            return Err(invalid("invalid task role"));
        }
        if !present(&self.connection)? {
            return Ok(());
        }
        let existing: Option<(String, String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT work_item,role,supersedes_task FROM task_lineage WHERE task_id=?1",
                [task],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != (work.into(), role.into(), supersedes.map(str::to_owned)) {
                return Err(invalid(
                    "lineage must be set before the task's first attempt",
                ));
            }
        } else if self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM attempts WHERE task_id=?1)",
            [task],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(invalid(
                "lineage must be set before the task's first attempt",
            ));
        }
        if let Some(old) = supersedes {
            if old == task {
                return Err(invalid("a task cannot supersede itself"));
            }
            let old_work: Option<String> = self
                .connection
                .query_row(
                    "SELECT work_item FROM task_lineage WHERE task_id=?1",
                    [old],
                    |r| r.get(0),
                )
                .optional()?;
            if old_work.as_deref() != Some(work) {
                return Err(invalid(
                    "--supersedes must name a task of the same work item",
                ));
            }
            if self.connection.query_row("SELECT EXISTS(SELECT 1 FROM attempts WHERE task_id=?1 AND state NOT IN ('completed','failed','cancelled','lost'))", [old], |r| r.get::<_,bool>(0))? { return Err(invalid("--supersedes task has an active attempt")); }
        }
        Ok(())
    }
    /// Recheck and insert together, after activation and before reservation.
    pub fn prepare_task_lineage(
        &mut self,
        task: &str,
        work: &str,
        role: &str,
        supersedes: Option<&str>,
    ) -> anyhow::Result<()> {
        self.atomic_replay(|store| {
            store.check_task_lineage(task, work, role, supersedes)?;
            if present(&store.connection)? {
                store.connection.execute("INSERT INTO task_lineage(task_id,work_item,role,supersedes_task,recorded_unix_ms) VALUES(?1,?2,?3,?4,CAST(unixepoch('subsec')*1000 AS INTEGER)) ON CONFLICT(task_id) DO NOTHING", params![task,work,role,supersedes])?;
            }
            Ok(())
        })
    }
}
