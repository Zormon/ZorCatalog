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
