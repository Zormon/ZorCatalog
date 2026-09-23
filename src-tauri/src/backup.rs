//! Copias externas de los catálogos: exportar e importar.
//!
//! El fichero resultante (`.zcbak`) es un contenedor propio con esta forma:
//!
//! ```text
//! 0   : magic  = b"ZCBK"      (4 bytes)
//! 4   : versión de formato u16 little-endian
//! 6   : reservado u16         (0 por ahora)
//! 8   : longitud del manifiesto u32 little-endian
//! 12  : manifiesto JSON (UTF-8)
//! ... : flujo gzip -> payload SQLite
//! ```
//!
//! El manifiesto va **sin comprimir** a propósito: así se puede rechazar un
//! fichero ajeno, avisar de una copia de una versión más reciente y leer los
//! recuentos sin descomprimir nada. La versión vive en la cabecera (y no en el
//! manifiesto) para poder dar ese aviso aunque el JSON cambie en el futuro.
//!
//! El payload es un SQLite con `meta`, `groups`, `disks`, `nodes` y `thumbs`
//! (sin FTS: al importar, el trigger `nodes_ai` de la BD real la reconstruye).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::{Compression, GzBuilder};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::models::{ExportSummary, ImportSummary, RenamedCatalog};

pub const MAGIC: &[u8; 4] = b"ZCBK";
/// Versión del contenedor. Al cambiarla hay que decidir qué hacer con las
/// copias antiguas (hoy no hay ninguna publicada, así que no hay migración).
pub const FORMAT_VERSION: u32 = 1;
pub const BACKUP_EXT: &str = "zcbak";

const HEADER_LEN: usize = 12;
/// Tope del manifiesto: evita reservar memoria a lo loco con un fichero
/// corrupto que declare una longitud absurda.
const MAX_MANIFEST: u32 = 64 * 1024;
/// Nivel de compresión. Las miniaturas son JPEG (apenas comprimen); el ahorro
/// real viene de `nodes` y de las rutas.
const LEVEL: u32 = 6;
const BUF: usize = 256 * 1024;

const NO_ES_COPIA: &str = "El archivo no es una copia de ZorCatalog";
const DANADA: &str = "La copia está dañada o está incompleta";

fn dberr(e: rusqlite::Error) -> String {
    e.to_string()
}

fn ioerr(e: std::io::Error) -> String {
    e.to_string()
}

/// Etapa del proceso, para informar a la interfaz.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Volcado de la BD real al payload.
    Db,
    /// Empaquetado (gzip) del payload.
    Compress,
    /// Desempaquetado (gunzip) del payload.
    Decompress,
    /// Fusión del payload en la BD real.
    Merge,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Db => "db",
            Stage::Compress => "compress",
            Stage::Decompress => "decompress",
            Stage::Merge => "merge",
        }
    }
}

/// Informa del avance. Los comandos lo implementan emitiendo eventos Tauri y
/// los tests usan [`Silent`].
pub trait Reporter {
    fn report(&mut self, stage: Stage, done: i64, total: i64, current: &str);
}

/// Reporter que no hace nada (tests).
#[cfg(test)]
pub struct Silent;

#[cfg(test)]
impl Reporter for Silent {
    fn report(&mut self, _stage: Stage, _done: i64, _total: i64, _current: &str) {}
}

/// Cabecera del contenedor, en el orden en que se escribe.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub app_version: String,
    pub exported_at: String,
    pub catalogs: i64,
    pub groups: i64,
    pub nodes: i64,
    pub thumbs: i64,
    /// Tamaño del payload sin comprimir, para poder mostrar el avance.
    pub payload_bytes: u64,
}

/// Recuento de filas del payload.
#[derive(Clone, Copy)]
struct Counts {
    catalogs: i64,
    groups: i64,
    nodes: i64,
    thumbs: i64,
}

/// Fila de `src.disks` que se está importando.
struct SrcDisk {
    id: i64,
    name: String,
    root_path: String,
    total_size: i64,
    file_count: i64,
    created_at: String,
    group_id: Option<i64>,
}

fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Esquema del payload. Las tablas van cualificadas con el alias del `ATTACH`.
/// No hay FTS ni claves foráneas: el payload solo se lee, y así se copia e
/// importa más rápido. El índice de `nodes` sí interesa, porque la importación
/// recorre el árbol por niveles.
fn backup_schema(alias: &str) -> String {
    format!(
        "CREATE TABLE {alias}.meta (
           format_version INTEGER NOT NULL,
           app_version TEXT NOT NULL,
           exported_at TEXT NOT NULL,
           catalogs INTEGER NOT NULL,
           groups INTEGER NOT NULL,
           nodes INTEGER NOT NULL,
           thumbs INTEGER NOT NULL
         );
         CREATE TABLE {alias}.groups (
           id INTEGER PRIMARY KEY,
           name TEXT NOT NULL,
           color TEXT,
           collapsed INTEGER NOT NULL DEFAULT 0,
           position INTEGER NOT NULL DEFAULT 0,
           created_at TEXT NOT NULL
         );
         CREATE TABLE {alias}.disks (
           id INTEGER PRIMARY KEY,
           name TEXT NOT NULL,
           root_path TEXT NOT NULL,
           total_size INTEGER NOT NULL DEFAULT 0,
           file_count INTEGER NOT NULL DEFAULT 0,
           created_at TEXT NOT NULL,
           group_id INTEGER,
           position INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE {alias}.nodes (
           id INTEGER PRIMARY KEY,
           disk_id INTEGER NOT NULL,
           parent_id INTEGER,
           name TEXT NOT NULL,
           is_dir INTEGER NOT NULL,
           size INTEGER,
           modified TEXT,
           path TEXT NOT NULL
         );
         CREATE INDEX {alias}.idx_nodes_disk_parent ON nodes(disk_id, parent_id);
         CREATE TABLE {alias}.thumbs (
           node_id INTEGER PRIMARY KEY,
           data BLOB NOT NULL
         );"
    )
}

