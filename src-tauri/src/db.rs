use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Conexión SQLite compartida. `Arc<Mutex<..>>` para poder clonarla
/// y usarla dentro de `spawn_blocking` sin bloquear el runtime async.
#[derive(Clone)]
pub struct Db(pub Arc<Mutex<Connection>>);

pub fn init_db(handle: &tauri::AppHandle) -> Result<Db, Box<dyn std::error::Error>> {
    let dir = handle.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("zorcatalog.db");
    let conn = Connection::open(path)?;

    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "cache_size", -65536i64)?; // 64 MB
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    conn.execute_batch(MIGRATIONS)?;

    Ok(Db(Arc::new(Mutex::new(conn))))
}

pub const MIGRATIONS: &str = "
CREATE TABLE IF NOT EXISTS disks (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  root_path TEXT NOT NULL,
  total_size INTEGER NOT NULL DEFAULT 0,
  file_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS nodes (
  id INTEGER PRIMARY KEY,
  disk_id INTEGER NOT NULL REFERENCES disks(id) ON DELETE CASCADE,
  parent_id INTEGER REFERENCES nodes(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  is_dir INTEGER NOT NULL,
  size INTEGER,
  modified TEXT,
  path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_nodes_disk_parent ON nodes(disk_id, parent_id);
CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(name);

CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
  name,
  path,
  content='nodes',
  content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS nodes_ai AFTER INSERT ON nodes BEGIN
  INSERT INTO nodes_fts(rowid, name, path) VALUES (new.id, new.name, new.path);
END;
CREATE TRIGGER IF NOT EXISTS nodes_ad AFTER DELETE ON nodes BEGIN
  INSERT INTO nodes_fts(nodes_fts, rowid, name, path) VALUES ('delete', old.id, old.name, old.path);
END;

CREATE TABLE IF NOT EXISTS thumbs (
  node_id INTEGER PRIMARY KEY REFERENCES nodes(id) ON DELETE CASCADE,
  data BLOB NOT NULL
);
";

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn migraciones_y_fts_funcionan() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();

        conn.execute(
            "INSERT INTO disks (name, root_path) VALUES ('test', 'C:/x')",
            params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, NULL, 'root', 1, 0, NULL, '')",
            params![],
        )
        .unwrap();
        let root = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'vacaciones.jpg', 0, 100, NULL, 'fotos/vacaciones.jpg')",
            params![root],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'notas.txt', 0, 50, NULL, 'notas.txt')",
            params![root],
        )
        .unwrap();

        // El trigger de insert pobló el índice FTS.
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes_fts f
                 JOIN nodes n ON n.id = f.rowid
                 WHERE nodes_fts MATCH 'vac*'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "la búsqueda por prefijo debe encontrar vacaciones.jpg");

        // Borrar el catálogo debe vaciar nodes (CASCADE) y el FTS (trigger).
        conn.execute("DELETE FROM disks WHERE id = 1", params![]).unwrap();
        let nodos: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(nodos, 0, "el cascade debe eliminar todos los nodos");
        let fts: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts, 0, "el trigger de delete debe vaciar el índice FTS");
    }
}
