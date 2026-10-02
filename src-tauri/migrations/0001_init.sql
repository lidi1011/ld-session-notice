CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  version INTEGER NOT NULL,
  provider TEXT NOT NULL,
  event_type TEXT NOT NULL,
  session_id TEXT,
  run_id TEXT,
  dedupe_key TEXT,
  title TEXT NOT NULL,
  content TEXT NOT NULL,
  level TEXT NOT NULL,
  project TEXT,
  cwd TEXT,
  command TEXT,
  exit_code INTEGER,
  duration_ms INTEGER,
  timestamp TEXT NOT NULL,
  received_at TEXT NOT NULL,
  raw_payload TEXT
);

CREATE INDEX IF NOT EXISTS idx_events_received_at ON events(received_at);
CREATE INDEX IF NOT EXISTS idx_events_level ON events(level);
CREATE INDEX IF NOT EXISTS idx_events_project ON events(project);

CREATE TABLE IF NOT EXISTS delivery_attempts (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  channel TEXT NOT NULL,
  dedupe_key TEXT,
  status TEXT NOT NULL,
  attempts INTEGER NOT NULL,
  last_error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_delivery_dedupe ON delivery_attempts(dedupe_key, created_at);

CREATE TABLE IF NOT EXISTS hook_installations (
  id TEXT PRIMARY KEY,
  config_path TEXT NOT NULL,
  backup_path TEXT,
  managed_block_hash TEXT,
  installed INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pending_approvals (
  id TEXT PRIMARY KEY,
  command TEXT NOT NULL,
  project TEXT,
  risk_level TEXT NOT NULL,
  rule TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
