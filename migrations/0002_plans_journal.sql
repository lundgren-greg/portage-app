-- 0002_plans_journal.sql — plans, dual residuals, journal (we_created)
-- Additive. Never edit once a catalog has applied this version.

CREATE TABLE plans (
  id              TEXT PRIMARY KEY,        -- "file-plan-" + 8 hex
  created_at      TEXT NOT NULL,
  status          TEXT NOT NULL,           -- drafted|confirmed|running|committed|aborted|unsatisfiable
  summary_json    TEXT NOT NULL,
  min_residual    INTEGER NOT NULL,        -- min trough (residual_during) over ops
  staging_reserve INTEGER NOT NULL
);

CREATE TABLE plan_ops (
  plan_id              TEXT NOT NULL REFERENCES plans(id),
  seq                  INTEGER NOT NULL,
  op_id                TEXT NOT NULL UNIQUE,
  kind                 TEXT NOT NULL,      -- upload_keep|upload_evict|evict|download|shuttle|ingest
  blob_id              INTEGER REFERENCES blobs(id),
  file_id              INTEGER REFERENCES files(id),
  size                 INTEGER NOT NULL,
  src_json             TEXT NOT NULL,
  dest_json            TEXT NOT NULL,
  residual_during_json TEXT NOT NULL,      -- {location_id: free_bytes} trough
  residual_after_json  TEXT NOT NULL,      -- {location_id: free_bytes} after op
  rollback_note        TEXT NOT NULL,
  PRIMARY KEY (plan_id, seq)
);

CREATE TABLE journal_ops (
  op_id           TEXT PRIMARY KEY,
  plan_id         TEXT NOT NULL REFERENCES plans(id),
  state           TEXT NOT NULL,
  offset          INTEGER NOT NULL DEFAULT 0,
  tmp_path        TEXT,
  session_uri     TEXT,
  we_created      INTEGER NOT NULL DEFAULT 0, -- 1 once dest object/tmp exists
  dest_remote_ref TEXT,                       -- set when cloud dest is created
  error           TEXT,
  updated_at      TEXT NOT NULL
);

CREATE TABLE apply_log (
  id              INTEGER PRIMARY KEY,
  plan_id         TEXT NOT NULL,
  op_id           TEXT,
  level           TEXT NOT NULL,
  message         TEXT NOT NULL,
  at              TEXT NOT NULL
);
