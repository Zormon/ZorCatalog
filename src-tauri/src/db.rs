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
    ensure_schema(&conn)?;

    Ok(Db(Arc::new(Mutex::new(conn))))
}

/// Cambios de esquema que `CREATE TABLE IF NOT EXISTS` no puede aplicar sobre
/// bases de datos ya existentes (columnas nuevas). Es idempotente y converge
/// al mismo esquema tanto en bases nuevas como antiguas.
pub fn ensure_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut cols: Vec<String> = Vec::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(disks)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        for c in rows {
            cols.push(c?);
        }
    }
    let has = |name: &str| cols.iter().any(|c| c == name);

    // La FK usa ON DELETE SET NULL como red de seguridad; aun así el comando
    // `delete_group` desagrupa explícitamente antes de borrar.
    if !has("group_id") {
        conn.execute(
            "ALTER TABLE disks
             ADD COLUMN group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL",
            [],
        )?;
    }
    if !has("position") {
        conn.execute(
            "ALTER TABLE disks ADD COLUMN position INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        // Backfill único con el orden que la UI mostraba hasta ahora
        // (alfabético), para que actualizar la app no reordene el panel.
        conn.execute(
            "UPDATE disks AS d SET position = (
               SELECT COUNT(*) FROM disks d2
               WHERE d2.name COLLATE NOCASE < d.name COLLATE NOCASE
             )",
            [],
        )?;
    }
    // Debe ir después de los ALTER: en bases antiguas `group_id` no existe aún
    // cuando se ejecuta MIGRATIONS.
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_disks_group ON disks(group_id, position);")?;
    Ok(())
}

pub const MIGRATIONS: &str = "
CREATE TABLE IF NOT EXISTS groups (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  color TEXT,
  collapsed INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS disks (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  root_path TEXT NOT NULL,
  total_size INTEGER NOT NULL DEFAULT 0,
  file_count INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
  position INTEGER NOT NULL DEFAULT 0
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

    fn columna_existe(conn: &Connection, tabla: &str, nombre: &str) -> bool {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({tabla})"))
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            if r.get::<_, String>(1).unwrap() == nombre {
                return true;
            }
        }
        false
    }

    /// Réplica del esquema anterior a los grupos (sin `group_id` ni `position`).
    const ESQUEMA_ANTIGUO: &str = "
    CREATE TABLE disks (
      id INTEGER PRIMARY KEY,
      name TEXT NOT NULL UNIQUE,
      root_path TEXT NOT NULL,
      total_size INTEGER NOT NULL DEFAULT 0,
      file_count INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );";

    #[test]
    fn ensure_schema_es_idempotente_en_bd_nueva() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS).unwrap();
        ensure_schema(&conn).unwrap();
        // Repetirlo no debe fallar ni duplicar nada.
        ensure_schema(&conn).unwrap();

        assert!(columna_existe(&conn, "disks", "group_id"));
        assert!(columna_existe(&conn, "disks", "position"));
        assert!(columna_existe(&conn, "groups", "collapsed"));
    }

    #[test]
    fn ensure_schema_migra_bd_antigua_conservando_el_orden() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(ESQUEMA_ANTIGUO).unwrap();
        // Insertados en orden no alfabético a propósito.
        for name in ["zeta", "alfa", "media"] {
            conn.execute(
                "INSERT INTO disks (name, root_path) VALUES (?1, 'C:/x')",
                params![name],
            )
            .unwrap();
        }

        // `MIGRATIONS` no toca una tabla que ya existe; `ensure_schema` sí.
        conn.execute_batch(MIGRATIONS).unwrap();
        ensure_schema(&conn).unwrap();

        assert!(columna_existe(&conn, "disks", "group_id"));
        assert!(columna_existe(&conn, "disks", "position"));

        // El backfill respeta el orden alfabético que mostraba la UI.
        let mut stmt = conn
            .prepare("SELECT name FROM disks ORDER BY position")
            .unwrap();
        let orden: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(orden, vec!["alfa", "media", "zeta"]);

        // Volver a migrar no reordena ni pierde catálogos.
        ensure_schema(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM disks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

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
