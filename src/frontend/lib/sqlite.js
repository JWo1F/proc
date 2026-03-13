// SQLite WASM wrapper for log storage.
// Uses OPFS SAH pool for persistence, falls back to in-memory.

import sqlite3InitModule from "@sqlite.org/sqlite-wasm";

let db = null;

// FTS index is synced in chunks via syncFTSChunk(), separate from inserts.
let ftsSynced = 0;
let ftsCount = 0;

const SCHEMA = `
CREATE TABLE IF NOT EXISTS entries (
  idx     INTEGER PRIMARY KEY,
  process TEXT NOT NULL,
  color   TEXT NOT NULL,
  raw     TEXT NOT NULL,
  plain   TEXT NOT NULL,
  level   TEXT,
  system  INTEGER,
  ts      REAL NOT NULL,
  has_ms  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ts      ON entries (ts, idx);
CREATE INDEX IF NOT EXISTS idx_process ON entries (process);
CREATE INDEX IF NOT EXISTS idx_level   ON entries (level);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
  plain, process,
  content=entries, content_rowid=idx,
  tokenize='trigram'
);
`;

export async function initDB() {
  if (db) return;
  const sqlite3 = await sqlite3InitModule();
  try {
    if (sqlite3.installOpfsSAHPoolVfs) {
      const pool = await sqlite3.installOpfsSAHPoolVfs({
        name: "procfile",
        clearOnInit: true,
      });
      db = new pool.OpfsSAHPoolDb("/procfile-logs.db");
    } else {
      db = new sqlite3.oo1.DB(":memory:");
    }
  } catch {
    db = new sqlite3.oo1.DB(":memory:");
  }
  db.exec(SCHEMA);
}

export function closeDB() {
  if (db) {
    db.close();
    db = null;
  }
}

export function clearAll() {
  if (!db) return;
  db.exec("DROP TABLE IF EXISTS entries_fts");
  db.exec("DROP TABLE IF EXISTS entries");
  db.exec(SCHEMA);
  ftsSynced = 0;
  ftsCount = 0;
}

export function putBatch(entries) {
  if (!db || entries.length === 0) return;
  db.exec("BEGIN");
  try {
    const stmt = db.prepare(
      "INSERT OR REPLACE INTO entries (idx, process, color, raw, plain, level, system, ts, has_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
    );
    try {
      for (const e of entries) {
        stmt.bind([
          e.index, e.process, e.color, e.raw, e.plain,
          e.level || null, e.system ? 1 : 0, e.ts, e.hasMs ? 1 : 0,
        ]).stepReset();
      }
    } finally {
      stmt.finalize();
    }
    db.exec("COMMIT");
  } catch (err) {
    db.exec("ROLLBACK");
    throw err;
  }
}

// Sync up to `limit` unindexed rows into FTS. Returns count synced.
export function syncFTSChunk(limit) {
  if (!db) return 0;
  const rows = db.exec({
    sql: "SELECT idx, plain, process FROM entries WHERE idx > ? ORDER BY idx LIMIT ?",
    bind: [ftsSynced, limit],
    returnValue: "resultRows",
  });
  if (rows.length === 0) return 0;
  db.exec("BEGIN");
  try {
    const stmt = db.prepare("INSERT INTO entries_fts(rowid, plain, process) VALUES(?,?,?)");
    try {
      for (const [idx, plain, process] of rows) stmt.bind([idx, plain, process]).stepReset();
    } finally {
      stmt.finalize();
    }
    db.exec("COMMIT");
  } catch (err) {
    db.exec("ROLLBACK");
    throw err;
  }
  ftsSynced = rows[rows.length - 1][0];
  ftsCount += rows.length;
  return rows.length;
}

export function getFTSCount() {
  return ftsCount;
}

function rowToEntry(row) {
  return {
    index: row[0],
    process: row[1],
    color: row[2],
    raw: row[3],
    plain: row[4],
    level: row[5],
    system: !!row[6],
    ts: row[7],
    hasMs: !!row[8],
  };
}

export function getByIndices(indices) {
  if (!db || indices.length === 0) return [];
  const lookup = new Map();
  for (let i = 0; i < indices.length; i++) lookup.set(indices[i], i);
  const results = new Array(indices.length);
  const placeholders = indices.map(() => "?").join(",");
  const rows = db.exec({
    sql: `SELECT idx, process, color, raw, plain, level, system, ts, has_ms FROM entries WHERE idx IN (${placeholders})`,
    bind: indices,
    returnValue: "resultRows",
  });
  for (const row of rows) {
    const pos = lookup.get(row[0]);
    if (pos !== undefined) results[pos] = rowToEntry(row);
  }
  return results;
}

export function getAllSorted() {
  if (!db) return [];
  const rows = db.exec({
    sql: "SELECT idx, process, color, raw, plain, level, system, ts, has_ms FROM entries ORDER BY ts, idx",
    returnValue: "resultRows",
  });
  return rows.map(rowToEntry);
}

