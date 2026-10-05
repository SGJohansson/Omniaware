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
    pub images: Vec<Img>,
}

/// One image attached to an entry. The same blob may appear several times (deliberate copies).
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    /// Attachment row id (stable handle for selection/removal).
    pub id: i64,
    /// Blob ref "<hash>.<ext>".
    pub blob: String,
}

/// Row in timeline / lists.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: i64,
    pub name: Option<String>,
    pub preview: String,
    pub images: usize,
    /// First attached image ref, for the row thumbnail.
    pub thumb: Option<String>,
    /// Every attached image ref (copies included), in order.
    pub blobs: Vec<String>,
    /// Words in the text body.
    pub words: usize,
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

/// Columns 5 and 6 of every list query: attachment count and first attachment.
const ATT_COLS: &str = "(SELECT count(*) FROM attachment a WHERE a.entry_id = e.id),
     (SELECT a.blob FROM attachment a WHERE a.entry_id = e.id ORDER BY a.pos LIMIT 1),
     (SELECT group_concat(a.blob, ',') FROM attachment a WHERE a.entry_id = e.id)";

fn item_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    let body: String = r.get(2)?;
    Ok(Item {
        id: r.get(0)?,
        name: r.get(1)?,
        preview: preview_of(&body),
        images: r.get::<_, i64>(5)? as usize,
        thumb: r.get(6)?,
        blobs: r
            .get::<_, Option<String>>(7)?
            .map(|s| s.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
        words: body.split_whitespace().filter(|w| !w.starts_with("![](")).count(),
        time: r.get(3)?,
        is_event: r.get::<_, i64>(4)? != 0,
    })
}

/// Schema v2: images become attachments instead of `![](blob:…)` text in the body.
const SCHEMA_V2: &str = r#"
CREATE TABLE attachment(
  entry_id INTEGER NOT NULL REFERENCES entry(id) ON DELETE CASCADE,
  blob     TEXT NOT NULL,
  pos      INTEGER NOT NULL,
  PRIMARY KEY (entry_id, blob)
);
"#;

