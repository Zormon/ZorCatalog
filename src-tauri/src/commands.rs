use std::path::PathBuf;
use std::sync::atomic::Ordering;

use rusqlite::{Connection, OptionalExtension};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::Db;
use crate::models::{
    BackupProgress, DiskMeta, ExportSummary, Group, ImportSummary, Node, ScanProgress, SearchHit,
    SidebarEntry,
};
use crate::scanner::{self, ScanState};

fn dberr(e: rusqlite::Error) -> String {
    e.to_string()
}

/// Error legible para nombres de grupo repetidos (`name` es UNIQUE).
fn group_err(e: rusqlite::Error) -> String {
    let msg = e.to_string();
    if msg.contains("UNIQUE") {
        "Ya existe un grupo con ese nombre".into()
    } else {
        msg
    }
}

const NODE_COLS: &str =
    "id, disk_id, parent_id, name, is_dir, size, modified, path";

const DISK_COLS: &str =
    "id, name, root_path, total_size, file_count, created_at, group_id, position";

const GROUP_COLS: &str = "id, name, color, collapsed, position";

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

fn disk_from_row(r: &rusqlite::Row) -> rusqlite::Result<DiskMeta> {
    Ok(DiskMeta {
        id: r.get(0)?,
        name: r.get(1)?,
        root_path: r.get(2)?,
        total_size: r.get(3)?,
        file_count: r.get(4)?,
        created_at: r.get(5)?,
        group_id: r.get(6)?,
        position: r.get(7)?,
    })
}

fn group_from_row(r: &rusqlite::Row) -> rusqlite::Result<Group> {
    Ok(Group {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        collapsed: r.get::<_, i64>(3)? != 0,
        position: r.get(4)?,
    })
}

fn group_meta(conn: &Connection, id: i64) -> Result<Group, String> {
    conn.query_row(
        &format!("SELECT {GROUP_COLS} FROM groups WHERE id = ?1"),
        rusqlite::params![id],
        group_from_row,
    )
    .map_err(dberr)
}

#[tauri::command]
pub fn list_catalogs(db: State<Db>) -> Result<Vec<DiskMeta>, String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {DISK_COLS} FROM disks ORDER BY position, name COLLATE NOCASE"
        ))
        .map_err(dberr)?;
    let rows = stmt.query_map([], disk_from_row).map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

fn disk_meta(conn: &Connection, id: i64) -> Result<DiskMeta, String> {
    conn.query_row(
        &format!("SELECT {DISK_COLS} FROM disks WHERE id = ?1"),
        rusqlite::params![id],
        disk_from_row,
    )
    .map_err(dberr)
}

#[tauri::command]
pub fn list_groups(db: State<Db>) -> Result<Vec<Group>, String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {GROUP_COLS} FROM groups ORDER BY position, name COLLATE NOCASE"
        ))
        .map_err(dberr)?;
    let rows = stmt.query_map([], group_from_row).map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

/// Crea un grupo al final de la lista del panel lateral.
#[tauri::command]
pub fn create_group(db: State<Db>, name: String, color: Option<String>) -> Result<Group, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("El nombre del grupo no puede estar vacío".into());
    }
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let position: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM groups",
            [],
            |r| r.get(0),
        )
        .map_err(dberr)?;
    conn.execute(
        "INSERT INTO groups (name, color, position) VALUES (?1, ?2, ?3)",
        rusqlite::params![name, color, position],
    )
    .map_err(group_err)?;
    let id = conn.last_insert_rowid();
    group_meta(&conn, id)
}

/// Actualiza nombre, color y estado de colapso. El frontend manda siempre los
/// tres valores (siempre los tiene), así que no hacen falta comandos sueltos.
#[tauri::command]
pub fn update_group(
    db: State<Db>,
    id: i64,
    name: String,
    color: Option<String>,
    collapsed: bool,
) -> Result<Group, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("El nombre del grupo no puede estar vacío".into());
    }
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let changed = conn
        .execute(
            "UPDATE groups SET name = ?2, color = ?3, collapsed = ?4 WHERE id = ?1",
            rusqlite::params![id, name, color, collapsed],
        )
        .map_err(group_err)?;
    if changed == 0 {
        return Err("El grupo no existe".into());
    }
    group_meta(&conn, id)
}