// Listas de columnas explícitas, como en el resto del proyecto: los
// `ALTER TABLE` añaden columnas al final y un `SELECT *` se desincronizaría.
const COPY_GROUPS: &str = "INSERT INTO bak.groups (id, name, color, collapsed, position, created_at)
                           SELECT id, name, color, collapsed, position, created_at FROM groups";

const COPY_DISKS: &str = "INSERT INTO bak.disks
                            (id, name, root_path, total_size, file_count, created_at, group_id, position)
                          SELECT id, name, root_path, total_size, file_count, created_at, group_id, position
                          FROM disks";

const COPY_NODES: &str = "INSERT INTO bak.nodes (id, disk_id, parent_id, name, is_dir, size, modified, path)
                          SELECT id, disk_id, parent_id, name, is_dir, size, modified, path
                          FROM nodes WHERE disk_id = ?1";

const COPY_THUMBS: &str = "INSERT INTO bak.thumbs (node_id, data)
                           SELECT node_id, data FROM thumbs";

/// Ruta única para el fichero de trabajo. Vive en la caché de la app (no en el
/// directorio temporal del sistema: en Linux `/tmp` suele ser tmpfs, o sea RAM)
/// y se borra siempre al terminar, salga bien o mal.
fn tmp_path(dir: &Path, tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!("zorcatalog-{tag}-{}-{nanos}.tmp", std::process::id()))
}

/// SQLite necesita la ruta como texto.
fn path_str(p: &Path) -> Result<String, String> {
    p.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "La ruta elegida no es válida".to_string())
}

/// El diálogo nativo no siempre añade la extensión; sin ella, el fichero se
/// quedaría fuera del filtro del diálogo de importar. Se añade al final (no se
/// sustituye) para no cargarse nombres con puntos.
fn with_ext(path: &Path) -> PathBuf {
    let tiene = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(BACKUP_EXT))
        .unwrap_or(false);
    if tiene {
        return path.to_path_buf();
    }
    let mut s = path.as_os_str().to_os_string();
    s.push(".");
    s.push(BACKUP_EXT);
    PathBuf::from(s)
}

// --- Cabecera ------------------------------------------------------------

fn write_header<W: Write>(w: &mut W, m: &Manifest) -> Result<(), String> {
    let json = serde_json::to_vec(m).map_err(|e| e.to_string())?;
    if json.len() as u32 > MAX_MANIFEST {
        return Err("El manifiesto de la copia es demasiado grande".into());
    }
    w.write_all(MAGIC).map_err(ioerr)?;
    w.write_all(&(FORMAT_VERSION as u16).to_le_bytes())
        .map_err(ioerr)?;
    w.write_all(&0u16.to_le_bytes()).map_err(ioerr)?;
    w.write_all(&(json.len() as u32).to_le_bytes())
        .map_err(ioerr)?;
    w.write_all(&json).map_err(ioerr)?;
    Ok(())
}

fn read_header<R: Read>(r: &mut R) -> Result<Manifest, String> {
    let mut head = [0u8; HEADER_LEN];
    r.read_exact(&mut head).map_err(|_| NO_ES_COPIA.to_string())?;
    if &head[0..4] != MAGIC {
        return Err(NO_ES_COPIA.into());
    }
    let version = u16::from_le_bytes([head[4], head[5]]) as u32;
    if version > FORMAT_VERSION {
        return Err(format!(
            "La copia se creó con una versión más reciente de ZorCatalog (formato {version}); \
             actualiza la aplicación para poder importarla."
        ));
    }
    let len = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    if len == 0 || len > MAX_MANIFEST {
        return Err(DANADA.into());
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).map_err(|_| DANADA.to_string())?;
    serde_json::from_slice(&buf).map_err(|_| DANADA.to_string())
}

// --- Compresión por streaming -------------------------------------------

/// Comprime `src` dentro de `dst` sin cargarlo entero en memoria.
fn gzip_into(
    src: &Path,
    dst: &mut impl Write,
    total: u64,
    rep: &mut dyn Reporter,
) -> Result<(), String> {
    let file = File::open(src).map_err(ioerr)?;
    let mut input = BufReader::with_capacity(BUF, file);
    let mut enc: GzEncoder<&mut dyn Write> = GzBuilder::new()
        .filename("zorcatalog.db")
        .mtime(0)
        .write(dst, Compression::new(LEVEL));

    let mut buf = vec![0u8; BUF];
    let mut done: u64 = 0;
    loop {
        let n = input.read(&mut buf).map_err(ioerr)?;
        if n == 0 {
            break;
        }
        enc.write_all(&buf[..n]).map_err(ioerr)?;
        done += n as u64;
        rep.report(Stage::Compress, done as i64, total as i64, "");
    }
    enc.finish().map_err(ioerr)?;
    Ok(())
}

