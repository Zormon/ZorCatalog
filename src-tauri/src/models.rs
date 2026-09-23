use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskMeta {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub total_size: i64,
    pub file_count: i64,
    pub created_at: String,
    /// Grupo al que pertenece el catálogo (`null` = nivel raíz).
    pub group_id: Option<i64>,
    /// Posición dentro de su contenedor (su grupo, o el nivel raíz).
    pub position: i64,
}

/// Grupo de catálogos. Solo hay un nivel: los grupos no anidan otros grupos.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub collapsed: bool,
    /// Posición entre los elementos de nivel raíz (comparte espacio con
    /// `DiskMeta::position` de los catálogos sin grupo).
    pub position: i64,
}

/// Un elemento del panel lateral, tal y como queda tras un drag & drop.
/// El frontend manda la lista completa y el backend reescribe el orden.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SidebarEntry {
    /// Grupo con sus catálogos ordenados.
    Group { id: i64, catalogs: Vec<i64> },
    /// Catálogo en el nivel raíz (sin grupo).
    Catalog { id: i64 },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: i64,
    pub disk_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<i64>,
    pub modified: Option<String>,
    pub path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiskStats {
    pub file_count: i64,
    pub dir_count: i64,
    pub total_size: i64,
    pub thumb_count: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub disk_id: i64,
    pub processed: i64,
    pub current: String,
    pub done: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub node: Node,
    pub disk_name: String,
}

/// Lo que quedó dentro del fichero `.zcbak` (para enseñar el resumen).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub catalogs: i64,
    pub groups: i64,
    pub nodes: i64,
    pub thumbs: i64,
    /// Tamaño del payload SQLite sin comprimir.
    pub payload_bytes: u64,
    /// Tamaño del `.zcbak` ya comprimido.
    pub file_bytes: u64,
    /// Ruta final del fichero (con la extensión `.zcbak` ya asegurada).
    pub file_path: String,
}

/// Catálogo al que hubo que cambiar el nombre por chocar con uno existente.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RenamedCatalog {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub catalogs_imported: i64,
    /// Catálogos omitidos por ser duplicados exactos (mismo nombre y ruta).
    pub catalogs_skipped: Vec<String>,
    pub catalogs_renamed: Vec<RenamedCatalog>,
    pub groups_created: i64,
    /// Grupos que ya existían y se han reutilizado tal cual.
    pub groups_reused: i64,
    pub nodes: i64,
}

/// Avance de una copia, emitido por el evento `backup-progress`.
///
/// La unidad de `done`/`total` depende de `stage`: en `db` y `merge` son
/// catálogos; en `compress` y `decompress`, bytes.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupProgress {
    pub phase: String,
    pub stage: String,
    pub done: i64,
    pub total: i64,
    pub current: String,
}
