CREATE TABLE agents (
  handle        TEXT PRIMARY KEY,
  role          TEXT,
  kind          TEXT NOT NULL DEFAULT 'agent' CHECK(kind IN ('agent','operator')),
  token_hash    BLOB NOT NULL UNIQUE,
  token_revoked INTEGER,
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE channels (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  name        TEXT UNIQUE NOT NULL,
  topic       TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK(kind IN ('channel','dm')),
  archived_at INTEGER,
  created_by  TEXT,
  created_at  INTEGER NOT NULL
);
CREATE TABLE channel_members (
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  handle     TEXT NOT NULL REFERENCES agents(handle),
  PRIMARY KEY (channel_id, handle)
);
CREATE TABLE messages (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  channel_id  INTEGER NOT NULL REFERENCES channels(id),
  author      TEXT NOT NULL REFERENCES agents(handle),
  kind        TEXT NOT NULL CHECK(kind IN ('question','answer','decision','status','note','task')),
  summary     TEXT NOT NULL,
  body        TEXT,
  reply_to    INTEGER REFERENCES messages(id),
  resolved_at INTEGER,
  resolved_by TEXT,
  created_at  INTEGER NOT NULL
);
CREATE TABLE message_mentions (
  message_id INTEGER NOT NULL REFERENCES messages(id),
  handle     TEXT NOT NULL REFERENCES agents(handle),
  PRIMARY KEY (message_id, handle)
);
CREATE TABLE subscriptions (
  handle        TEXT NOT NULL REFERENCES agents(handle),
  channel_id    INTEGER NOT NULL REFERENCES channels(id),
  last_acked_id INTEGER NOT NULL DEFAULT 0,
  active        INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (handle, channel_id)
);
CREATE INDEX idx_messages_channel_id ON messages(channel_id, id);
CREATE INDEX idx_mentions_handle ON message_mentions(handle, message_id);
CREATE INDEX idx_messages_kind_resolved ON messages(kind, resolved_at);
CREATE INDEX idx_subscriptions_channel ON subscriptions(channel_id, active, handle);