/// Descomprime `src` en `dst`, comprobando que salen todos los bytes que
/// anunciaba el manifiesto.
fn gunzip_into(
    src: &mut impl Read,
    dst: &Path,
    total: u64,
    rep: &mut dyn Reporter,
) -> Result<(), String> {
    let file = File::create(dst).map_err(ioerr)?;
    let mut out = BufWriter::with_capacity(BUF, file);
    let mut dec = GzDecoder::new(src);

    let mut buf = vec![0u8; BUF];
    let mut done: u64 = 0;
    loop {
        let n = dec.read(&mut buf).map_err(|_| DANADA.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(ioerr)?;
        done += n as u64;
        rep.report(Stage::Decompress, done as i64, total as i64, "");
    }
    out.flush().map_err(ioerr)?;
    drop(out);

    if total > 0 && done != total {
        return Err(DANADA.into());
    }
    Ok(())
}

// --- Exportar ------------------------------------------------------------

/// Escribe en `path` una copia de todos los catálogos.
pub fn export_backup(
    conn: &mut Connection,
    path: &Path,
    tmp_dir: &Path,
    rep: &mut dyn Reporter,
) -> Result<ExportSummary, String> {
    let path = with_ext(path);
    // El diálogo nativo ya ha preguntado antes de sobreescribir.
    if path.exists() {
        std::fs::remove_file(&path).map_err(ioerr)?;
    }

    let tmp = tmp_path(tmp_dir, "payload");
    let _ = std::fs::remove_file(&tmp);

    let result = build_payload(conn, &tmp, rep)
        .and_then(|counts| write_package(&tmp, &path, counts, rep));

    let _ = std::fs::remove_file(&tmp);
    if result.is_err() {
        // No dejar una copia a medias si algo ha fallado a mitad.
        let _ = std::fs::remove_file(&path);
    }
    result
}

/// Vuelca la BD real al SQLite temporal. Devuelve el recuento de filas.
fn build_payload(
    conn: &mut Connection,
    tmp: &Path,
    rep: &mut dyn Reporter,
) -> Result<Counts, String> {
    // `ATTACH` fuera de cualquier transacción: SQLite no lo admite dentro.
    conn.execute("ATTACH DATABASE ?1 AS bak", rusqlite::params![path_str(tmp)?])
        .map_err(|e| format!("No se pudo preparar la copia: {e}"))?;

    let inner = copy_into_payload(conn, rep);
    // El `DETACH` tiene que ocurrir SIEMPRE: si la conexión se queda con `bak`
    // adjunta, el siguiente `ATTACH` falla con "database bak is already in use".
    let detach = conn.execute_batch("DETACH DATABASE bak").map_err(dberr);

    match (inner, detach) {
        (Ok(counts), Ok(())) => Ok(counts),
        (Err(e), _) => Err(e),
        (Ok(_), Err(e)) => Err(e),
    }
}

fn copy_into_payload(conn: &mut Connection, rep: &mut dyn Reporter) -> Result<Counts, String> {
    let counts = read_counts(conn)?;
    let disks: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name FROM disks ORDER BY position, name COLLATE NOCASE")
            .map_err(dberr)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(dberr)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(dberr)?
    };
    let total = disks.len() as i64;

    let tx = conn.transaction().map_err(dberr)?;
    tx.execute_batch(&backup_schema("bak")).map_err(dberr)?;

    tx.execute(
        "INSERT INTO bak.meta
           (format_version, app_version, exported_at, catalogs, groups, nodes, thumbs)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            FORMAT_VERSION as i64,
            env!("CARGO_PKG_VERSION"),
            now_iso(),
            counts.catalogs,
            counts.groups,
            counts.nodes,
            counts.thumbs,
        ],
    )
    .map_err(dberr)?;

    rep.report(Stage::Db, 0, total, "grupos y catálogos");
    tx.execute_batch(COPY_GROUPS).map_err(dberr)?;
    tx.execute_batch(COPY_DISKS).map_err(dberr)?;

    // Los nodos, catálogo a catálogo, para poder informar del avance.
    for (i, (id, name)) in disks.iter().enumerate() {
        tx.execute(COPY_NODES, rusqlite::params![id]).map_err(dberr)?;
        rep.report(Stage::Db, (i + 1) as i64, total, name);
    }

    // Las miniaturas no cuelgan de `disks`, así que van de una vez.
    rep.report(Stage::Db, total, total, "miniaturas");
    tx.execute_batch(COPY_THUMBS).map_err(dberr)?;

    tx.commit().map_err(dberr)?;
    Ok(counts)
}

