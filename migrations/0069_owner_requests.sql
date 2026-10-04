-- Owner decisions are authority history: lifetime retention, canonical backup only.
CREATE TABLE owner_requests (
 id TEXT PRIMARY KEY,
 action TEXT NOT NULL CHECK(action IN ('cap','repository')),
 task TEXT NOT NULL,
 contract_digest TEXT NOT NULL,
 repository TEXT NOT NULL,
 summary TEXT NOT NULL,
 expires INTEGER NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('pending','approved','rejected','consumed')),
 decision_by TEXT,
 decided INTEGER
);
CREATE INDEX owner_requests_cap ON owner_requests(task,contract_digest,status,expires);
UPDATE store_meta SET schema_version = 69;
PRAGMA user_version = 69;