/// Lógica de `delete_group`, separada del comando para poder testearla.
/// NO destructivo: los catálogos del grupo vuelven al nivel raíz, al final de
/// la lista, para no chocar con las posiciones ya existentes.
pub fn ungroup_all(conn: &Connection, group_id: i64) -> Result<(), String> {
    let mut next: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM (
               SELECT position FROM groups
               UNION ALL SELECT position FROM disks WHERE group_id IS NULL
             )",
            [],
            |r| r.get(0),
        )
        .map_err(dberr)?;
    let children: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT id FROM disks WHERE group_id = ?1 ORDER BY position")
            .map_err(dberr)?;
        let rows = stmt
            .query_map(rusqlite::params![group_id], |r| r.get(0))
            .map_err(dberr)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(dberr)?
    };
    for child in children {
        conn.execute(
            "UPDATE disks SET group_id = NULL, position = ?2 WHERE id = ?1",
            rusqlite::params![child, next],
        )
        .map_err(dberr)?;
        next += 1;
    }
    conn.execute(
        "DELETE FROM groups WHERE id = ?1",
        rusqlite::params![group_id],
    )
    .map_err(dberr)?;
    Ok(())
}

#[tauri::command]
pub fn delete_group(db: State<Db>, id: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    ungroup_all(&conn, id)
}

/// Lógica de `set_sidebar_layout`, separada del comando para poder testearla.
///
/// Reescribe TODO el orden del panel lateral desde la lista que manda el
/// frontend tras un drag & drop. Un único camino de código cubre reordenar
/// catálogos, meterlos/sacarlos de grupos y reordenar los propios grupos.
///
/// `groups.position` y `disks.position` de los catálogos sin grupo comparten
/// espacio de coordenadas: el contador `pos` recorre la lista de nivel raíz
/// entera, de modo que nunca hay empates al mezclar grupos y catálogos sueltos.
pub fn apply_sidebar_layout(conn: &mut Connection, entries: &[SidebarEntry]) -> Result<(), String> {
    let tx = conn.transaction().map_err(dberr)?;
    let mut pos: i64 = 0;
    for entry in entries {
        match entry {
            SidebarEntry::Group { id, catalogs } => {
                let changed = tx
                    .execute(
                        "UPDATE groups SET position = ?2 WHERE id = ?1",
                        rusqlite::params![id, pos],
                    )
                    .map_err(dberr)?;
                if changed == 0 {
                    return Err("El grupo indicado no existe".into());
                }
                for (i, catalog_id) in catalogs.iter().enumerate() {
                    tx.execute(
                        "UPDATE disks SET group_id = ?2, position = ?3 WHERE id = ?1",
                        rusqlite::params![catalog_id, id, i as i64],
                    )
                    .map_err(dberr)?;
                }
                pos += 1;
            }
            SidebarEntry::Catalog { id } => {
                tx.execute(
                    "UPDATE disks SET group_id = NULL, position = ?2 WHERE id = ?1",
                    rusqlite::params![id, pos],
                )
                .map_err(dberr)?;
                pos += 1;
            }
        }
    }
    tx.commit().map_err(dberr)?;
    Ok(())
}