fn read_counts(conn: &Connection) -> Result<Counts, String> {
    let uno = |sql: &str| -> Result<i64, String> {
        conn.query_row(sql, [], |r| r.get(0)).map_err(dberr)
    };
    Ok(Counts {
        catalogs: uno("SELECT COUNT(*) FROM disks")?,
        groups: uno("SELECT COUNT(*) FROM groups")?,
        nodes: uno("SELECT COUNT(*) FROM nodes")?,
        thumbs: uno("SELECT COUNT(*) FROM thumbs")?,
    })
}

/// Escribe la cabecera, el manifiesto y el payload comprimido en `path`.
fn write_package(
    tmp: &Path,
    path: &Path,
    counts: Counts,
    rep: &mut dyn Reporter,
) -> Result<ExportSummary, String> {
    let payload_bytes = std::fs::metadata(tmp).map_err(ioerr)?.len();
    let manifest = Manifest {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: now_iso(),
        catalogs: counts.catalogs,
        groups: counts.groups,
        nodes: counts.nodes,
        thumbs: counts.thumbs,
        payload_bytes,
    };

    {
        let file = File::create(path).map_err(ioerr)?;
        let mut out = BufWriter::with_capacity(BUF, file);
        write_header(&mut out, &manifest)?;
        gzip_into(tmp, &mut out, payload_bytes, rep)?;
        out.flush().map_err(ioerr)?;
    }

    let file_bytes = std::fs::metadata(path).map_err(ioerr)?.len();
    Ok(ExportSummary {
        catalogs: counts.catalogs,
        groups: counts.groups,
        nodes: counts.nodes,
        thumbs: counts.thumbs,
        payload_bytes,
        file_bytes,
        file_path: path.to_string_lossy().into_owned(),
    })
}

// --- Importar ------------------------------------------------------------

/// Fusiona la copia de `path` en la BD real. Nunca borra nada.
pub fn import_backup(
    conn: &mut Connection,
    path: &Path,
    tmp_dir: &Path,
    rep: &mut dyn Reporter,
) -> Result<ImportSummary, String> {
    let tmp = tmp_path(tmp_dir, "import");
    let _ = std::fs::remove_file(&tmp);

    let result = (|| -> Result<ImportSummary, String> {
        let file = File::open(path).map_err(ioerr)?;
        let mut input = BufReader::with_capacity(BUF, file);
        let manifest = read_header(&mut input)?;
        gunzip_into(&mut input, &tmp, manifest.payload_bytes, rep)?;
        merge_payload(conn, &tmp, rep)
    })();

    let _ = std::fs::remove_file(&tmp);
    result
}

fn merge_payload(
    conn: &mut Connection,
    tmp: &Path,
    rep: &mut dyn Reporter,
) -> Result<ImportSummary, String> {
    conn.execute("ATTACH DATABASE ?1 AS src", rusqlite::params![path_str(tmp)?])
        .map_err(|_| DANADA.to_string())?;

    let inner = merge_into_db(conn, rep);
    let detach = conn.execute_batch("DETACH DATABASE src").map_err(dberr);

    match (inner, detach) {
        (Ok(s), Ok(())) => Ok(s),
        (Err(e), _) => Err(e),
        (Ok(_), Err(e)) => Err(e),
    }
}

fn merge_into_db(conn: &mut Connection, rep: &mut dyn Reporter) -> Result<ImportSummary, String> {
    // Si el payload no es un SQLite nuestro, esto falla aquí y no tocamos nada.
    let version: i64 = conn
        .query_row("SELECT format_version FROM src.meta LIMIT 1", [], |r| r.get(0))
        .map_err(|_| DANADA.to_string())?;
    if version > FORMAT_VERSION as i64 {
        return Err(format!(
            "La copia se creó con una versión más reciente de ZorCatalog (formato {version}); \
             actualiza la aplicación para poder importarla."
        ));
    }

    let tx = conn.transaction().map_err(dberr)?;
    // Tabla de paso para poder copiar las miniaturas de golpe al final, con un
    // JOIN en vez de una consulta por nodo. Es TEMP y vive en la conexión, así
    // que hay que tirarla antes de crear la de esta importación.
    tx.execute_batch(
        "DROP TABLE IF EXISTS node_map;
         CREATE TEMP TABLE node_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);",
    )
    .map_err(dberr)?;

    let mut groups_created = 0i64;
    let mut groups_reused = 0i64;
    let group_map = merge_groups(&tx, &mut groups_created, &mut groups_reused)?;

    let disks = read_src_disks(&tx)?;
    let total = disks.len() as i64;
    let mut summary = ImportSummary {
        catalogs_imported: 0,
        catalogs_skipped: Vec::new(),
        catalogs_renamed: Vec::new(),
        groups_created,
        groups_reused,
        nodes: 0,
    };

    for (i, d) in disks.iter().enumerate() {
        // Duplicado exacto: ya está (misma ruta), no lo repetimos.
        let duplicado: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM disks WHERE name = ?1 AND root_path = ?2)",
                rusqlite::params![d.name, d.root_path],
                |r| r.get(0),
            )
            .map_err(dberr)?;
        if duplicado {
            summary.catalogs_skipped.push(d.name.clone());
            rep.report(Stage::Merge, (i + 1) as i64, total, &d.name);
            continue;
        }

        let name = free_name(&tx, &d.name)?;
        if name != d.name {
            summary.catalogs_renamed.push(RenamedCatalog {
                from: d.name.clone(),
                to: name.clone(),
            });
        }

        let group_id = d.group_id.and_then(|g| group_map.get(&g).copied());
        let position = match group_id {
            Some(g) => next_group_position(&tx, g)?,
            None => next_root_position(&tx)?,
        };
        let new_id: i64 = tx
            .query_row(
                "INSERT INTO disks
                   (name, root_path, total_size, file_count, created_at, group_id, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id",
                rusqlite::params![
                    name,
                    d.root_path,
                    d.total_size,
                    d.file_count,
                    d.created_at,
                    group_id,
                    position,
                ],
                |r| r.get(0),
            )
            .map_err(dberr)?;

        summary.nodes += copy_nodes(&tx, d.id, new_id)?;
        summary.catalogs_imported += 1;
        rep.report(Stage::Merge, (i + 1) as i64, total, &name);
    }

    // Las miniaturas de todos los catálogos importados, de una vez.
    tx.execute(
        "INSERT INTO thumbs (node_id, data)
         SELECT m.new_id, t.data FROM src.thumbs t JOIN node_map m ON m.old_id = t.node_id",
        [],
    )
    .map_err(dberr)?;

    tx.commit().map_err(dberr)?;
    Ok(summary)
}