export function queryFilteredIndices(filter) {
  if (!db) return [];
  const conditions = [];
  const params = [];

  if (filter.hiddenProcesses && filter.hiddenProcesses.length > 0) {
    const placeholders = filter.hiddenProcesses.map(() => "?").join(",");
    conditions.push(`process NOT IN (${placeholders})`);
    params.push(...filter.hiddenProcesses);
  }

  if (filter.hiddenLevels && filter.hiddenLevels.length > 0) {
    const levelConds = [];
    const nonNoneLevels = filter.hiddenLevels.filter((l) => l !== "none");
    if (nonNoneLevels.length > 0) {
      const placeholders = nonNoneLevels.map(() => "?").join(",");
      levelConds.push(`level IN (${placeholders})`);
      params.push(...nonNoneLevels);
    }
    if (filter.hiddenLevels.includes("none")) {
      levelConds.push("level IS NULL");
    }
    conditions.push(`NOT (${levelConds.join(" OR ")})`);
  }

  // FTS5 match for plain text query
  if (filter.query) {
    // Escape double quotes in the query for FTS5
    const escaped = filter.query.replace(/"/g, '""');
    conditions.push(`idx IN (SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?)`);
    params.push(`"${escaped}"`);
  }

  // Token filtering via LIKE (tokens are already lowercased)
  if (filter.activeTokens && filter.activeTokens.length > 0) {
    for (const token of filter.activeTokens) {
      conditions.push(`(LOWER(plain) LIKE ? OR LOWER(process) LIKE ?)`);
      const pattern = `%${token.replace(/%/g, "\\%").replace(/_/g, "\\_")}%`;
      params.push(pattern, pattern);
    }
  }

  let sql = "SELECT idx FROM entries";
  if (conditions.length > 0) {
    sql += " WHERE " + conditions.join(" AND ");
  }
  sql += " ORDER BY ts, idx";

  const rows = db.exec({ sql, bind: params, returnValue: "resultRows" });
  return rows.map((r) => r[0]);
}

export function getEntryCount() {
  if (!db) return 0;
  const rows = db.exec({ sql: "SELECT COUNT(*) FROM entries", returnValue: "resultRows" });
  return rows[0][0];
}

export function getDBSize() {
  if (!db) return 0;
  const pages = db.exec({ sql: "PRAGMA page_count", returnValue: "resultRows" });
  const size = db.exec({ sql: "PRAGMA page_size", returnValue: "resultRows" });
  return pages[0][0] * size[0][0];
}

export function getProcesses() {
  if (!db) return [];
  const rows = db.exec({
    sql: "SELECT process, color FROM entries GROUP BY process ORDER BY MIN(idx)",
    returnValue: "resultRows",
  });
  return rows; // [[process, color], ...]
}

export function computeVolume() {
  if (!db) return null;
  const range = db.exec({
    sql: "SELECT MIN(ts), MAX(ts), COUNT(*) FROM entries",
    returnValue: "resultRows",
  });
  if (!range.length || range[0][2] === 0) return null;

  const [minTs, maxTs, count] = range[0];
  const span = maxTs - minTs;
  const BUCKET_TARGET = 60;
  const bucketCount = span < 1 ? 1 : Math.min(BUCKET_TARGET, Math.ceil(span));
  const interval = span / bucketCount || 1;

  const buckets = [];
  for (let i = 0; i < bucketCount; i++) {
    buckets.push({ debug: 0, info: 0, warn: 0, error: 0, fatal: 0, none: 0 });
  }

  const grouped = db.exec({
    sql: `SELECT
            CASE
              WHEN CAST((ts - ?) / ? AS INTEGER) >= ?
              THEN ? - 1
              ELSE CAST((ts - ?) / ? AS INTEGER)
            END AS bucket,
            COALESCE(level, 'none') AS lvl,
            COUNT(*) AS cnt
          FROM entries
          GROUP BY bucket, lvl`,
    bind: [minTs, interval, bucketCount, bucketCount, minTs, interval],
    returnValue: "resultRows",
  });

  for (const [bucket, lvl, cnt] of grouped) {
    if (bucket >= 0 && bucket < bucketCount && buckets[bucket][lvl] !== undefined) {
      buckets[bucket][lvl] = cnt;
    }
  }

  return { buckets, start: minTs, interval };
}

export function getFilteredEntries(filter) {
  if (!db) return [];
  const conditions = [];
  const params = [];

  if (filter.hiddenProcesses && filter.hiddenProcesses.length > 0) {
    const placeholders = filter.hiddenProcesses.map(() => "?").join(",");
    conditions.push(`process NOT IN (${placeholders})`);
    params.push(...filter.hiddenProcesses);
  }

  if (filter.hiddenLevels && filter.hiddenLevels.length > 0) {
    const levelConds = [];
    const nonNoneLevels = filter.hiddenLevels.filter((l) => l !== "none");
    if (nonNoneLevels.length > 0) {
      const placeholders = nonNoneLevels.map(() => "?").join(",");
      levelConds.push(`level IN (${placeholders})`);
      params.push(...nonNoneLevels);
    }
    if (filter.hiddenLevels.includes("none")) {
      levelConds.push("level IS NULL");
    }
    conditions.push(`NOT (${levelConds.join(" OR ")})`);
  }

  if (filter.query) {
    const escaped = filter.query.replace(/"/g, '""');
    conditions.push(`idx IN (SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?)`);
    params.push(`"${escaped}"`);
  }

  if (filter.activeTokens && filter.activeTokens.length > 0) {
    for (const token of filter.activeTokens) {
      conditions.push(`(LOWER(plain) LIKE ? OR LOWER(process) LIKE ?)`);
      const pattern = `%${token.replace(/%/g, "\\%").replace(/_/g, "\\_")}%`;
      params.push(pattern, pattern);
    }
  }

  let sql = "SELECT idx, process, color, raw, plain, level, system, ts, has_ms FROM entries";
  if (conditions.length > 0) {
    sql += " WHERE " + conditions.join(" AND ");
  }
  sql += " ORDER BY ts, idx";

  const rows = db.exec({ sql, bind: params, returnValue: "resultRows" });
  return rows.map(rowToEntry);
}
