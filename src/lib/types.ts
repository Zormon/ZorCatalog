export interface DiskMeta {
  id: number;
  name: string;
  rootPath: string;
  totalSize: number;
  fileCount: number;
  createdAt: string;
  /** Grupo al que pertenece (`null` = nivel raíz). */
  groupId: number | null;
  /** Posición dentro de su contenedor (su grupo, o el nivel raíz). */
  position: number;
}

/** Grupo de catálogos. Solo hay un nivel: los grupos no anidan grupos. */
export interface CatalogGroup {
  id: number;
  name: string;
  color: string | null;
  collapsed: boolean;
  /** Posición entre los elementos de nivel raíz (comparte espacio con
   *  `DiskMeta.position` de los catálogos sin grupo). */
  position: number;
}

/** Un elemento del panel lateral tal y como queda tras un drag & drop. */
export type SidebarLayoutEntry =
  | { kind: "group"; id: number; catalogs: number[] }
  | { kind: "catalog"; id: number };

/** Campos de un grupo que la interfaz puede modificar. */
export type GroupPatch = {
  name?: string;
  color?: string | null;
  collapsed?: boolean;
};

/** Paleta de colores disponible para los grupos. */
export const GROUP_COLORS = [
  "#4f9cf9",
  "#3fb950",
  "#d29922",
  "#e5534b",
  "#a371f7",
  "#2dd4bf",
] as const;

export interface Node {
  id: number;
  diskId: number;
  parentId: number | null;
  name: string;
  isDir: boolean;
  size: number | null;
  modified: string | null;
  path: string;
}

export interface DiskStats {
  fileCount: number;
  dirCount: number;
  totalSize: number;
  thumbCount: number;
}

export interface ScanProgress {
  diskId: number;
  processed: number;
  current: string;
  done: boolean;
}

export interface SearchHit {
  node: Node;
  diskName: string;
}

/**
 * Avance de una copia (evento `backup-progress`). La unidad de `done`/`total`
 * depende de `stage`: en `db` y `merge` son catálogos; en `compress` y
 * `decompress`, bytes.
 */
export interface BackupProgress {
  phase: "export" | "import";
  stage: "db" | "compress" | "decompress" | "merge";
  done: number;
  total: number;
  current: string;
}

/** Lo que quedó dentro del fichero `.zcbak`. */
export interface ExportSummary {
  catalogs: number;
  groups: number;
  nodes: number;
  thumbs: number;
  /** Tamaño del payload SQLite sin comprimir. */
  payloadBytes: number;
  /** Tamaño del `.zcbak` ya comprimido. */
  fileBytes: number;
  /** Ruta final del fichero (con la extensión `.zcbak` ya asegurada). */
  filePath: string;
}

/** Catálogo al que hubo que cambiar el nombre por chocar con uno existente. */
export interface RenamedCatalog {
  from: string;
  to: string;
}

export interface ImportSummary {
  catalogsImported: number;
  /** Catálogos omitidos por ser duplicados exactos (mismo nombre y ruta). */
  catalogsSkipped: string[];
  catalogsRenamed: RenamedCatalog[];
  groupsCreated: number;
  /** Grupos que ya existían y se han reutilizado tal cual. */
  groupsReused: number;
  nodes: number;
}

/** Resultado de la última copia, para el diálogo de resumen. */
export type BackupResult =
  | { kind: "export"; data: ExportSummary }
  | { kind: "import"; data: ImportSummary };
