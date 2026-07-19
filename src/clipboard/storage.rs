use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::{ClipboardItem, ContentType};

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS clipboard_items (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    content_type TEXT    NOT NULL,
    text_content TEXT,
    preview      TEXT    NOT NULL DEFAULT '',
    file_paths   TEXT,
    content_hash TEXT    NOT NULL UNIQUE,
    byte_size    INTEGER NOT NULL DEFAULT 0,
    is_pinned    INTEGER NOT NULL DEFAULT 0,
    source_app   TEXT,
    created_at   TEXT    NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at   TEXT    NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_clip_created ON clipboard_items(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_clip_pinned  ON clipboard_items(is_pinned) WHERE is_pinned = 1;
CREATE INDEX IF NOT EXISTS idx_clip_hash    ON clipboard_items(content_hash);
CREATE INDEX IF NOT EXISTS idx_clip_type    ON clipboard_items(content_type);

CREATE TRIGGER IF NOT EXISTS clip_items_update_ts
AFTER UPDATE ON clipboard_items
BEGIN
    UPDATE clipboard_items SET updated_at = datetime('now','localtime')
    WHERE id = new.id;
END;
"#;

/// Thread-safe clipboard storage backed by SQLite.
#[derive(Clone)]
pub struct ClipboardStorage {
    conn: Arc<Mutex<Connection>>,
}

impl ClipboardStorage {
    /// Open (or create) the database at `%APPDATA%\MemoryCleaner\clipboard.db`.
    pub fn open() -> Result<Self> {
        let db_path = db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        let conn =
            Connection::open(&db_path).with_context(|| format!("open {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        migrate_sort_order(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a new clipboard item. Returns the new row id.
    /// If `content_hash` already exists, bumps `updated_at` and returns the existing id.
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &self,
        content_type: ContentType,
        text_content: Option<&str>,
        preview: &str,
        file_paths: Option<&[String]>,
        content_hash: &str,
        byte_size: i64,
        source_app: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let max_sort_order: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) FROM clipboard_items",
            [],
            |row| row.get(0),
        )?;
        let new_sort_order = max_sort_order + 1;
        // Try insert; on hash conflict, update timestamp and return existing id.
        let file_paths_json = file_paths.map(|fp| serde_json::to_string(fp).unwrap_or_default());
        let result = conn.execute(
            "INSERT INTO clipboard_items
                (content_type, text_content, preview, file_paths, content_hash, byte_size, source_app, sort_order)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                content_type.as_str(),
                text_content,
                preview,
                file_paths_json,
                content_hash,
                byte_size,
                source_app,
                new_sort_order,
            ],
        );
        match result {
            Ok(_) => Ok(conn.last_insert_rowid()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                // Hash exists — bump sort_order and touch updated_at.
                conn.execute(
                    "UPDATE clipboard_items
                     SET updated_at = datetime('now','localtime'),
                         sort_order = ?1
                     WHERE content_hash = ?2",
                    params![new_sort_order, content_hash],
                )?;
                let id: i64 = conn.query_row(
                    "SELECT id FROM clipboard_items WHERE content_hash = ?1",
                    params![content_hash],
                    |row| row.get(0),
                )?;
                Ok(id)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Query items, optionally filtered by content type and search text.
    /// Returns at most `limit` items, ordered by pinned first then newest.
    pub fn query(
        &self,
        content_type: Option<ContentType>,
        search: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ClipboardItem>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, content_type, text_content, preview, file_paths,
                    content_hash, byte_size, is_pinned, source_app, created_at
             FROM clipboard_items WHERE 1=1",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1usize;

        if let Some(ct) = content_type {
            sql.push_str(&format!(" AND content_type = ?{param_idx}"));
            param_values.push(Box::new(ct.as_str().to_string()));
            param_idx += 1;
        }
        if let Some(q) = search
            && !q.is_empty()
        {
            sql.push_str(&format!(
                " AND (preview LIKE ?{param_idx} OR text_content LIKE ?{param_idx})"
            ));
            param_values.push(Box::new(format!("%{q}%")));
            param_idx += 1;
        }

        sql.push_str(" ORDER BY is_pinned DESC, sort_order DESC, created_at DESC");
        sql.push_str(&format!(" LIMIT ?{param_idx}"));
        param_values.push(Box::new(limit as i64));
        param_idx += 1;
        sql.push_str(&format!(" OFFSET ?{param_idx}"));
        param_values.push(Box::new(offset as i64));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_ref.as_slice(), |row| {
            let ct_str: String = row.get(1)?;
            let fp_json: Option<String> = row.get(4)?;
            let file_paths: Option<Vec<String>> =
                fp_json.and_then(|j| serde_json::from_str(&j).ok());
            Ok(ClipboardItem {
                id: row.get(0)?,
                content_type: ContentType::parse_content_type(&ct_str),
                text_content: row.get(2)?,
                preview: row.get(3)?,
                file_paths,
                content_hash: row.get(5)?,
                byte_size: row.get(6)?,
                is_pinned: row.get::<_, i64>(7)? != 0,
                source_app: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;

        let mut items = Vec::with_capacity(limit);
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// Get total item count, optionally filtered.
    pub fn count(&self, content_type: Option<ContentType>, search: Option<&str>) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from("SELECT COUNT(*) FROM clipboard_items WHERE 1=1");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1usize;

        if let Some(ct) = content_type {
            sql.push_str(&format!(" AND content_type = ?{param_idx}"));
            param_values.push(Box::new(ct.as_str().to_string()));
            param_idx += 1;
        }
        if let Some(q) = search
            && !q.is_empty()
        {
            sql.push_str(&format!(
                " AND (preview LIKE ?{param_idx} OR text_content LIKE ?{param_idx})"
            ));
            param_values.push(Box::new(format!("%{q}%")));
        }

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn.query_row(&sql, params_ref.as_slice(), |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Swap sort order between two items (manual reorder).
    pub fn move_item_by_id(&self, from_id: i64, to_id: i64) -> Result<()> {
        if from_id == to_id {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let from_sort_order: i64 = conn.query_row(
            "SELECT sort_order FROM clipboard_items WHERE id = ?1",
            params![from_id],
            |row| row.get(0),
        )?;
        let to_sort_order: i64 = conn.query_row(
            "SELECT sort_order FROM clipboard_items WHERE id = ?1",
            params![to_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "UPDATE clipboard_items SET sort_order = ?1 WHERE id = ?2",
            params![to_sort_order, from_id],
        )?;
        conn.execute(
            "UPDATE clipboard_items SET sort_order = ?1 WHERE id = ?2",
            params![from_sort_order, to_id],
        )?;
        Ok(())
    }

    /// Toggle pin status of an item.
    pub fn toggle_pin(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE clipboard_items SET is_pinned = 1 - is_pinned WHERE id = ?1",
            params![id],
        )?;
        let pinned: i64 = conn.query_row(
            "SELECT is_pinned FROM clipboard_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(pinned != 0)
    }

    /// Delete a single item.
    pub fn delete(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM clipboard_items WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete all non-pinned items.
    pub fn clear_unpinned(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count = conn.execute("DELETE FROM clipboard_items WHERE is_pinned = 0", [])?;
        Ok(count)
    }

    /// Auto-cleanup items older than `days` (non-pinned only).
    pub fn auto_cleanup(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();
        let count = conn.execute(
            "DELETE FROM clipboard_items WHERE is_pinned = 0
             AND created_at < datetime('now', '-' || ?1 || ' days', 'localtime')",
            params![days],
        )?;
        Ok(count)
    }

    /// Get a single item by id.
    pub fn get(&self, id: i64) -> Result<Option<ClipboardItem>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, content_type, text_content, preview, file_paths,
                    content_hash, byte_size, is_pinned, source_app, created_at
             FROM clipboard_items WHERE id = ?1",
            params![id],
            |row| {
                let ct_str: String = row.get(1)?;
                let fp_json: Option<String> = row.get(4)?;
                let file_paths: Option<Vec<String>> =
                    fp_json.and_then(|j| serde_json::from_str(&j).ok());
                Ok(ClipboardItem {
                    id: row.get(0)?,
                    content_type: ContentType::parse_content_type(&ct_str),
                    text_content: row.get(2)?,
                    preview: row.get(3)?,
                    file_paths,
                    content_hash: row.get(5)?,
                    byte_size: row.get(6)?,
                    is_pinned: row.get::<_, i64>(7)? != 0,
                    source_app: row.get(8)?,
                    created_at: row.get(9)?,
                })
            },
        );
        match result {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn migrate_sort_order(conn: &Connection) -> Result<()> {
    let has_sort_order: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('clipboard_items') WHERE name = 'sort_order'",
        [],
        |row| row.get(0),
    )?;
    if !has_sort_order {
        conn.execute_batch(
            "ALTER TABLE clipboard_items ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
             UPDATE clipboard_items SET sort_order = id;
             CREATE INDEX IF NOT EXISTS idx_clip_sort_order ON clipboard_items(sort_order DESC);",
        )?;
    }
    Ok(())
}

fn db_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("MemoryCleaner").join("clipboard.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_storage() -> ClipboardStorage {
        // Use in-memory DB for tests
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch(SCHEMA_SQL).unwrap();
        migrate_sort_order(&conn).unwrap();
        ClipboardStorage {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    #[test]
    fn insert_and_query_text() {
        let s = test_storage();
        let id = s
            .insert(
                ContentType::Text,
                Some("hello world"),
                "hello world",
                None,
                "abc123",
                11,
                Some("test"),
            )
            .unwrap();
        assert!(id > 0);

        let items = s.query(None, None, 100, 0).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text_content.as_deref(), Some("hello world"));
    }

    #[test]
    fn dedup_by_hash() {
        let s = test_storage();
        let id1 = s
            .insert(
                ContentType::Text,
                Some("dup"),
                "dup",
                None,
                "same_hash",
                3,
                None,
            )
            .unwrap();
        let id2 = s
            .insert(
                ContentType::Text,
                Some("dup"),
                "dup",
                None,
                "same_hash",
                3,
                None,
            )
            .unwrap();
        assert_eq!(id1, id2);
        assert_eq!(s.query(None, None, 100, 0).unwrap().len(), 1);
    }

    #[test]
    fn pin_and_delete() {
        let s = test_storage();
        let id = s
            .insert(
                ContentType::Text,
                Some("pin me"),
                "pin me",
                None,
                "hash_pin",
                6,
                None,
            )
            .unwrap();

        assert!(s.toggle_pin(id).unwrap()); // now pinned
        s.delete(id).unwrap(); // should work even when pinned
        assert!(s.get(id).unwrap().is_none());
    }

    #[test]
    fn search_filter() {
        let s = test_storage();
        s.insert(
            ContentType::Text,
            Some("alpha bravo"),
            "alpha bravo",
            None,
            "h1",
            11,
            None,
        )
        .unwrap();
        s.insert(
            ContentType::Text,
            Some("charlie delta"),
            "charlie delta",
            None,
            "h2",
            13,
            None,
        )
        .unwrap();

        let items = s.query(None, Some("alpha"), 100, 0).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].preview, "alpha bravo");
    }

    #[test]
    fn count_items() {
        let s = test_storage();
        assert_eq!(s.count(None, None).unwrap(), 0);
        s.insert(ContentType::Text, Some("a"), "a", None, "ha", 1, None)
            .unwrap();
        assert_eq!(s.count(None, None).unwrap(), 1);
    }

    #[test]
    fn move_item_by_id_swaps_order() {
        let s = test_storage();
        let id1 = s
            .insert(
                ContentType::Text,
                Some("first"),
                "first",
                None,
                "hash1",
                5,
                None,
            )
            .unwrap();
        let id2 = s
            .insert(
                ContentType::Text,
                Some("second"),
                "second",
                None,
                "hash2",
                6,
                None,
            )
            .unwrap();

        s.move_item_by_id(id1, id2).unwrap();
        let items = s.query(None, None, 100, 0).unwrap();
        assert_eq!(items[0].id, id1);
        assert_eq!(items[1].id, id2);
    }
}
