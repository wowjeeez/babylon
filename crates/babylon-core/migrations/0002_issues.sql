ALTER TABLE channels ADD COLUMN issue_prefix TEXT;
CREATE UNIQUE INDEX idx_channels_prefix ON channels(issue_prefix) WHERE issue_prefix IS NOT NULL;
CREATE TABLE issues (
  message_id INTEGER PRIMARY KEY REFERENCES messages(id),
  channel_id INTEGER NOT NULL REFERENCES channels(id),
  number     INTEGER NOT NULL,
  parent_id  INTEGER REFERENCES messages(id),
  status     TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','in_progress','blocked','closed')),
  UNIQUE(channel_id, number)
);
CREATE INDEX idx_issues_chan_status ON issues(channel_id, status);
CREATE INDEX idx_issues_parent ON issues(parent_id);
CREATE TABLE templates (
  channel_id INTEGER REFERENCES channels(id),
  name       TEXT NOT NULL,
  title      TEXT,
  body       TEXT NOT NULL,
  updated_by TEXT NOT NULL REFERENCES agents(handle),
  updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_templates_scope_name ON templates(IFNULL(channel_id,0), name);
