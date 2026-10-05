use super::*;
use crate::operations::{DeliveryState,Outcome};
use std::collections::BTreeSet;

pub(super) fn read_all(db:&Connection)->Result<Vec<InboxItem>> {read_all_with_budget(db,None)}
pub(super) fn read_all_with_budget(db:&Connection,budget:Option<&read_budget::ReadBudget>)->Result<Vec<InboxItem>> {read_matching(db,"",budget)}
/// Exactly the items with `!seen && !done`, through `inbox_items_unseen`.
pub(super) fn read_unseen(db:&Connection)->Result<Vec<InboxItem>> {read_matching(db,"WHERE seen=0 AND done=0",None)}
/// Owner memory decision delivery survives inbox consumption: keep its exact
/// evidence visible while the associated notification is unresolved.
pub(super) fn read_notification_items(db:&Connection)->Result<Vec<InboxItem>> {
    // Keep the ordinary unseen read on its existing partial index. Only the
    // sealed decision operations add consumed owner evidence by primary key.
    let mut items:std::collections::BTreeMap<_,_>=read_unseen(db)?.into_iter().map(|item|(item.content.id.clone(),item)).collect();
    let version:u32=db.query_row("PRAGMA user_version",[],|r|r.get(0))?;
    if version>=71 {
        let retained=read_matching(db,"WHERE id IN (SELECT json_extract(o.payload,'$.inbox_ids[0]') FROM result_memory_decisions m JOIN operations o ON o.id=m.notification_id JOIN operation_delivery d ON d.operation_id=o.id WHERE d.state IN ('pending','claimed','ambiguous'))",None)?;
        for item in retained {items.insert(item.content.id.clone(),item);}
    }
    Ok(items.into_values().collect())
}
fn read_matching(db:&Connection,filter:&str,budget:Option<&read_budget::ReadBudget>)->Result<Vec<InboxItem>> {
    let mut stmt=db.prepare(&format!("SELECT revision,payload,payload_hash,seen,done,id FROM inbox_items {filter} ORDER BY id"))?;
    let mut rows=stmt.query([])?;
    let mut result=Vec::new();
    while let Some(r)=rows.next()? {
        if let Some(budget)=budget {budget.row(r,&[(1,1)])?;}
        let values=(r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,bool>(4)?,r.get::<_,String>(5)?);
        let(revision,payload,hash,seen,done,id)=values;
        if format!("{:x}",Sha256::digest(payload.as_bytes()))!=hash{return Err(StoreError::Corrupt("inbox payload hash mismatch".into()));}
        let content:InboxContent=serde_json::from_str(&payload).map_err(|e|StoreError::Corrupt(e.to_string()))?;
        if content.id!=id{return Err(StoreError::Corrupt("inbox row identity mismatch".into()));}
        content.validate().map_err(StoreError::Corrupt)?;result.push(InboxItem{revision,content,seen,done});
    }
    Ok(result)
}
pub(super) fn insert(db:&Connection,item:&InboxItem)->Result<()> {
    item.content.validate().map_err(StoreError::Invalid)?;
    let payload=serde_json::to_string(&item.content).map_err(|e|StoreError::Invalid(e.to_string()))?;
    if payload.len()>16*MAX_RECORD_BYTES{return Err(StoreError::Invalid("inbox record too large".into()));}
    db.execute("INSERT INTO inbox_items VALUES(?1,?2,?3,?4,?5,?6)",params![item.content.id,integer(item.revision)?,payload,format!("{:x}",Sha256::digest(payload.as_bytes())),item.seen,item.done])?;Ok(())
}
/// Imports only immutable provenance; upgrades never read stale legacy files.
pub(super) fn import_sources(db:&Connection)->Result<()> {
    let seen:Option<(Vec<u8>,String)>=db.query_row("SELECT bytes,digest FROM legacy_sources WHERE path='.state/inbox-seen.json'",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((bytes,hash))=&seen {if format!("{:x}",Sha256::digest(bytes))!=*hash{return Err(StoreError::Corrupt("seen source hash mismatch".into()));}}
    let seen:BTreeSet<String>=seen.map(|(b,_)|serde_json::from_slice(&b)).transpose().map_err(|e|StoreError::Corrupt(e.to_string()))?.unwrap_or_default();
    let records={let mut stmt=db.prepare("SELECT path,bytes,digest FROM legacy_sources WHERE kind='inbox' ORDER BY path")?;stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?};
    for(path,bytes,digest)in records {
        if !path.ends_with(".md"){continue;}
        if format!("{:x}",Sha256::digest(&bytes))!=digest{return Err(StoreError::Corrupt("inbox source digest mismatch".into()));}
        let text=std::str::from_utf8(&bytes).map_err(|e|StoreError::Corrupt(e.to_string()))?;
        let rest=text.strip_prefix("+++\n").ok_or_else(||StoreError::Corrupt("inbox header missing".into()))?;
        let(header,body)=rest.split_once("\n+++\n").or_else(||rest.strip_suffix("\n+++").map(|h|(h,""))).ok_or_else(||StoreError::Corrupt("inbox header unclosed".into()))?;
        let mut content:InboxContent=toml::from_str(header).map_err(|e|StoreError::Corrupt(e.to_string()))?;
        content.body=body.trim_matches('\n').into();
        let expected=if path.starts_with("inbox/done/"){format!("inbox/done/{}.md",content.id)}else{format!("inbox/{}.md",content.id)};
        if expected!=path{return Err(StoreError::Corrupt("inbox source path/identity mismatch".into()));}
        let item=InboxItem{revision:1,seen:seen.contains(&content.id),done:path.starts_with("inbox/done/"),content};insert(db,&item)?;
    }
    // Legacy review decisions have no canonical worker proposal identity. Retain
    // their bytes and surface unresolved decisions without inventing one.
    let review:Option<Vec<u8>>=db.query_row("SELECT bytes FROM legacy_sources WHERE path='.state/memory-review.json'",[],|r|r.get(0)).optional()?;
    if let Some(bytes)=review {
        let state:serde_json::Value=serde_json::from_slice(&bytes).map_err(|e|StoreError::Corrupt(e.to_string()))?;
        for obligation in state["obligations"].as_array().ok_or_else(||StoreError::Corrupt("invalid legacy reviews".into()))? {
            if !matches!(obligation["status"].as_str(),Some("pending"|"deferred")) {continue;}
            let id=format!("legacy-memory-review-{:x}",Sha256::digest(obligation["id"].as_str().unwrap_or_default().as_bytes()));
            if db.query_row("SELECT EXISTS(SELECT 1 FROM inbox_items WHERE id=?1)",[&id],|r|r.get::<_,bool>(0))? {continue;}
            let content=InboxContent{id,kind:"memory-review".into(),subject:obligation["thread_id"].as_str().unwrap_or_default().into(),
                created:obligation["created"].as_str().unwrap_or_default().into(),summary:format!("Retained legacy memory review {} ({}) requires an explicit decision",obligation["id"].as_str().unwrap_or_default(),obligation["status"].as_str().unwrap_or_default()),body:String::new()};
            insert(db,&InboxItem{revision:1,content,seen:false,done:false})?;
        }
    }
    Ok(())
}
impl SqliteStore {
    /// Indexed read for a blocking client; no project lock or write transaction.
    pub fn unseen_inbox(&self) -> Result<Vec<InboxItem>> {
        check_schema(&self.connection)?;
        read_unseen(&self.connection)
    }

    /// Safe internal adapter: inspect, insert/deduplicate, and receipt commit in
    /// one SQLite transaction. It never retries an external/terminal effect.
    pub fn drain_inbox(&mut self,expected_head:u64,now:i64)->Result<usize> {
        super::delivery::now_check(now)?;
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;check_schema(&tx)?;
        if head(&tx)?!=expected_head{return Err(StoreError::Conflict);}
        let operations=read_operations(&tx)?;let mut count=0;
        let mut items=read_all(&tx)?.into_iter().map(|i|(i.content.id.clone(),i)).collect::<std::collections::BTreeMap<_,_>>();
        for op in operations.into_iter().filter(|o|o.kind=="legacy.inbox") {
            let old=super::delivery::delivery(&tx,&op.id)?;
            if !matches!(old.state,DeliveryState::Pending|DeliveryState::Ambiguous){continue;}
            // Explicit internal reconciliation can inspect ambiguous records now;
            // ordinary pending records still respect their backoff.
            if old.state==DeliveryState::Pending && old.next_due_ms>now {continue;}
            let revision:i64=tx.query_row("SELECT revision FROM tasks WHERE id=?1",[op.task.as_ref().ok_or(StoreError::Conflict)?.as_str()],|r|r.get(0))?;
            if integer(op.expected_revision)?!=revision{return Err(StoreError::Conflict);}
            if op.payload_version!=1{return Err(StoreError::Invalid("unsupported inbox intent version".into()));}
            if ["id","kind","subject","summary","body"].iter().any(|key|op.payload.get(key).is_none_or(|v|!v.is_string())) {return Err(StoreError::Corrupt("inbox intent requires all content fields".into()));}
            let mut content:InboxContent=serde_json::from_value(op.payload.clone()).map_err(|e|StoreError::Corrupt(e.to_string()))?;
            content.validate().map_err(StoreError::Corrupt)?;
            if content.id!=op.target{return Err(StoreError::Corrupt("inbox intent target mismatch".into()));}
            content.summary=content.summary.chars().map(|c|if c.is_control(){' '}else{c}).collect();content.body=content.body.trim_matches('\n').trim_end().into();
            let existing=items.get(&content.id);
            if let Some(existing)=existing {
                if !existing.content.same_delivery(&content){return Err(StoreError::Conflict);}
            }else{
                content.created=jiff::Timestamp::from_millisecond(now).map_err(|e|StoreError::Invalid(e.to_string()))?.to_string();
                let item=InboxItem{revision:1,content:content.clone(),seen:false,done:false};
                insert(&tx,&item)?;items.insert(content.id.clone(),item);
                tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('inbox.delivered',?1,1,1,?2)",params![content.id,serde_json::to_string(&content).map_err(|e|StoreError::Invalid(e.to_string()))?])?;
            }
            super::delivery::update_outcome(&tx,&old,&Outcome::Confirmed{observed_identity:format!("inbox:{}",content.id)},now,"atomic-inbox-adapter")?;
            count+=1;
        }
        tx.commit()?;Ok(count)
    }
    /// Deliver one memory-review reminder row with a caller-stable id.
    ///
    /// Purpose-built intake following the `enqueue_*` pattern: optimistic head
    /// check, idempotent insert with same-content verification, and an
    /// `inbox.delivered` event in one transaction. No signed control is
    /// required: like legacy ticker inbox writes, this creates
    /// coordinator-visible data only — no memory authority, ownership,
    /// delivery, or external-effect state changes. Scope is fenced inside the
    /// store: kind must be `memory-review`, the id must carry the
    /// `memory-review-` prefix, and the body must be empty (the summary is a
    /// pointer; Remember text never lands in the DB inbox).
    pub fn deliver_memory_review_reminder(&mut self,expected_head:u64,content:&InboxContent,now:i64)->Result<crate::domain::ReminderOutcome> {
        super::delivery::now_check(now)?;
        content.validate().map_err(StoreError::Invalid)?;
        if content.kind!="memory-review"||!content.id.starts_with("memory-review-")||!content.body.is_empty()||content.subject.is_empty()||content.summary.is_empty() {
            return Err(StoreError::Invalid("memory-review reminder requires kind memory-review, a memory-review- id, a subject, a summary, and an empty body".into()));
        }
        self.deliver_stable_notice(expected_head,content,now)
    }
    /// TM4.5 health alert notice (docs/telemetry/contracts-health.md §6): the
    /// same stable-id, deduplicated delivery as memory-review reminders, for
    /// kind `telemetry-health` under a `telemetry-health-` id. Advisory text only.
    pub fn deliver_telemetry_notice(&mut self,expected_head:u64,content:&InboxContent,now:i64)->Result<crate::domain::ReminderOutcome> {
        super::delivery::now_check(now)?;
        content.validate().map_err(StoreError::Invalid)?;
        if content.kind!="telemetry-health"||!content.id.starts_with("telemetry-health-")||!content.body.is_empty()||content.subject.is_empty()||content.summary.is_empty() {
            return Err(StoreError::Invalid("telemetry notice requires kind telemetry-health, a telemetry-health- id, a subject, a summary, and an empty body".into()));
        }
        self.deliver_stable_notice(expected_head,content,now)
    }
    /// Insert one stable-id notice, or recognize its committed row.
    fn deliver_stable_notice(&mut self,expected_head:u64,content:&InboxContent,now:i64)->Result<crate::domain::ReminderOutcome> {
        use crate::domain::ReminderOutcome;
        let mut content=content.clone();
        content.summary=content.summary.chars().map(|c|if c.is_control(){' '}else{c}).collect();
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;check_schema(&tx)?;
        // An existing row resolves before the head check: it is either the
        // committed delivery being retried or a divergent row that no retry
        // can fix, so neither should spend a head-conflict retry.
        let existing:Option<(String,String)>=tx.query_row("SELECT payload,payload_hash FROM inbox_items WHERE id=?1",[content.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((payload,hash))=existing {
            if format!("{:x}",Sha256::digest(payload.as_bytes()))!=hash{return Err(StoreError::Corrupt("inbox payload hash mismatch".into()));}
            let old:InboxContent=serde_json::from_str(&payload).map_err(|e|StoreError::Corrupt(e.to_string()))?;
            // Same stable id and reminder identity: a retry after the insert
            // committed. The summary is a rendering of mutable obligation
            // state, so a disposition between the commit and the caller's
            // counter update legitimately changes it; the committed row
            // already surfaced the obligation and counts as delivered, left
            // untouched. A different kind, subject or body under one id
            // cannot come from a re-render: it is corruption or tampering,
            // never a silent overwrite or a second reminder spend.
            if !old.same_reminder(&content){return Err(StoreError::Invalid(format!("inbox row {} holds divergent bytes; preserve and repair the store",content.id)));}
            return Ok(ReminderOutcome::AlreadyDelivered);
        }
        if head(&tx)?!=expected_head{return Err(StoreError::Conflict);}
        content.created=jiff::Timestamp::from_millisecond(now).map_err(|e|StoreError::Invalid(e.to_string()))?.to_string();
        let item=InboxItem{revision:1,content:content.clone(),seen:false,done:false};
        insert(&tx,&item)?;
        tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('inbox.delivered',?1,1,1,?2)",params![content.id,serde_json::to_string(&content).map_err(|e|StoreError::Invalid(e.to_string()))?])?;
        tx.commit()?;
        Ok(ReminderOutcome::Delivered)
    }
    /// Update only items actually shown to the caller at this event head.
    pub fn update_inbox(&mut self,expected_head:u64,ids:&[String],done:bool)->Result<usize> {
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;check_schema(&tx)?;
        if head(&tx)?!=expected_head{return Err(StoreError::Conflict);}
        let items=read_all(&tx)?;let mut unique=BTreeSet::new();let mut count=0;
        for id in ids {
            if !unique.insert(id){continue;}
            let item=items.iter().find(|i|&i.content.id==id).ok_or(StoreError::Conflict)?;
            if (done&&item.done)||(!done&&item.seen){continue;}
            let revision=item.revision.checked_add(1).ok_or_else(||StoreError::Invalid("inbox revision exhausted".into()))?;
            tx.execute("UPDATE inbox_items SET revision=?2,seen=1,done=?3 WHERE id=?1",params![id,integer(revision)?,done||item.done])?;
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES(?1,?2,?3,1,?4)",params![if done{"inbox.done"}else{"inbox.seen"},id,integer(revision)?,serde_json::json!({"seen":true,"done":done||item.done}).to_string()])?;
            count+=1;
        }
        tx.commit()?;Ok(count)
    }
}

/// Result notices are data, committed atomically with their originating event.
pub(super) fn result_notice(db: &Connection, kind: &str, event: &str, task: &str, attempt: &str, result: &str, feedback: &str) -> Result<()> {
    let id = format!("worker-result-{:x}", Sha256::digest(format!("{kind}\0{event}").as_bytes()));
    if db.query_row("SELECT EXISTS(SELECT 1 FROM inbox_items WHERE id=?1)", [&id], |r| r.get::<_, bool>(0))? { return Ok(()); }
    let feedback: String = feedback.chars().take(2000).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let content = InboxContent { id, kind: kind.into(), subject: task.into(),
        created: jiff::Timestamp::now().to_string(),
        summary: format!("{kind}: task {task}, attempt {attempt}, submission/result {result}; {feedback}"), body: String::new() };
    insert(db, &InboxItem { revision: 1, content, seen: false, done: false })
}
pub(super) fn ended_notice(db: &Connection, attempt: &Attempt) -> Result<()> {
    let schema: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if schema < 26 { return Ok(()); }
    let submitted: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM result_submissions WHERE attempt_id=?1)", [attempt.id.as_str()], |r| r.get(0))?;
    if !submitted {
        let receipt: Option<(String,String)> = if schema >= 62 {
            db.query_row("SELECT c.session_id,c.outcome FROM review_session_launches l JOIN review_completions c USING(session_id) WHERE l.attempt_id=?1", [attempt.id.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?
        } else { None };
        if let Some((session, outcome)) = receipt {
            result_notice(db, "attempt.review_receipt_without_result", attempt.id.as_str(), attempt.task.as_str(), attempt.id.as_str(), &session,
                &format!("review receipt recorded ({outcome}); report result not submitted; receipt and findings remain visible in telemetry review show/report"))?;
        } else { result_notice(db, "attempt.ended_without_submission", attempt.id.as_str(), attempt.task.as_str(), attempt.id.as_str(), "none", attempt.state.as_str())?; }
    }
    Ok(())
}

impl SqliteStore {
    /// Advisory idle observation for an exact attempt revision. Busy breaks
    /// the stretch; unknown defers notices without rearming a delivered one.
    pub fn observe_worker_idle(&mut self, expected_head: u64, attempt: &AttemptId, revision: u64, idle: Option<bool>, now: i64) -> Result<bool> {
        super::delivery::now_check(now)?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_schema(&tx)?;
        if head(&tx)? != expected_head { return Err(StoreError::Conflict); }
        let current = read_attempt(&tx, attempt)?;
        if current.revision != revision { return Err(StoreError::Conflict); }
        let submitted: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM result_submissions WHERE attempt_id=?1)", [attempt.as_str()], |r| r.get(0))?;
        let idle = if current.state == AttemptState::Running && !current.termination_observed && !submitted { idle } else { Some(false) };
        let old: Option<(i64, Option<i64>, i64, bool)> = tx.query_row("SELECT generation,idle_since_ms,observed_ms,notified FROM worker_idle_stretches WHERE attempt_id=?1", [attempt.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let (mut generation, mut since, observed, mut notified) = old.unwrap_or((0,None,0,false));
        if now < observed { return Err(StoreError::Conflict); }
        if idle == Some(false) { since = None; notified = false; }
        else if idle == Some(true) && since.is_none() { generation += 1; since = Some(now); notified = false; }
        let threshold = crate::timing::pass(std::time::Duration::from_secs(600)).as_millis() as i64;
        let due = idle == Some(true) && since.is_some_and(|since| now - since >= threshold) && !notified;
        if due {
            let minutes = (now - since.unwrap()) / 60_000;
            let content = InboxContent { id: format!("worker-idle-{}-{generation}", attempt.as_str()), kind: "attempt.worker_idle".into(), subject: current.task.as_str().into(), created: jiff::Timestamp::from_millisecond(now).map_err(|e| StoreError::Invalid(e.to_string()))?.to_string(), summary: format!("attempt {}: worker idle for {minutes} min without submitting", attempt.as_str()), body: String::new() };
            insert(&tx, &InboxItem { revision: 1, content: content.clone(), seen: false, done: false })?;
            tx.execute("INSERT INTO events(kind,entity,revision,payload_version,payload) VALUES('inbox.delivered',?1,1,1,?2)", params![content.id,serde_json::to_string(&content).map_err(|e| StoreError::Invalid(e.to_string()))?])?;
            notified = true;
        }
        tx.execute("INSERT INTO worker_idle_stretches VALUES(?1,?2,?3,?4,?5) ON CONFLICT(attempt_id) DO UPDATE SET generation=excluded.generation,idle_since_ms=excluded.idle_since_ms,observed_ms=excluded.observed_ms,notified=excluded.notified", params![attempt.as_str(),generation,since,now,notified])?;
        tx.commit()?;
        Ok(due)
    }

    /// Metadata-only question notice. Stable native call identity deduplicates
    /// collectors and restarts; argument/question text is never accepted here.
    pub fn notify_worker_question(&mut self, attempt: &AttemptId, session: &str, call: &str, tool: &str, now: i64) -> Result<crate::domain::ReminderOutcome> {
        super::delivery::now_check(now)?;
        if !matches!(tool,"request_user_input"|"request_user_input_async") || session.is_empty() || call.is_empty() { return Err(StoreError::Invalid("invalid question metadata".into())); }
        let current = read_attempt(&self.connection, attempt)?;
        let content = InboxContent { id: format!("worker-question-{:x}", Sha256::digest(format!("{}\0{session}\0{call}",attempt.as_str()).as_bytes())), kind: "attempt.worker_question".into(), subject: current.task.as_str().into(), created: String::new(), summary: format!("attempt {}: worker called {tool}; user questions are not answered in isolated workers; inspect or nudge the worker to state assumptions",attempt.as_str()), body: String::new() };
        self.deliver_stable_notice(self.current_head()?, &content, now)
    }
}