#[tauri::command]
pub fn set_sidebar_layout(db: State<Db>, entries: Vec<SidebarEntry>) -> Result<(), String> {
    let mut guard = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    apply_sidebar_layout(&mut guard, &entries)
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
    group_id: Option<i64>,
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
        if let Some(gid) = group_id {
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM groups WHERE id = ?1)",
                    rusqlite::params![gid],
                    |r| r.get(0),
                )
                .map_err(dberr)?;
            if !exists {
                scan.active.store(false, Ordering::SeqCst);
                return Err("El grupo seleccionado ya no existe".into());
            }
        }
        // Se coloca al final de su contenedor (su grupo o el nivel raíz).
        let position: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM disks
                 WHERE group_id = ?1
                    OR (?1 IS NULL AND group_id IS NULL)",
                rusqlite::params![group_id],
                |r| r.get(0),
            )
            .map_err(dberr)?;
        conn.execute(
            "INSERT INTO disks (name, root_path, group_id, position)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![name, path, group_id, position],
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

/// SQL de la búsqueda. Vive en su propia función para que los tests usen
/// exactamente la misma consulta y no se desincronicen.
///
/// IMPORTANTE: el lado izquierdo de MATCH debe ser el nombre REAL de la tabla
/// FTS (sin alias); `f MATCH ?1` lanza "no such column: f".
///
/// Parámetros: ?1 consulta FTS, ?2 catálogo concreto, ?3 grupo completo,
/// ?4 límite. `disk_id` y `group_id` son mutuamente excluyentes.
fn search_sql() -> String {
    format!(
        "SELECT {}, d.name FROM nodes n
         JOIN disks d ON d.id = n.disk_id
         JOIN nodes_fts ON nodes_fts.rowid = n.id
         WHERE nodes_fts MATCH ?1
           AND (?2 IS NULL OR n.disk_id = ?2)
           AND (?3 IS NULL OR n.disk_id IN (SELECT id FROM disks WHERE group_id = ?3))
         ORDER BY nodes_fts.rank
         LIMIT ?4",
        node_cols_prefixed("n.")
    )
}

/// Búsqueda full-text por nombre/ruta con FTS5 (prefijos por término).
#[tauri::command]
pub fn search(
    db: State<Db>,
    query: String,
    disk_id: Option<i64>,
    group_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SearchHit>, String> {
    let mq = fts_query(&query);
    if mq.trim().is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let conn = db.0.lock().map_err(|_| "bd bloqueada".to_string())?;
    let mut stmt = conn.prepare(&search_sql()).map_err(dberr)?;
    let rows = stmt
        .query_map(rusqlite::params![mq, disk_id, group_id, limit], |r| {
            Ok(SearchHit {
                node: node_from_row(r)?,
                disk_name: r.get(8)?,
            })
        })
        .map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

/// Convierte la consulta del usuario en una expresión FTS5 por prefijos:
/// "vac tor" → "name:vac* name:tor*" y "mi-foto.jpg" → "name:mi* name:foto* name:jpg*".
/// Los separadores parten el término (el tokenizer indexa por palabras);
/// los caracteres especiales de FTS5 se descartan al no ser alfanuméricos.
///
/// El prefijo `name:` restringe la búsqueda a la columna `name` del índice FTS,
/// de modo que la ruta del nodo (columna `path`) NO cuenta para las coincidencias.
/// Así, buscar "otros" solo devuelve archivos/carpetas cuyo nombre contiene "otros",
/// y no los que simplemente están dentro de una carpeta llamada "otros".
fn fts_query(q: &str) -> String {
    q.split_whitespace()
        .flat_map(|t| {
            t.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|s| !s.is_empty())
                .map(|s| format!("name:{s}*"))
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

// --- Copias externas (.zcbak) --------------------------------------------

/// Emite el avance de una copia por el evento `backup-progress`.
struct EventReporter<'a> {
    app: &'a AppHandle,
    phase: &'static str,
}

impl crate::backup::Reporter for EventReporter<'_> {
    fn report(&mut self, stage: crate::backup::Stage, done: i64, total: i64, current: &str) {
        let _ = self.app.emit(
            "backup-progress",
            BackupProgress {
                phase: self.phase.to_string(),
                stage: stage.as_str().to_string(),
                done,
                total,
                current: current.to_string(),
            },
        );
    }
}

/// Carpeta de trabajo de las copias. Se usa la caché de la app y no el
/// directorio temporal del sistema: en Linux `/tmp` suele ser tmpfs (RAM) y el
/// payload de una copia puede ocupar cientos de MB.
fn backup_tmp_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("No se pudo preparar la carpeta de trabajo: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("No se pudo preparar la carpeta de trabajo: {e}"))?;
    Ok(dir)
}

/// Una copia hecha con un escaneo en curso guardaría un catálogo a medias.
fn ensure_idle(scan: &ScanState) -> Result<(), String> {
    if scan.active.load(Ordering::SeqCst) {
        return Err("Hay un escaneo en curso; espera a que termine y vuelve a intentarlo".into());
    }
    Ok(())
}

/// Exporta todos los catálogos a un paquete `.zcbak` comprimido.
#[tauri::command]
pub async fn export_backup(
    app: AppHandle,
    db: State<'_, Db>,
    scan: State<'_, ScanState>,
    path: String,
) -> Result<ExportSummary, String> {
    ensure_idle(&scan)?;
    let tmp_dir = backup_tmp_dir(&app)?;
    let dest = PathBuf::from(&path);
    let db2 = db.inner().clone();
    let app2 = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db2.0.lock().map_err(|_| "bd bloqueada".to_string())?;
        let mut rep = EventReporter {
            app: &app2,
            phase: "export",
        };
        crate::backup::export_backup(&mut conn, &dest, &tmp_dir, &mut rep)
    })
    .await
    .map_err(|_| "la exportación terminó inesperadamente".to_string())?
}

/// Fusiona un paquete `.zcbak` en la base de datos actual (no borra nada).
#[tauri::command]
pub async fn import_backup(
    app: AppHandle,
    db: State<'_, Db>,
    scan: State<'_, ScanState>,
    path: String,
) -> Result<ImportSummary, String> {
    ensure_idle(&scan)?;
    let tmp_dir = backup_tmp_dir(&app)?;
    let src = PathBuf::from(&path);
    let db2 = db.inner().clone();
    let app2 = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db2.0.lock().map_err(|_| "bd bloqueada".to_string())?;
        let mut rep = EventReporter {
            app: &app2,
            phase: "import",
        };
        crate::backup::import_backup(&mut conn, &src, &tmp_dir, &mut rep)
    })
    .await
    .map_err(|_| "la importación terminó inesperadamente".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MIGRATIONS;

    #[test]
    fn fts_query_genera_prefijos() {
        assert_eq!(fts_query("vac tor"), "name:vac* name:tor*");
        assert_eq!(fts_query("mi-foto.jpg"), "name:mi* name:foto* name:jpg*");
        assert_eq!(fts_query("  ***  "), "");
        assert_eq!(fts_query("año"), "name:año*");
    }

    #[test]
    fn busqueda_ignora_la_ruta() {
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
        // La carpeta se llama "otros"...
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'otros', 1, 0, NULL, 'otros')",
            rusqlite::params![root],
        )
        .unwrap();
        let otros = conn.last_insert_rowid();
        // ...y este archivo está DENTRO de "otros", pero su nombre no lo contiene.
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'foto.jpg', 0, 100, NULL, 'otros/foto.jpg')",
            rusqlite::params![otros],
        )
        .unwrap();
        // Un archivo cuyo nombre SÍ contiene la cadena.
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'otros-apuntes.txt', 0, 10, NULL, 'otros-apuntes.txt')",
            rusqlite::params![root],
        )
        .unwrap();

        // Mismo SQL que usa el comando search, con la consulta ya preparada.
        let sql = search_sql();
        let mut names: Vec<String> = {
            let mut stmt = conn.prepare(&sql).unwrap();
            let rows = stmt
                .query_map(
                    rusqlite::params![
                        fts_query("otros"),
                        Option::<i64>::None,
                        Option::<i64>::None,
                        200i64
                    ],
                    |r| r.get::<_, String>(3),
                )
                .unwrap();
            rows.collect::<Result<Vec<_>, _>>().unwrap()
        };
        // Solo la carpeta "otros" y "otros-apuntes.txt"; NUNCA "otros/foto.jpg".
        names.sort();
        assert_eq!(names, vec!["otros".to_string(), "otros-apuntes.txt".to_string()]);
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
        let sql = search_sql();
        let hits: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM ({sql})"),
                rusqlite::params![
                    "vac*",
                    Option::<i64>::None,
                    Option::<i64>::None,
                    200i64
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "vac* debe encontrar vacaciones.jpg");

        // Filtrado por disco: un disco inexistente no devuelve filas.
        let hits: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM ({sql})"),
                rusqlite::params!["vac*", 999i64, Option::<i64>::None, 200i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0);
    }

    #[test]
    fn busqueda_filtra_por_grupo() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();

        conn.execute(
            "INSERT INTO groups (id, name, position) VALUES (1, 'Viajes', 0)",
            rusqlite::params![],
        )
        .unwrap();
        // Catálogo dentro del grupo y catálogo suelto.
        conn.execute(
            "INSERT INTO disks (id, name, root_path, group_id, position)
             VALUES (1, 'dentro', 'C:/a', 1, 0)",
            rusqlite::params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO disks (id, name, root_path, group_id, position)
             VALUES (2, 'fuera', 'C:/b', NULL, 1)",
            rusqlite::params![],
        )
        .unwrap();
        for disk in [1i64, 2i64] {
            conn.execute(
                "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
                 VALUES (?1, NULL, 'root', 1, 0, NULL, '')",
                rusqlite::params![disk],
            )
            .unwrap();
            let root = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
                 VALUES (?1, ?2, 'playa.jpg', 0, 10, NULL, 'playa.jpg')",
                rusqlite::params![disk, root],
            )
            .unwrap();
        }

        let sql = search_sql();
        let count = |group: Option<i64>| -> i64 {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM ({sql})"),
                rusqlite::params!["playa*", Option::<i64>::None, group, 200i64],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(count(None), 2, "sin filtro aparecen los dos catálogos");
        assert_eq!(count(Some(1)), 1, "el grupo 1 solo contiene 'dentro'");
        assert_eq!(count(Some(99)), 0, "un grupo inexistente no devuelve nada");
    }

    fn bd_de_prueba() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::MIGRATIONS).unwrap();
        crate::db::ensure_schema(&conn).unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn
    }

    /// Grupos en orden de nivel raíz: `(nombre, position)`.
    fn grupos_ordenados(conn: &Connection) -> Vec<(String, i64)> {
        let mut stmt = conn
            .prepare("SELECT name, position FROM groups ORDER BY position")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    /// Catálogos por nombre: `(nombre, grupo, position)`.
    fn catalogs(conn: &Connection) -> Vec<(String, Option<i64>, i64)> {
        let mut stmt = conn
            .prepare("SELECT name, group_id, position FROM disks ORDER BY name")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    }

    /// Estado inicial: grupo 1 "Viajes" (fotos, playa), grupo 2 "Trabajo"
    /// (informes) y "suelto" en la raíz.
    fn siembra(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO groups (id, name, position) VALUES (1, 'Viajes', 0), (2, 'Trabajo', 1);
             INSERT INTO disks (id, name, root_path, group_id, position) VALUES
               (10, 'fotos',    'C:/a', 1,    0),
               (11, 'playa',    'C:/b', 1,    1),
               (12, 'informes', 'C:/c', 2,    0),
               (13, 'suelto',   'C:/d', NULL, 2);",
        )
        .unwrap();
    }

    #[test]
    fn layout_reordena_y_mueve_entre_grupos() {
        let mut conn = bd_de_prueba();
        siembra(&conn);
        assert_eq!(
            grupos_ordenados(&conn),
            vec![("Viajes".into(), 0), ("Trabajo".into(), 1)]
        );

        // "suelto" sube al principio, "playa" se muda de Viajes a Trabajo y
        // Viajes pasa al final.
        apply_sidebar_layout(
            &mut conn,
            &[
                SidebarEntry::Catalog { id: 13 },
                SidebarEntry::Group {
                    id: 2,
                    catalogs: vec![12, 11],
                },
                SidebarEntry::Group {
                    id: 1,
                    catalogs: vec![10],
                },
            ],
        )
        .unwrap();

        // Grupos y catálogos sueltos comparten espacio: 0 (suelto), 1 y 2 (grupos).
        assert_eq!(
            grupos_ordenados(&conn),
            vec![("Trabajo".into(), 1), ("Viajes".into(), 2)]
        );
        assert_eq!(
            catalogs(&conn),
            vec![
                ("fotos".into(), Some(1), 0),
                ("informes".into(), Some(2), 0),
                ("playa".into(), Some(2), 1),
                ("suelto".into(), None, 0),
            ]
        );
    }

    #[test]
    fn layout_es_atomico_si_un_grupo_no_existe() {
        let mut conn = bd_de_prueba();
        siembra(&conn);
        let antes = catalogs(&conn);

        // El primer grupo es válido, el segundo no: todo debe revertirse.
        let err = apply_sidebar_layout(
            &mut conn,
            &[
                SidebarEntry::Group {
                    id: 1,
                    catalogs: vec![11, 10],
                },
                SidebarEntry::Group {
                    id: 99,
                    catalogs: vec![],
                },
            ],
        );
        assert!(err.is_err());
        assert_eq!(catalogs(&conn), antes, "las posiciones no deben cambiar");
    }

    #[test]
    fn delete_group_deja_catalogos_sin_grupo() {
        let conn = bd_de_prueba();
        siembra(&conn);

        ungroup_all(&conn, 1).unwrap();

        let grupos: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0))
            .unwrap();
        assert_eq!(grupos, 1, "solo desaparece el grupo borrado");
        // Los catálogos sobreviven, sin grupo y al final de la lista raíz
        // ("suelto" ya ocupaba la posición 2).
        assert_eq!(
            catalogs(&conn),
            vec![
                ("fotos".into(), None, 3),
                ("informes".into(), Some(2), 0),
                ("playa".into(), None, 4),
                ("suelto".into(), None, 2),
            ]
        );
    }
}
