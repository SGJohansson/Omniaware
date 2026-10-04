use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub type Result<T> = rusqlite::Result<T>;

/// Schema v1. Entries have no fixed type: `starts` => calendar, `postit` row => desktop,
/// `task` rows (parsed `- [ ]`) => todo view. Journal = all entries grouped by `created`.
const SCHEMA_V1: &str = r#"
CREATE TABLE entry(
  id      INTEGER PRIMARY KEY,
  name    TEXT UNIQUE,
  body    TEXT NOT NULL,
  created INTEGER NOT NULL,
  updated INTEGER NOT NULL,
  starts  INTEGER,
  ends    INTEGER,
  all_day INTEGER NOT NULL DEFAULT 0,
  remind  INTEGER,
  deleted INTEGER
);
CREATE INDEX entry_created ON entry(created);
CREATE INDEX entry_starts  ON entry(starts) WHERE starts IS NOT NULL;

CREATE TABLE revision(
  entry_id INTEGER NOT NULL REFERENCES entry(id) ON DELETE CASCADE,
  ts       INTEGER NOT NULL,
  body     TEXT NOT NULL
);
CREATE INDEX revision_entry ON revision(entry_id, ts);

CREATE TABLE postit(
  entry_id INTEGER PRIMARY KEY REFERENCES entry(id) ON DELETE CASCADE,
  x INTEGER, y INTEGER, w INTEGER, h INTEGER,
  color TEXT, click_through INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE task(
  entry_id INTEGER NOT NULL REFERENCES entry(id) ON DELETE CASCADE,
  line INTEGER NOT NULL, done INTEGER NOT NULL, text TEXT NOT NULL
);
CREATE INDEX task_open ON task(done, entry_id);

CREATE TABLE tag(
  entry_id INTEGER NOT NULL REFERENCES entry(id) ON DELETE CASCADE,
  tag TEXT NOT NULL
);
CREATE INDEX tag_tag ON tag(tag);

CREATE TABLE blob(hash TEXT PRIMARY KEY, mime TEXT NOT NULL, created INTEGER NOT NULL);

CREATE VIRTUAL TABLE entry_fts USING fts5(name, body, content='entry', content_rowid='id');
CREATE TRIGGER entry_ai AFTER INSERT ON entry BEGIN
  INSERT INTO entry_fts(rowid, name, body) VALUES (new.id, new.name, new.body);
END;
CREATE TRIGGER entry_ad AFTER DELETE ON entry BEGIN
  INSERT INTO entry_fts(entry_fts, rowid, name, body) VALUES ('delete', old.id, old.name, old.body);
END;
CREATE TRIGGER entry_au AFTER UPDATE OF name, body ON entry BEGIN
  INSERT INTO entry_fts(entry_fts, rowid, name, body) VALUES ('delete', old.id, old.name, old.body);
  INSERT INTO entry_fts(rowid, name, body) VALUES (new.id, new.name, new.body);
END;
"#;

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub struct Db {
    conn: Connection,
}

/// Full entry, for the editor.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: i64,
    pub name: Option<String>,
    pub body: String,
    pub created: i64,
    pub starts: Option<i64>,
}

/// Row in timeline / lists.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: i64,
    pub name: Option<String>,
    pub preview: String,
    pub images: usize,
    /// Display time: `starts` for events in range, else `created` (or `updated`/`deleted` for lists).
    pub time: i64,
    pub is_event: bool,
}