/// Mapea cada grupo del payload a un grupo de la BD real: reutiliza el que ya
/// se llame igual (sin tocarle color, colapso ni posición) o lo crea al final.
fn merge_groups(
    conn: &Connection,
    created: &mut i64,
    reused: &mut i64,
) -> Result<HashMap<i64, i64>, String> {
    let src: Vec<(i64, String, Option<String>, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, color, collapsed FROM src.groups
                 ORDER BY position, name COLLATE NOCASE",
            )
            .map_err(dberr)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(dberr)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(dberr)?
    };

    let mut map = HashMap::new();
    for (old_id, name, color, collapsed) in src {
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM groups WHERE name = ?1",
                rusqlite::params![name],
                |r| r.get(0),
            )
            .optional()
            .map_err(dberr)?;
        let new_id = match existing {
            Some(id) => {
                *reused += 1;
                id
            }
            None => {
                let position = next_root_position(conn)?;
                let id: i64 = conn
                    .query_row(
                        "INSERT INTO groups (name, color, collapsed, position)
                         VALUES (?1, ?2, ?3, ?4) RETURNING id",
                        rusqlite::params![name, color, collapsed, position],
                        |r| r.get(0),
                    )
                    .map_err(|e| {
                        let msg = e.to_string();
                        if msg.contains("UNIQUE") {
                            format!("Ya existe un grupo llamado «{name}»")
                        } else {
                            msg
                        }
                    })?;
                *created += 1;
                id
            }
        };
        map.insert(old_id, new_id);
    }
    Ok(map)
}

fn read_src_disks(conn: &Connection) -> Result<Vec<SrcDisk>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, root_path, total_size, file_count, created_at, group_id
             FROM src.disks ORDER BY position, name COLLATE NOCASE",
        )
        .map_err(dberr)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SrcDisk {
                id: r.get(0)?,
                name: r.get(1)?,
                root_path: r.get(2)?,
                total_size: r.get(3)?,
                file_count: r.get(4)?,
                created_at: r.get(5)?,
                group_id: r.get(6)?,
            })
        })
        .map_err(dberr)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(dberr)
}

/// Busca un nombre libre para el catálogo: el suyo, o `«nombre» (importado)`,
/// `«nombre» (importado 2)`, etc. `disks.name` es UNIQUE.
fn free_name(conn: &Connection, name: &str) -> Result<String, String> {
    let taken = |candidate: &str| -> Result<bool, String> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM disks WHERE name = ?1)",
            rusqlite::params![candidate],
            |r| r.get(0),
        )
        .map_err(dberr)
    };

    if !taken(name)? {
        return Ok(name.to_string());
    }
    let first = format!("{name} (importado)");
    if !taken(&first)? {
        return Ok(first);
    }
    for n in 2..1000 {
        let candidate = format!("{name} (importado {n})");
        if !taken(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(format!(
        "No se pudo encontrar un nombre libre para el catálogo «{name}»"
    ))
}

/// Posición al final del nivel raíz. Grupos y catálogos sueltos comparten
/// espacio de coordenadas, así que se mira el máximo de los dos (igual que
/// hace `ungroup_all`).
fn next_root_position(conn: &Connection) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM (
           SELECT position FROM groups
           UNION ALL SELECT position FROM disks WHERE group_id IS NULL
         )",
        [],
        |r| r.get(0),
    )
    .map_err(dberr)
}

fn next_group_position(conn: &Connection, group_id: i64) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM disks WHERE group_id = ?1",
        rusqlite::params![group_id],
        |r| r.get(0),
    )
    .map_err(dberr)
}

