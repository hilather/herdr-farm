//! Durable delivery through public store/dispatch entry points and a real writer.
#![cfg(feature = "state-store")]
use std::{path::PathBuf, time::{Duration, Instant}};
use herdr_farm::{domain::{Commit, Mutation, Operation, OperationId, Task, TaskId, TaskState},
    operations::{Claim, DeliveryState, Outcome, dispatch::{self, DeliveryAdapter, PreparedDelivery, DispatchRequest, DispatchResult}},
    store::SqliteStore};

struct LocalDelivery { database: PathBuf, effect: PathBuf, deadline: Instant }
impl DeliveryAdapter for LocalDelivery {
    type Prepared = Self;
    fn prepare(&mut self, _: &Operation) -> anyhow::Result<Self> {
        Ok(Self { database: self.database.clone(), effect: self.effect.clone(), deadline: self.deadline })
    }
}
impl PreparedDelivery for LocalDelivery {
    fn revalidate(&mut self, _: &Operation) -> anyhow::Result<()> { Ok(()) }
    fn recording_deadline(&self) -> Option<Instant> { Some(self.deadline) }
    fn deliver(&mut self, _: &Operation, _: &Claim) -> anyhow::Result<Outcome> {
        std::fs::write(&self.effect, b"delivered once")?;
        let writer = rusqlite::Connection::open(&self.database)?;
        writer.execute_batch("BEGIN IMMEDIATE")?;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(800));
            writer.execute_batch("COMMIT").unwrap();
        });
        Ok(Outcome::Confirmed { observed_identity: self.effect.display().to_string() })
    }
}

#[test]
fn delivery_records_after_competing_writer_and_respects_original_control_deadline() {
    for budget in [Duration::from_secs(5), Duration::from_millis(150)] {
        let lab = tempfile::tempdir().unwrap();
        let database = lab.path().join("state.db");
        let effect = lab.path().join("effect");
        let mut store = SqliteStore::create(&database).unwrap();
        let task = Task { id: TaskId::new("task").unwrap(), revision: 1, state: TaskState::Ready,
            title: "deliver local artifact".into(), active_attempt: None };
        let operation = Operation { id: OperationId::new("delivery").unwrap(), task: Some(task.id.clone()),
            kind: "local.artifact".into(), target: effect.display().to_string(), payload_version: 1,
            payload: serde_json::json!({}), expected_revision: 1, due_unix_ms: 0, idempotency_key: "local-artifact".into() };
        store.commit(Commit { expected_head: 0, mutations: vec![Mutation::Task { expected: None, next: task }, Mutation::Enqueue(operation.clone())] }).unwrap();
        let mut adapter = LocalDelivery { database: database.clone(), effect: effect.clone(), deadline: Instant::now() + budget };
        let request = || DispatchRequest { operation: &operation.id, expected_revision: 1, owner: "local-delivery", lease_ms: 5000 };
        let started = Instant::now();
        let result = dispatch::dispatch_one(&mut store, request(), &mut adapter, || jiff::Timestamp::now().as_millisecond()).unwrap();
        if budget.as_secs() == 5 {
            assert!(matches!(result, DispatchResult::Recorded(ref delivery) if delivery.state == DeliveryState::Confirmed));
        } else {
            assert!(matches!(result, DispatchResult::Unrecorded { ref error, .. } if error.contains("Busy") || error.contains("Deadline")), "{result:?}");
            assert!(started.elapsed() < Duration::from_millis(700), "recording exceeded its original deadline");
        }
        // Read through a reopened store, after the competing writer releases.
        std::thread::sleep(Duration::from_millis(850));
        let mut reopened = SqliteStore::open(&database).unwrap();
        let snapshot = reopened.read_snapshot(None).unwrap();
        assert_eq!(std::fs::read(&effect).unwrap(), b"delivered once");
        assert_eq!(snapshot.deliveries[0].attempts, 1);
        assert_eq!(snapshot.deliveries[0].state, if budget.as_secs() == 5 { DeliveryState::Confirmed } else { DeliveryState::Claimed });
        assert!(dispatch::dispatch_one(&mut reopened, request(), &mut adapter, || jiff::Timestamp::now().as_millisecond()).is_err());
        assert_eq!(std::fs::read(&effect).unwrap(), b"delivered once");
        assert_eq!(snapshot.events.iter().filter(|event| event.kind == "operation.outcome").count(), if budget.as_secs() == 5 { 1 } else { 0 });
    }
}