/// First non-empty line, markdown image refs stripped, capped.
pub fn preview_of(body: &str) -> String {
    let line = body
        .lines()
        .map(|l| strip_images(l).trim().to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut out: String = line.chars().take(140).collect();
    if line.chars().count() > 140 {
        out.push('…');
    }
    out
}

fn strip_images(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find("![") {
        out.push_str(&rest[..i]);
        match rest[i..].find(')') {
            Some(j) => rest = &rest[i + j + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `blob:<hash>.<ext>` references in a body.
pub fn blob_refs(body: &str) -> Vec<String> {
    body.match_indices("(blob:")
        .filter_map(|(i, _)| {
            let r = &body[i + 6..];
            r.find(')').map(|j| r[..j].to_string())
        })
        .collect()
}

fn item_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    let body: String = r.get(2)?;
    Ok(Item {
        id: r.get(0)?,
        name: r.get(1)?,
        preview: preview_of(&body),
        images: blob_refs(&body).len(),
        time: r.get(3)?,
        is_event: r.get::<_, i64>(4)? != 0,
    })
}

/// Turns user input into an FTS5 prefix query: every token must match (`"tok"*`).
fn fts_query(q: &str) -> Option<String> {
    let toks: Vec<String> = q
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!toks.is_empty()).then(|| toks.join(" "))
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))?;
        conn.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=2000;",
        )?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let v: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v < 1 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V1)?;
            tx.execute_batch("PRAGMA user_version=1")?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn insert(&self, body: &str) -> Result<i64> {
        let t = now_ms();
        self.conn.execute(
            "INSERT INTO entry(body, created, updated) VALUES (?1, ?2, ?2)",
            params![body, t],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_body(&self, id: i64, body: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE entry SET body=?1, updated=?2 WHERE id=?3 AND body IS NOT ?1",
            params![body, now_ms(), id],
        )?;
        Ok(())
    }

    pub fn delete_hard(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM entry WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn delete_soft(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE entry SET deleted=?1 WHERE id=?2", params![now_ms(), id])?;
        Ok(())
    }

    /// Snapshot unless identical to the latest revision.
    pub fn add_revision(&self, id: i64, body: &str) -> Result<()> {
        let last: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM revision WHERE entry_id=?1 ORDER BY ts DESC, rowid DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if last.as_deref() != Some(body) {
            self.conn.execute(
                "INSERT INTO revision(entry_id, ts, body) VALUES (?1, ?2, ?3)",
                params![id, now_ms(), body],
            )?;
        }
        Ok(())
    }

    /// Id of the live entry holding `name`, if any.
    pub fn name_owner(&self, name: &str) -> Result<Option<i64>> {
        self.conn
            .query_row("SELECT id FROM entry WHERE name=?1", [name], |r| r.get(0))
            .optional()
    }

    /// Assigns `name` to `id`. If another entry holds it, that entry keeps its content but
    /// loses the name (nothing is ever overwritten).
    pub fn set_name(&self, id: i64, name: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let t = now_ms();
        tx.execute(
            "UPDATE entry SET name=NULL, updated=?2 WHERE name=?1 AND id<>?3",
            params![name, t, id],
        )?;
        tx.execute("UPDATE entry SET name=?1, updated=?2 WHERE id=?3", params![name, t, id])?;
        tx.commit()
    }

    pub fn add_blob(&self, hash: &str, mime: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO blob(hash, mime, created) VALUES (?1, ?2, ?3)",
            params![hash, mime, now_ms()],
        )?;
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Entry> {
        self.conn.query_row(
            "SELECT id, name, body, created, starts FROM entry WHERE id=?1",
            [id],
            |r| {
                Ok(Entry {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    body: r.get(2)?,
                    created: r.get(3)?,
                    starts: r.get(4)?,
                })
            },
        )
    }

    pub fn clear_name(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE entry SET name=NULL, updated=?1 WHERE id=?2", params![now_ms(), id])?;
        Ok(())
    }

    pub fn restore(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE entry SET deleted=NULL WHERE id=?1", [id])?;
        Ok(())
    }

    /// Live entries created in [from, to) or starting in [from, to), oldest first.
    pub fn day_items(&self, from: i64, to: i64) -> Result<Vec<Item>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, name, body,
                    CASE WHEN starts >= ?1 AND starts < ?2 THEN starts ELSE created END AS t,
                    (starts >= ?1 AND starts < ?2) IS 1
             FROM entry
             WHERE deleted IS NULL
               AND ((created >= ?1 AND created < ?2) OR (starts >= ?1 AND starts < ?2))
             ORDER BY t",
        )?;
        st.query_map([from, to], item_row)?.collect()
    }

    /// Every timestamp (created/starts) of live entries in [from, to); caller buckets by local day.
    pub fn stamps_between(&self, from: i64, to: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare_cached(
            "SELECT created FROM entry WHERE deleted IS NULL AND created >= ?1 AND created < ?2
             UNION ALL
             SELECT starts FROM entry WHERE deleted IS NULL AND starts >= ?1 AND starts < ?2",
        )?;
        st.query_map([from, to], |r| r.get(0))?.collect()
    }

    pub fn named(&self) -> Result<Vec<Item>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, name, body, updated, 0 FROM entry
             WHERE deleted IS NULL AND name IS NOT NULL ORDER BY name COLLATE NOCASE",
        )?;
        st.query_map([], item_row)?.collect()
    }

    pub fn trash(&self) -> Result<Vec<Item>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, name, body, deleted, 0 FROM entry WHERE deleted IS NOT NULL ORDER BY deleted DESC",
        )?;
        st.query_map([], item_row)?.collect()
    }

    /// Full-text search (prefix match on every word), best first.
    pub fn search(&self, q: &str) -> Result<Vec<Item>> {
        let Some(fq) = fts_query(q) else { return Ok(Vec::new()) };
        let mut st = self.conn.prepare_cached(
            "SELECT e.id, e.name, e.body, e.created, 0
             FROM entry_fts f JOIN entry e ON e.id = f.rowid
             WHERE entry_fts MATCH ?1 AND e.deleted IS NULL
             ORDER BY rank LIMIT 60",
        )?;
        st.query_map([fq], item_row)?.collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn previews() {
        assert_eq!(preview_of("\n![](blob:ab.png)\n  hej  \nmer"), "hej");
        assert_eq!(blob_refs("x ![](blob:ab.png) y ![](blob:cd.png)"), vec!["ab.png", "cd.png"]);
    }

    use super::*;

    fn tmp() -> (Db, std::path::PathBuf) {
        let p = std::env::temp_dir().join(format!("omni-test-{}.db", now_ms()));
        (Db::open(&p).unwrap(), p)
    }

    #[test]
    fn lifecycle() {
        let (db, _p) = tmp();
        let a = db.insert("hej världen").unwrap();
        db.update_body(a, "hej världen, åäö").unwrap();
        db.add_revision(a, "hej världen, åäö").unwrap();
        db.add_revision(a, "hej världen, åäö").unwrap(); // dedup
        let n: i64 = db.conn.query_row("SELECT count(*) FROM revision", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let ids = |v: Vec<Item>| v.into_iter().map(|i| i.id).collect::<Vec<_>>();
        assert_eq!(ids(db.search("åäö").unwrap()), vec![a]);
        assert_eq!(ids(db.search("vär").unwrap()), vec![a]); // prefix

        db.set_name(a, "adress").unwrap();
        let b = db.insert("ny adress").unwrap();
        assert_eq!(db.name_owner("adress").unwrap(), Some(a));
        db.set_name(b, "adress").unwrap();
        assert_eq!(db.name_owner("adress").unwrap(), Some(b));
        let a_body: String =
            db.conn.query_row("SELECT body FROM entry WHERE id=?1", [a], |r| r.get(0)).unwrap();
        assert_eq!(a_body, "hej världen, åäö");
        assert_eq!(ids(db.search("adress").unwrap()), vec![b]);
        assert_eq!(db.named().unwrap().len(), 1);

        let day = db.day_items(0, i64::MAX).unwrap();
        assert_eq!(day.len(), 2);
        assert_eq!(db.stamps_between(0, i64::MAX).unwrap().len(), 2);

        db.delete_soft(b).unwrap();
        assert!(db.search("adress").unwrap().is_empty());
        assert_eq!(db.trash().unwrap().len(), 1);
        db.restore(b).unwrap();
        assert_eq!(db.trash().unwrap().len(), 0);

        db.delete_hard(a).unwrap();
        assert!(db.search("världen").unwrap().is_empty());
        assert!(db.search("\"").unwrap().is_empty());
    }
}
