use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use chrono::DateTime;
use rayon::prelude::*;
use tauri::Emitter;
use walkdir::WalkDir;

use crate::db::Db;
use crate::models::ScanProgress;

/// Estado del escaneo gestionado por Tauri.
pub struct ScanState {
    pub active: Arc<AtomicBool>,
    pub cancel: Arc<AtomicBool>,
}

impl ScanState {
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

const IMAGE_EXTS: [&str; 8] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"];
const THUMB_MAX_BYTES: u64 = 50 * 1024 * 1024;
const THUMB_SIZE: u32 = 256;
const BATCH: usize = 5000;

#[derive(Clone)]
struct Entry {
    /// Ruta absoluta del elemento (para generar la miniatura).
    abs: PathBuf,
    /// Ruta relativa a la raíz, con separador '/'.
    rel: String,
    name: String,
    is_dir: bool,
    size: u64,
    modified: Option<String>,
    thumb: Option<Vec<u8>>,
}

fn to_iso(t: std::time::SystemTime) -> Option<String> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let dt = DateTime::from_timestamp(secs, 0)?;
    Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn collect_entries(root: &Path, cancel: &AtomicBool) -> Result<Vec<Entry>, String> {
    // walkdir incluye archivos ocultos por defecto.
    let mut entries = Vec::new();
    for result in WalkDir::new(root) {
        if cancel.load(Ordering::Relaxed) {
            return Err("escaneo cancelado".into());
        }
        let entry = match result {
            Ok(e) => e,
            Err(err) => {
                eprintln!("zorcatalog: entrando en error de escaneo: {err}");
                continue;
            }
        };
        let ft = entry.file_type();
        if ft.is_symlink() || (!ft.is_dir() && !ft.is_file()) {
            continue;
        }
        let abs = entry.into_path();
        let rel_os = match abs.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if rel_os.as_os_str().is_empty() {
            // La raíz se crea aparte como nodo padre NULL.
            continue;
        }
        let rel = rel_os.to_string_lossy().replace('\\', "/");
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
        let (size, modified) = match std::fs::metadata(&abs) {
            Ok(m) => (
                if ft.is_file() { m.len() } else { 0 },
                m.modified().ok(),
            ),
            Err(_) => (0, None),
        };
        entries.push(Entry {
            abs,
            rel,
            name,
            is_dir: ft.is_dir(),
            size,
            modified: modified.and_then(to_iso),
            thumb: None,
        });
    }
    Ok(entries)
}

fn generate_thumb(path: &Path, size: u64) -> Option<Vec<u8>> {
    if size == 0 || size > THUMB_MAX_BYTES {
        return None;
    }
    let ext = path
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.decode().ok()?;
    let thumb = img.resize(THUMB_SIZE, THUMB_SIZE, image::imageops::FilterType::Lanczos3);
    let mut buf = std::io::Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Jpeg).ok()?;
    Some(buf.into_inner())
}

/// Escanea `root`, genera miniaturas y escribe todo en la BD por lotes.
/// Devuelve el número de archivos y el tamaño total (solo archivos).
pub fn scan_into_db(
    db: &Db,
    disk_id: i64,
    root: &Path,
    cancel: Arc<AtomicBool>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let mut entries = collect_entries(root, &cancel)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("escaneo cancelado".into());
    }

    // Miniaturas en paralelo (decodificación de imágenes es lo más lento).
    entries.par_iter_mut().for_each(|e| {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        e.thumb = generate_thumb(&e.abs, e.size);
    });
    if cancel.load(Ordering::Relaxed) {
        return Err("escaneo cancelado".into());
    }

    // Ordenar por profundidad garantiza que cada carpeta se inserta
    // antes que sus hijos (el padre debe existir para asignar parent_id).
    entries.sort_by_key(|e| e.rel.matches('/').count() as u32);

    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    let mut dir_ids: HashMap<String, i64> = HashMap::new();
    let mut processed: i64 = 0;
    let mut file_count: i64 = 0;
    let mut total_size: i64 = 0;
    let mut last_rel = String::new();

    {
        let conn = db
            .0
            .lock()
            .map_err(|_| "no se pudo bloquear la base de datos".to_string())?;
        conn.execute(
            "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
             VALUES (?1, NULL, ?2, 1, 0, NULL, '')",
            rusqlite::params![disk_id, root_name],
        )
        .map_err(|e| e.to_string())?;
        dir_ids.insert(String::new(), conn.last_insert_rowid());
    }

    for chunk in entries.chunks(BATCH) {
        if cancel.load(Ordering::Relaxed) {
            return Err("escaneo cancelado".into());
        }
        let mut pending_thumbs: Vec<(i64, Vec<u8>)> = Vec::new();
        {
            let conn = db
                .0
                .lock()
                .map_err(|_| "no se pudo bloquear la base de datos".to_string())?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO nodes (disk_id, parent_id, name, is_dir, size, modified, path)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         RETURNING id",
                    )
                    .map_err(|e| e.to_string())?;
                for e in chunk {
                    let parent_rel = match e.rel.rsplit_once('/') {
                        Some((p, _)) => p.to_string(),
                        None => String::new(),
                    };
                    let parent_id = match dir_ids.get(&parent_rel) {
                        Some(id) => *id,
                        None => continue,
                    };
                    let id: i64 = stmt
                        .query_row(
                            rusqlite::params![
                                disk_id,
                                parent_id,
                                e.name,
                                e.is_dir as i64,
                                e.size as i64,
                                e.modified,
                                e.rel
                            ],
                            |r| r.get(0),
                        )
                        .map_err(|err| format!("{err} ({})", e.rel))?;
                    if e.is_dir {
                        dir_ids.insert(e.rel.clone(), id);
                    } else {
                        file_count += 1;
                        total_size += e.size as i64;
                    }
                    if let Some(t) = &e.thumb {
                        pending_thumbs.push((id, t.clone()));
                    }
                    last_rel = e.rel.clone();
                }
            }
            if !pending_thumbs.is_empty() {
                let mut ts = tx
                    .prepare("INSERT INTO thumbs (node_id, data) VALUES (?1, ?2)")
                    .map_err(|e| e.to_string())?;
                for (id, blob) in &pending_thumbs {
                    ts.execute(rusqlite::params![id, blob])
                        .map_err(|e| e.to_string())?;
                }
            }
            tx.commit().map_err(|e| e.to_string())?;
        }

        processed += chunk.len() as i64;
        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                disk_id,
                processed,
                current: last_rel.clone(),
                done: false,
            },
        );
    }

    {
        let conn = db
            .0
            .lock()
            .map_err(|_| "no se pudo bloquear la base de datos".to_string())?;
        conn.execute(
            "UPDATE disks SET total_size = ?1, file_count = ?2 WHERE id = ?3",
            rusqlite::params![total_size, file_count, disk_id],
        )
        .map_err(|e| e.to_string())?;
    }

    let _ = app.emit(
        "scan-progress",
        ScanProgress {
            disk_id,
            processed,
            current: String::new(),
            done: true,
        },
    );
    Ok(())
}
