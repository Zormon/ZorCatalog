use std::path::PathBuf;
use std::sync::atomic::Ordering;

use rusqlite::{Connection, OptionalExtension};
use tauri::{AppHandle, Emitter, State};

use crate::db::Db;
use crate::models::{DiskMeta, Node, ScanProgress, SearchHit};
use crate::scanner::{self, ScanState};

fn dberr(e: rusqlite::Error) -> String {
    e.to_string()
}

const NODE_COLS: &str =
    "id, disk_id, parent_id, name, is_dir, size, modified, path";

/// Columnas de nodes cualificadas con un prefijo de tabla (p. ej. "n.").
fn node_cols_prefixed(prefix: &str) -> String {
    NODE_COLS
        .split(',')
        .map(|c| format!("{prefix}{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn node_from_row(r: &rusqlite::Row) -> rusqlite::Result<Node> {
    Ok(Node {
        id: r.get(0)?,
        disk_id: r.get(1)?,
        parent_id: r.get(2)?,
        name: r.get(3)?,
        is_dir: r.get::<_, i64>(4)? != 0,
        size: r.get(5)?,
        modified: r.get(6)?,
        path: r.get(7)?,
    })
}

#[tauri::command]
pub fn list_catalogs(db: State<Db>) -> Result<Vec<DiskMeta>, String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, root_path, total_size, file_count, created_at
             FROM disks ORDER BY name COLLATE NOCASE",
        )
        .map_err(dberr)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DiskMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                root_path: r.get(2)?,
                total_size: r.get(3)?,
                file_count: r.get(4)?,
                created_at: r.get(5)?,
            })
        })
        .map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

fn disk_meta(conn: &Connection, id: i64) -> Result<DiskMeta, String> {
    conn.query_row(
        "SELECT id, name, root_path, total_size, file_count, created_at
         FROM disks WHERE id = ?1",
        rusqlite::params![id],
        |r| {
            Ok(DiskMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                root_path: r.get(2)?,
                total_size: r.get(3)?,
                file_count: r.get(4)?,
                created_at: r.get(5)?,
            })
        },
    )
    .map_err(dberr)
}

/// Escanea la ruta y crea el catálogo. Bloquea hasta terminar
/// (el progreso llega por el evento `scan-progress`).
#[tauri::command]
pub async fn create_catalog(
    app: AppHandle,
    db: State<'_, Db>,
    scan: State<'_, ScanState>,
    name: String,
    path: String,
) -> Result<DiskMeta, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("El nombre del catálogo no puede estar vacío".into());
    }
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("La ruta no existe o no es una carpeta: {path}"));
    }

    if scan
        .active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Ya hay un escaneo en curso".into());
    }
    scan.cancel.store(false, Ordering::SeqCst);

    let disk_id = {
        let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
        conn.execute(
            "INSERT INTO disks (name, root_path) VALUES (?1, ?2)",
            rusqlite::params![name, path],
        )
        .map_err(dberr)?;
        conn.last_insert_rowid()
    };

    let db2 = db.inner().clone();
    let cancel2 = scan.cancel.clone();
    let app2 = app.clone();
    let join = tauri::async_runtime::spawn_blocking(move || {
        scanner::scan_into_db(&db2, disk_id, &root, cancel2, app2)
    });

    let result = join
        .await
        .map_err(|_| "el hilo de escaneo terminó inesperadamente".to_string())?;

    scan.active.store(false, Ordering::SeqCst);

    match result {
        Ok(()) => {
            // Asegurar que la UI vea el evento done aunque haya ido muy rápido.
            let _ = app.emit(
                "scan-progress",
                ScanProgress {
                    disk_id,
                    processed: 0,
                    current: String::new(),
                    done: true,
                },
            );
            let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
            disk_meta(&conn, disk_id)
        }
        Err(err) => {
            // Si falla (o se cancela), no dejar un catálogo vacío.
            if let Ok(conn) = db.0.lock() {
                let _ = conn.execute(
                    "DELETE FROM disks WHERE id = ?1",
                    rusqlite::params![disk_id],
                );
            }
            Err(err)
        }
    }
}

#[tauri::command]
pub fn cancel_scan(scan: State<ScanState>) {
    scan.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn is_scanning(scan: State<ScanState>) -> bool {
    scan.active.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn delete_catalog(db: State<Db>, id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    conn.execute("DELETE FROM disks WHERE id = ?1", rusqlite::params![id])
        .map_err(dberr)?;
    Ok(())
}

/// Hijos de una carpeta. `parent_id == None` devuelve los hijos de la raíz
/// del catálogo (no la raíz en sí).
#[tauri::command]
pub fn get_children(
    db: State<Db>,
    disk_id: i64,
    parent_id: Option<i64>,
) -> Result<Vec<Node>, String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let parent_id = match parent_id {
        Some(id) => id,
        None => conn
            .query_row(
                "SELECT id FROM nodes WHERE disk_id = ?1 AND parent_id IS NULL",
                rusqlite::params![disk_id],
                |r| r.get(0),
            )
            .map_err(dberr)?,
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {NODE_COLS} FROM nodes
             WHERE disk_id = ?1 AND parent_id = ?2
             ORDER BY is_dir DESC, name COLLATE NOCASE"
        ))
        .map_err(dberr)?;
    let rows = stmt
        .query_map(rusqlite::params![disk_id, parent_id], node_from_row)
        .map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

/// Camino completo de un nodo: de la raíz hacia el propio nodo (incluido).
#[tauri::command]
pub fn get_ancestors(db: State<Db>, node_id: i64) -> Result<Vec<Node>, String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    // `id` existe en `nodes` y en `up`; hay que cualificar todas las columnas.
    let mut stmt = conn
        .prepare(&format!(
            "WITH RECURSIVE up(id) AS (
               SELECT id FROM nodes WHERE id = ?1
               UNION ALL
               SELECT n.parent_id FROM nodes n JOIN up u ON n.id = u.id
             )
             SELECT {} FROM nodes n
             JOIN up u ON n.id = u.id
             WHERE u.id IS NOT NULL",
            node_cols_prefixed("n.")
        ))
        .map_err(dberr)?;
    let rows = stmt
        .query_map(rusqlite::params![node_id], node_from_row)
        .map_err(dberr)?;
    let mut chain: Vec<Node> = rows.collect::<Result<Vec<_>, _>>().map_err(dberr)?;
    // Hijo → raíz; devolvemos raíz → nodo.
    chain.reverse();
    Ok(chain)
}