/// Removes `![…](blob:…)` refs from a body and tidies the blank lines they leave.
fn strip_blob_refs(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("![") {
        let tail = &rest[i..];
        match tail.find(')') {
            Some(j) if tail[..j].contains("](blob:") => {
                out.push_str(&rest[..i]);
                rest = &tail[j + 1..];
            }
            _ => {
                out.push_str(&rest[..i + 2]);
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    let lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    let mut tidy: Vec<&str> = Vec::new();
    for l in lines {
        if l.is_empty() && tidy.last().is_none_or(|p: &&str| p.is_empty()) {
            continue;
        }
        tidy.push(l);
    }
    while tidy.last().is_some_and(|l| l.is_empty()) {
        tidy.pop();
    }
    tidy.join("\n")
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
        if v < 2 {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(SCHEMA_V2)?;
            let rows: Vec<(i64, String)> = {
                let mut st = tx.prepare("SELECT id, body FROM entry WHERE body LIKE '%](blob:%'")?;
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_>>()?
            };
            for (id, body) in rows {
                for (pos, b) in blob_refs(&body).into_iter().enumerate() {
                    tx.execute(
                        "INSERT OR IGNORE INTO attachment(entry_id, blob, pos) VALUES (?1, ?2, ?3)", // v2 table
                        params![id, b, pos as i64],
                    )?;
                }
                tx.execute("UPDATE entry SET body=?1 WHERE id=?2", params![strip_blob_refs(&body), id])?;
            }
            tx.execute_batch("PRAGMA user_version=2")?;
            tx.commit()?;
        }
        if v < 3 {
            // v3: attachments get their own id so the same image can be attached more than once.
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(
                "CREATE TABLE attachment_v3(
                   id       INTEGER PRIMARY KEY,
                   entry_id INTEGER NOT NULL REFERENCES entry(id) ON DELETE CASCADE,
                   blob     TEXT NOT NULL,
                   pos      INTEGER NOT NULL
                 );
                 INSERT INTO attachment_v3(entry_id, blob, pos)
                   SELECT entry_id, blob, pos FROM attachment ORDER BY entry_id, pos;
                 DROP TABLE attachment;
                 ALTER TABLE attachment_v3 RENAME TO attachment;
                 CREATE INDEX attachment_entry ON attachment(entry_id, pos);
                 PRAGMA user_version=3;",
            )?;
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
        let mut e = self.conn.query_row(
            "SELECT id, name, body, created, starts FROM entry WHERE id=?1",
            [id],
            |r| {
                Ok(Entry {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    body: r.get(2)?,
                    created: r.get(3)?,
                    starts: r.get(4)?,
                    images: Vec::new(),
                })
            },
        )?;
        e.images = self.attachments(id)?;
        Ok(e)
    }

    pub fn attachments(&self, id: i64) -> Result<Vec<Img>> {
        let mut st = self.conn.prepare_cached("SELECT id, blob FROM attachment WHERE entry_id=?1 ORDER BY pos, id")?;
        st.query_map([id], |r| Ok(Img { id: r.get(0)?, blob: r.get(1)? }))?.collect()
    }

    /// Appends an image to an entry; returns the attachment id.
    pub fn add_attachment(&self, id: i64, blob: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO attachment(entry_id, blob, pos)
             VALUES (?1, ?2, (SELECT coalesce(max(pos), -1) + 1 FROM attachment WHERE entry_id=?1))",
            params![id, blob],
        )?;
        let aid = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE entry SET updated=?1 WHERE id=?2", params![now_ms(), id])?;
        Ok(aid)
    }

    pub fn remove_attachment(&self, attachment_id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM attachment WHERE id=?1", [attachment_id])?;
        Ok(())
    }

    pub fn revision_count(&self, id: i64) -> Result<i64> {
        self.conn.query_row("SELECT count(*) FROM revision WHERE entry_id=?1", [id], |r| r.get(0))
    }

    /// Body and images of the most recently created live entry (to avoid re-capturing the same clipboard).
    pub fn latest(&self) -> Result<Option<(String, Vec<String>)>> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, body FROM entry WHERE deleted IS NULL ORDER BY created DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((id, body)) => Ok(Some((body, self.attachments(id)?.into_iter().map(|i| i.blob).collect()))),
            None => Ok(None),
        }
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
        let mut st = self.conn.prepare_cached(&format!(
            "SELECT e.id, e.name, e.body,
                    CASE WHEN e.starts >= ?1 AND e.starts < ?2 THEN e.starts ELSE e.created END AS t,
                    (e.starts >= ?1 AND e.starts < ?2) IS 1, {ATT_COLS}
             FROM entry e
             WHERE e.deleted IS NULL
               AND ((e.created >= ?1 AND e.created < ?2) OR (e.starts >= ?1 AND e.starts < ?2))
             ORDER BY t"
        ))?;
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
        let mut st = self.conn.prepare_cached(&format!(
            "SELECT e.id, e.name, e.body, e.updated, 0, {ATT_COLS} FROM entry e
             WHERE e.deleted IS NULL AND e.name IS NOT NULL ORDER BY e.name COLLATE NOCASE"
        ))?;
        st.query_map([], item_row)?.collect()
    }

    pub fn trash(&self) -> Result<Vec<Item>> {
        let mut st = self.conn.prepare_cached(&format!(
            "SELECT e.id, e.name, e.body, e.deleted, 0, {ATT_COLS} FROM entry e
             WHERE e.deleted IS NOT NULL ORDER BY e.deleted DESC"
        ))?;
        st.query_map([], item_row)?.collect()
    }

    /// Full-text search (prefix match on every word), best first.
    pub fn search(&self, q: &str) -> Result<Vec<Item>> {
        let Some(fq) = fts_query(q) else { return Ok(Vec::new()) };
        let mut st = self.conn.prepare_cached(&format!(
            "SELECT e.id, e.name, e.body, e.created, 0, {ATT_COLS}
             FROM entry_fts f JOIN entry e ON e.id = f.rowid
             WHERE entry_fts MATCH ?1 AND e.deleted IS NULL
             ORDER BY rank LIMIT 60"
        ))?;
        st.query_map([fq], item_row)?.collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrates_inline_images_to_attachments() {
        let p = std::env::temp_dir().join(format!("omni-mig-{}.db", now_ms()));
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch(SCHEMA_V1).unwrap();
            c.execute_batch("PRAGMA user_version=1").unwrap();
            c.execute(
                "INSERT INTO entry(body, created, updated) VALUES ('hej\n![](blob:aa11.png)\n\nslut ![x](https://x)', 1, 1)",
                [],
            )
            .unwrap();
            c.execute("INSERT INTO entry(body, created, updated) VALUES ('![](blob:bb22.png)\n', 2, 2)", []).unwrap();
        }
        let db = Db::open(&p).unwrap();
        let a = db.get(1).unwrap();
        assert_eq!(a.body, "hej\n\nslut ![x](https://x)");
        let blobs = |v: Vec<Img>| v.into_iter().map(|i| i.blob).collect::<Vec<_>>();
        assert_eq!(blobs(a.images), vec!["aa11.png"]);
        let b = db.get(2).unwrap();
        assert_eq!((b.body.as_str(), blobs(b.images.clone())), ("", vec!["bb22.png".to_string()]));
        let items = db.day_items(0, i64::MAX).unwrap();
        assert_eq!(items[1].thumb.as_deref(), Some("bb22.png"));
        // Deliberate copies are allowed and individually removable.
        let c1 = db.add_attachment(2, "cc33.png").unwrap();
        let c2 = db.add_attachment(2, "cc33.png").unwrap();
        assert_ne!(c1, c2);
        assert_eq!(blobs(db.attachments(2).unwrap()), vec!["bb22.png", "cc33.png", "cc33.png"]);
        db.remove_attachment(b.images[0].id).unwrap();
        db.remove_attachment(c1).unwrap();
        assert_eq!(db.latest().unwrap().unwrap().1, vec!["cc33.png"]);
        assert_eq!(db.attachments(2).unwrap()[0].id, c2);
    }

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