/// Copia el árbol de un catálogo remapeando los ids: `nodes.parent_id` apunta a
/// otro nodo, así que hay que traducir de los ids del payload a los nuevos.
///
/// Se recorre por profundidad porque `foreign_keys` está activo: el padre tiene
/// que existir antes que el hijo.
fn copy_nodes(conn: &Connection, old_disk: i64, new_disk: i64) -> Result<i64, String> {
    let ids: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "WITH RECURSIVE d(id, depth) AS (
                   SELECT id, 0 FROM src.nodes WHERE disk_id = ?1 AND parent_id IS NULL
                   UNION ALL
                   SELECT n.id, d.depth + 1 FROM src.nodes n JOIN d ON n.parent_id = d.id
                 )
                 SELECT id FROM d ORDER BY depth",
            )
            .map_err(dberr)?;
        let rows = stmt
            .query_map(rusqlite::params![old_disk], |r| r.get(0))
            .map_err(dberr)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(dberr)?
    };

    let mut read = conn
        .prepare(
            "SELECT parent_id, name, is_dir, size, modified, path FROM src.nodes WHERE id = ?1",
        )
        .map_err(dberr)?;
    let mut insert = conn
        .prepare(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id",
        )
        .map_err(dberr)?;
    let mut record = conn
        .prepare("INSERT INTO node_map (old_id, new_id) VALUES (?1, ?2)")
        .map_err(dberr)?;

    // El mapa en memoria evita una consulta a `node_map` por cada nodo.
    let mut map: HashMap<i64, i64> = HashMap::new();
    let mut count = 0i64;

    for old_id in ids {
        let (parent, name, is_dir, size, modified, path) = read
            .query_row(rusqlite::params![old_id], |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(dberr)?;

        let new_parent = match parent {
            None => None,
            Some(p) => Some(*map.get(&p).ok_or_else(|| DANADA.to_string())?),
        };
        let new_id: i64 = insert
            .query_row(
                rusqlite::params![new_disk, new_parent, name, is_dir, size, modified, path],
                |r| r.get(0),
            )
            .map_err(dberr)?;
        record
            .execute(rusqlite::params![old_id, new_id])
            .map_err(dberr)?;
        map.insert(old_id, new_id);
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ensure_schema, MIGRATIONS};
    use rusqlite::params;

    /// Carpeta temporal exclusiva del test, borrada al terminar.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("zorcatalog-test-{tag}"));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn entradas(&self) -> usize {
            std::fs::read_dir(&self.0).unwrap().count()
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// BD en memoria con el esquema real de la app.
    fn bd_vacia() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch(MIGRATIONS).unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    const JPG: &[u8] = b"jpeg-de-mentira";

    /// BD con dos grupos, dos catálogos, un árbol anidado y una miniatura.
    fn bd_con_datos() -> Connection {
        let conn = bd_vacia();
        conn.execute(
            "INSERT INTO groups (name, color, collapsed, position) VALUES ('Fotos', '#4f9cf9', 0, 0)",
            params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO groups (name, color, collapsed, position) VALUES ('Sueltas', NULL, 1, 1)",
            params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO disks (name, root_path, total_size, file_count, group_id, position)
             VALUES ('Discos 2024', 'E:/fotos', 300, 2, 1, 0)",
            params![],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO disks (name, root_path, total_size, file_count, group_id, position)
             VALUES ('Sueltas', 'E:/sueltas', 10, 1, NULL, 2)",
            params![],
        )
        .unwrap();
        // Catálogo 1: raíz -> viajes/ -> playa.jpg
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, NULL, 'fotos', 1, 0, NULL, '')",
            params![],
        )
        .unwrap();
        let root = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'viajes', 1, 0, NULL, 'viajes')",
            params![root],
        )
        .unwrap();
        let viajes = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (1, ?1, 'playa.jpg', 0, 300, NULL, 'viajes/playa.jpg')",
            params![viajes],
        )
        .unwrap();
        let playa = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO thumbs (node_id, data) VALUES (?1, ?2)",
            params![playa, JPG],
        )
        .unwrap();
        // Catálogo 2: raíz -> notas.txt
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (2, NULL, 'sueltas', 1, 0, NULL, '')",
            params![],
        )
        .unwrap();
        let root2 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (2, ?1, 'notas.txt', 0, 10, NULL, 'notas.txt')",
            params![root2],
        )
        .unwrap();
        conn
    }

    fn exportar(conn: &mut Connection, dir: &TmpDir, name: &str) -> PathBuf {
        let dest = dir.file(name);
        export_backup(conn, &dest, dir.path(), &mut Silent).unwrap();
        dest
    }

    #[test]
    fn exportar_e_importar_conserva_todo() {
        let dir = TmpDir::new("roundtrip");
        let mut origen = bd_con_datos();
        let copia = exportar(&mut origen, &dir, "copia.zcbak");

        let mut destino = bd_vacia();
        let res = import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap();

        assert_eq!(res.catalogs_imported, 2);
        assert_eq!(res.groups_created, 2);
        assert_eq!(res.groups_reused, 0);
        assert_eq!(res.nodes, 5);
        assert!(res.catalogs_skipped.is_empty());
        assert!(res.catalogs_renamed.is_empty());

        // Catálogos y su grupo.
        let (name, path, group): (String, String, Option<i64>) = destino
            .query_row(
                "SELECT name, root_path, group_id FROM disks WHERE name = 'Discos 2024'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(name, "Discos 2024");
        assert_eq!(path, "E:/fotos");
        let grupo: String = destino
            .query_row("SELECT name FROM groups WHERE id = ?1", params![group], |r| r.get(0))
            .unwrap();
        assert_eq!(grupo, "Fotos");

        // La jerarquía se remapeó bien: playa.jpg cuelga de viajes, que cuelga de la raíz.
        let (viajes_parent, root_name): (Option<i64>, String) = destino
            .query_row(
                "SELECT n.parent_id, p.name FROM nodes n JOIN nodes p ON p.id = n.parent_id
                 WHERE n.name = 'viajes' AND n.disk_id = (SELECT id FROM disks WHERE name = 'Discos 2024')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(viajes_parent.is_some());
        assert_eq!(root_name, "fotos");

        let playa_parent: String = destino
            .query_row(
                "SELECT p.name FROM nodes n JOIN nodes p ON p.id = n.parent_id
                 WHERE n.name = 'playa.jpg'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(playa_parent, "viajes");

        // La miniatura viajó entera.
        let data: Vec<u8> = destino
            .query_row(
                "SELECT t.data FROM thumbs t JOIN nodes n ON n.id = t.node_id
                 WHERE n.name = 'playa.jpg'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(data, JPG);

        // El trigger de FTS repobló el índice al insertar los nodos.
        let hits: i64 = destino
            .query_row(
                "SELECT COUNT(*) FROM nodes_fts WHERE nodes_fts MATCH 'playa*'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "la búsqueda debe encontrar playa.jpg tras importar");
    }

    #[test]
    fn la_cabecera_y_el_manifiesto_cuentan_lo_mismo() {
        let dir = TmpDir::new("manifiesto");
        let mut origen = bd_con_datos();
        let copia = exportar(&mut origen, &dir, "copia.zcbak");

        let res = export_backup(&mut bd_con_datos(), &dir.file("otra.zcbak"), dir.path(), &mut Silent);
        let res = res.unwrap();
        assert!(res.file_bytes > 0);
        assert!(res.payload_bytes > 0);
        assert_eq!(res.catalogs, 2);
        assert_eq!(res.groups, 2);
        assert_eq!(res.nodes, 5);
        assert_eq!(res.thumbs, 1);

        let mut f = File::open(&copia).unwrap();
        let manifest = read_header(&mut f).unwrap();
        assert_eq!(manifest.catalogs, 2);
        assert_eq!(manifest.groups, 2);
        assert_eq!(manifest.nodes, 5);
        assert_eq!(manifest.thumbs, 1);
        assert_eq!(manifest.payload_bytes, res.payload_bytes);
        assert_eq!(manifest.app_version, env!("CARGO_PKG_VERSION"));
        assert!(!manifest.exported_at.is_empty());

        // Y tras la cabecera viene el flujo gzip.
        let mut gz = [0u8; 2];
        f.read_exact(&mut gz).unwrap();
        assert_eq!(gz, [0x1f, 0x8b], "el payload debe ir comprimido en gzip");
    }

    #[test]
    fn rechaza_un_fichero_ajeno() {
        let dir = TmpDir::new("ajeno");
        let falso = dir.file("falso.zcbak");
        std::fs::write(&falso, b"esto no es una copia de ZorCatalog").unwrap();

        let mut destino = bd_vacia();
        let err = import_backup(&mut destino, &falso, dir.path(), &mut Silent).unwrap_err();
        assert!(err.contains("no es una copia"), "mensaje inesperado: {err}");

        // Y tampoco se ha tocado la BD.
        let n: i64 = destino
            .query_row("SELECT COUNT(*) FROM disks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn rechaza_una_copia_truncada() {
        let dir = TmpDir::new("truncada");
        let mut origen = bd_con_datos();
        let copia = exportar(&mut origen, &dir, "copia.zcbak");

        // Dejamos la cabecera y un trozo de gzip, nada más.
        let bytes = std::fs::read(&copia).unwrap();
        std::fs::write(&copia, &bytes[..bytes.len() - 40]).unwrap();

        let mut destino = bd_vacia();
        let err = import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap_err();
        assert!(
            err.contains("dañada") || err.contains("incompleta"),
            "mensaje inesperado: {err}"
        );
        let n: i64 = destino
            .query_row("SELECT COUNT(*) FROM disks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "una copia rota no debe dejar nada a medias");
    }

    #[test]
    fn importar_dos_veces_omite_los_duplicados() {
        let dir = TmpDir::new("duplicados");
        let mut origen = bd_con_datos();
        let copia = exportar(&mut origen, &dir, "copia.zcbak");

        let mut destino = bd_vacia();
        import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap();
        let res = import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap();

        assert_eq!(res.catalogs_imported, 0);
        assert_eq!(res.catalogs_skipped.len(), 2);
        assert_eq!(res.groups_reused, 2);
        assert_eq!(res.groups_created, 0);

        let n: i64 = destino
            .query_row("SELECT COUNT(*) FROM disks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "no debe haber catálogos repetidos");
    }

    #[test]
    fn renombra_si_el_nombre_choca_con_otra_ruta() {
        let dir = TmpDir::new("renombrado");
        let mut origen = bd_con_datos();
        let copia = exportar(&mut origen, &dir, "copia.zcbak");

        // Ya existe un catálogo con el mismo nombre pero otra ruta.
        let mut destino = bd_vacia();
        destino
            .execute(
                "INSERT INTO disks (name, root_path) VALUES ('Discos 2024', 'X:/otra')",
                params![],
            )
            .unwrap();

        let res = import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap();
        assert_eq!(res.catalogs_imported, 2);
        assert_eq!(res.catalogs_renamed.len(), 1);
        assert_eq!(res.catalogs_renamed[0].from, "Discos 2024");
        assert_eq!(res.catalogs_renamed[0].to, "Discos 2024 (importado)");

        let n: i64 = destino
            .query_row(
                "SELECT COUNT(*) FROM disks WHERE name = 'Discos 2024 (importado)' AND root_path = 'E:/fotos'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn deja_los_catalogos_sueltos_al_final_del_nivel_raiz() {
        let dir = TmpDir::new("posiciones");
        let mut origen = bd_con_datos();
        let copia = exportar(&mut origen, &dir, "copia.zcbak");

        let mut destino = bd_vacia();
        import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap();

        // El grupo 'Sueltas' (posición 1) va antes que el catálogo suelto 'Sueltas'
        // (que debe quedar al final, en la posición 2).
        let (grupo, suelto): (i64, i64) = destino
            .query_row(
                "SELECT (SELECT position FROM groups WHERE name = 'Sueltas'),
                        (SELECT position FROM disks WHERE name = 'Sueltas')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(suelto > grupo, "grupo={grupo} suelto={suelto}");
    }

    #[test]
    fn anade_la_extension_si_falta() {
        assert_eq!(with_ext(Path::new("/x/copia")), PathBuf::from("/x/copia.zcbak"));
        assert_eq!(with_ext(Path::new("/x/copia.zcbak")), PathBuf::from("/x/copia.zcbak"));
        assert_eq!(with_ext(Path::new("/x/COPIA.ZCBAK")), PathBuf::from("/x/COPIA.ZCBAK"));
        // No se sustituye una extensión que ya exista: se añade al final.
        assert_eq!(with_ext(Path::new("/x/mi.copia")), PathBuf::from("/x/mi.copia.zcbak"));
    }

    #[test]
    fn limpia_los_temporales_aunque_falle() {
        let dir = TmpDir::new("temporales");
        let falso = dir.file("falso.zcbak");
        std::fs::write(&falso, b"nada").unwrap();

        let mut destino = bd_vacia();
        let _ = import_backup(&mut destino, &falso, dir.path(), &mut Silent).unwrap_err();
        assert_eq!(dir.entradas(), 1, "solo debe quedar el fichero falso");

        // Y en una exportación correcta tampoco queda basura.
        let mut origen = bd_con_datos();
        let _ = exportar(&mut origen, &dir, "copia.zcbak");
        assert_eq!(dir.entradas(), 2, "no deben quedar temporales");
    }

    /// Prueba de humo contra la base de datos real de la app. Es opt-in porque
    /// depende de datos que solo existen en la máquina del usuario:
    ///
    /// ```text
    /// ZORCATALOG_DB=~/.local/share/com.zordev.zorcatalog/zorcatalog.db \
    ///   cargo test humo_contra_la_bd_real -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "necesita ZORCATALOG_DB apuntando a una BD real"]
    fn humo_contra_la_bd_real() {
        let ruta = std::env::var("ZORCATALOG_DB")
            .expect("define ZORCATALOG_DB con la ruta de la BD real");
        let dir = TmpDir::new("humo");

        // Se trabaja sobre una copia (con su `-wal`, si la app está abierta)
        // para no tocar la BD real. Además, `ATTACH` hereda el modo de la
        // conexión, así que abrirla en solo lectura impediría crear el payload.
        let copia_db = dir.file("real.db");
        for sufijo in ["", "-wal", "-shm"] {
            let origen = PathBuf::from(format!("{ruta}{sufijo}"));
            if origen.exists() {
                std::fs::copy(&origen, dir.file(&format!("real.db{sufijo}"))).unwrap();
            }
        }
        let mut real = Connection::open(&copia_db).unwrap();

        let copia = dir.file("real.zcbak");
        let res = export_backup(&mut real, &copia, dir.path(), &mut Silent).unwrap();
        println!(
            "exportado: {} catálogos, {} grupos, {} nodos, {} miniaturas · {} -> {} bytes",
            res.catalogs, res.groups, res.nodes, res.thumbs, res.payload_bytes, res.file_bytes
        );
        assert!(res.catalogs > 0, "la BD real no tiene catálogos");

        let mut destino = bd_vacia();
        let imp = import_backup(&mut destino, &copia, dir.path(), &mut Silent).unwrap();
        assert_eq!(imp.catalogs_imported, res.catalogs);
        assert_eq!(imp.nodes, res.nodes);

        // Cada catálogo tiene su nodo raíz, así que el árbol queda navegable.
        let raices: i64 = destino
            .query_row("SELECT COUNT(*) FROM nodes WHERE parent_id IS NULL", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(raices, res.catalogs);
    }
}