/// Búsqueda full-text por nombre/ruta con FTS5 (prefijos por término).
#[tauri::command]
pub fn search(
    db: State<Db>,
    query: String,
    disk_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SearchHit>, String> {
    let mq = fts_query(&query);
    if mq.trim().is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    // IMPORTANTE: el lado izquierdo de MATCH debe ser el nombre REAL de la
    // tabla FTS (sin alias); `f MATCH ?1` lanza "no such column: f".
    let sql = format!(
        "SELECT {}, d.name FROM nodes n
         JOIN disks d ON d.id = n.disk_id
         JOIN nodes_fts ON nodes_fts.rowid = n.id
         WHERE nodes_fts MATCH ?1 AND (?2 IS NULL OR n.disk_id = ?2)
         ORDER BY nodes_fts.rank
         LIMIT ?3",
        node_cols_prefixed("n.")
    );
    let mut stmt = conn.prepare(&sql).map_err(dberr)?;
    let rows = stmt
        .query_map(rusqlite::params![mq, disk_id, limit], |r| {
            Ok(SearchHit {
                node: node_from_row(r)?,
                disk_name: r.get(8)?,
            })
        })
        .map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

/// Convierte la consulta del usuario en una expresión FTS5 por prefijos:
/// "vac tor" → "vac* tor*" y "mi-foto.jpg" → "mi* foto* jpg*".
/// Los separadores parten el término (el tokenizer indexa por palabras);
/// los caracteres especiales de FTS5 se descartan al no ser alfanuméricos.
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .flat_map(|t| {
            t.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|s| !s.is_empty())
                .map(|s| format!("{s}*"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[tauri::command]
pub fn get_stats(db: State<Db>, disk_id: i64) -> Result<crate::models::DiskStats, String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT
               COALESCE(SUM(CASE WHEN is_dir = 0 THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN is_dir = 1 AND parent_id IS NOT NULL THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN is_dir = 0 THEN size ELSE 0 END), 0)
             FROM nodes WHERE disk_id = ?1",
        )
        .map_err(dberr)?;
    let (file_count, dir_count, total_size) = stmt
        .query_row(rusqlite::params![disk_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(dberr)?;
    let thumb_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM thumbs t JOIN nodes n ON n.id = t.node_id WHERE n.disk_id = ?1",
            rusqlite::params![disk_id],
            |r| r.get(0),
        )
        .map_err(dberr)?;
    Ok(crate::models::DiskStats {
        file_count,
        dir_count,
        total_size,
        thumb_count,
    })
}

/// Devuelve el BLOB de la miniatura (lo consume el protocolo custom `thumb://`).
pub fn thumb_blob(conn: &Connection, node_id: i64) -> Result<Vec<u8>, String> {
    conn.query_row(
        "SELECT data FROM thumbs WHERE node_id = ?1",
        rusqlite::params![node_id],
        |r| r.get::<_, Option<Vec<u8>>>(0),
    )
    .optional()
    .map_err(dberr)?
    .map(|o| o.unwrap_or_default())
    .ok_or_else(|| "sin miniatura".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MIGRATIONS;

    #[test]
    fn fts_query_genera_prefijos() {
        assert_eq!(fts_query("vac tor"), "vac* tor*");
        assert_eq!(fts_query("mi-foto.jpg"), "mi* foto* jpg*");
        assert_eq!(fts_query("  ***  "), "");
        assert_eq!(fts_query("año"), "año*");
    }

    #[test]
    fn busqueda_fts_end_to_end() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();

        conn.execute(
            "INSERT INTO disks (name, root_path) VALUES ('discos', 'C:/x')",
            rusqlite::params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, NULL, 'root', 1, 0, NULL, '')",
            rusqlite::params![],
        )
        .unwrap();
        let root = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'vacaciones.jpg', 0, 100, NULL, 'fotos/vacaciones.jpg')",
            rusqlite::params![root],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'notas.txt', 0, 50, NULL, 'notas.txt')",
            rusqlite::params![root],
        )
        .unwrap();

        // Mismo SQL que usa el comando search (regresión del bug de alias).
        let sql = format!(
            "SELECT {}, d.name FROM nodes n
             JOIN disks d ON d.id = n.disk_id
             JOIN nodes_fts ON nodes_fts.rowid = n.id
             WHERE nodes_fts MATCH ?1 AND (?2 IS NULL OR n.disk_id = ?2)
             ORDER BY nodes_fts.rank
             LIMIT ?3",
            node_cols_prefixed("n.")
        );
        let hits: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM ({sql})"),
                rusqlite::params!["vac*", Option::<i64>::None, 200i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "vac* debe encontrar vacaciones.jpg");

        // Filtrado por disco: un disco inexistente no devuelve filas.
        let hits: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM ({sql})"),
                rusqlite::params!["vac*", 999i64, 200i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }
}
