-- 0001_init.sql — providers, files, blobs, replicas, checksums, scans
-- Additive. Never edit once a catalog has applied this version.

PRAGMA journal_mode = WAL;

CREATE TABLE providers (
  id            TEXT PRIMARY KEY,          -- "gdrive", "local-d"
  kind          TEXT NOT NULL,             -- local|google_drive|onedrive
  account       TEXT,
  config_json   TEXT NOT NULL DEFAULT '{}',
  created_at    TEXT NOT NULL
);

CREATE TABLE locations (
  id            TEXT PRIMARY KEY,          -- volume serial or provider id
  provider_id   TEXT NOT NULL REFERENCES providers(id),
  kind          TEXT NOT NULL,             -- volume|cloud
  label         TEXT,
  root          TEXT,
  UNIQUE(provider_id, root)
);

CREATE TABLE overlay_roots (
  path          TEXT PRIMARY KEY,
  provider_id   TEXT NOT NULL,             -- may be "overlay:onedrive" before cloud provider is added
  detector      TEXT NOT NULL              -- onedrive_userfolder|drivefs_mount|drivefs_cache|cloud_filter_volume
);

CREATE TABLE scans (
  id            INTEGER PRIMARY KEY,
  provider_id   TEXT NOT NULL REFERENCES providers(id),
  started_at    TEXT NOT NULL,
  finished_at   TEXT,
  files_seen    INTEGER NOT NULL DEFAULT 0,
  status        TEXT NOT NULL              -- running|ok|error
);

CREATE TABLE files (
  id            INTEGER PRIMARY KEY,
  location_id   TEXT NOT NULL REFERENCES locations(id),
  parent_id     INTEGER REFERENCES files(id),
  path          TEXT NOT NULL,             -- provider-relative, no '..'
  name          TEXT NOT NULL,
  kind          TEXT NOT NULL DEFAULT 'byte', -- byte|directory|shortcut
  shortcut_target_ref TEXT,                -- set iff kind='shortcut'; not a replica
  is_dir        INTEGER NOT NULL DEFAULT 0,
  size          INTEGER,
  mtime_utc     TEXT,
  ntfs_file_id  TEXT,                      -- local only
  volume_serial TEXT,
  mime          TEXT,
  hydration     TEXT NOT NULL,             -- local_full|placeholder|cloud_native
  remote_ref    TEXT,                      -- provider item id
  last_scan_id  INTEGER REFERENCES scans(id),
  UNIQUE(location_id, path)
);

CREATE INDEX files_remote ON files(location_id, remote_ref);
CREATE INDEX files_name ON files(name);

CREATE TABLE blobs (
  id            INTEGER PRIMARY KEY,
  content_id    TEXT UNIQUE,               -- b3:hex, nullable until hashed
  size          INTEGER NOT NULL,
  mime          TEXT,
  duration_ms   INTEGER,
  width         INTEGER,
  height        INTEGER
);

CREATE TABLE replicas (
  id            INTEGER PRIMARY KEY,
  blob_id       INTEGER NOT NULL REFERENCES blobs(id),
  file_id       INTEGER NOT NULL REFERENCES files(id),
  state         TEXT NOT NULL,             -- verified|suspect|partial
  UNIQUE(file_id)
);

CREATE INDEX replicas_blob ON replicas(blob_id, state);

CREATE TABLE provider_checksums (
  provider_id   TEXT NOT NULL REFERENCES providers(id),
  remote_ref    TEXT NOT NULL,
  algo          TEXT NOT NULL,             -- md5|sha1|sha256|quickxor
  hex           TEXT NOT NULL,
  size          INTEGER NOT NULL,
  blob_id       INTEGER REFERENCES blobs(id),
  PRIMARY KEY (provider_id, remote_ref, algo)
);

CREATE INDEX provider_checksums_lookup ON provider_checksums(algo, hex, size);

CREATE TABLE scan_cursors (
  provider_id   TEXT PRIMARY KEY REFERENCES providers(id),
  cursor        TEXT,
  full_scan_at  TEXT,
  last_scan_at  TEXT
);

CREATE TABLE capacity_snapshots (
  id            INTEGER PRIMARY KEY,
  location_id   TEXT NOT NULL REFERENCES locations(id),
  total_bytes   INTEGER,
  used_bytes    INTEGER NOT NULL,
  free_bytes    INTEGER NOT NULL,
  quota_bytes   INTEGER,
  measured_at   TEXT NOT NULL
);

CREATE TABLE collections_cache (
  file_id       INTEGER NOT NULL REFERENCES files(id),
  collection    TEXT NOT NULL,
  PRIMARY KEY (file_id, collection)
);

-- Name+size grouping for `portage dups` only. Not last-copy. Not a merge.
-- Implemented as a query, not a table:
--   SELECT size, lower(name), group_concat(id) FROM files
--   WHERE kind='byte' GROUP BY size, lower(name) HAVING count(*) > 1;
