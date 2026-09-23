import { invoke } from "@tauri-apps/api/core";
import type {
  CatalogGroup,
  DiskMeta,
  DiskStats,
  ExportSummary,
  ImportSummary,
  Node,
  SearchHit,
  SidebarLayoutEntry,
} from "./types";

/**
 * `pnpm dev:web` sirve el frontend en un navegador normal, donde no existe el
 * puente con Rust (`window.__TAURI_INTERNALS__`). Sin este control, cada
 * comando reventaba con "Cannot read properties of undefined (reading
 * 'invoke')"; así al menos el aviso explica qué pasa.
 */
export function isDesktop() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

const SIN_PUENTE =
  "Sin conexión con el backend: esta vista solo funciona dentro de la app de " +
  "escritorio (arráncala con `pnpm dev`, no abras el frontend en el navegador).";

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktop()) return Promise.reject(new Error(SIN_PUENTE));
  return invoke<T>(cmd, args);
}

export const api = {
  listCatalogs: () => call<DiskMeta[]>("list_catalogs"),
  createCatalog: (name: string, path: string, groupId: number | null) =>
    call<DiskMeta>("create_catalog", { name, path, groupId }),
  deleteCatalog: (id: number) => call<void>("delete_catalog", { id }),
  listGroups: () => call<CatalogGroup[]>("list_groups"),
  createGroup: (name: string, color: string | null) =>
    call<CatalogGroup>("create_group", { name, color }),
  updateGroup: (
    id: number,
    name: string,
    color: string | null,
    collapsed: boolean,
  ) => call<CatalogGroup>("update_group", { id, name, color, collapsed }),
  deleteGroup: (id: number) => call<void>("delete_group", { id }),
  /** Reescribe el orden completo del panel lateral tras un drag & drop. */
  setSidebarLayout: (entries: SidebarLayoutEntry[]) =>
    call<void>("set_sidebar_layout", { entries }),
  getChildren: (diskId: number, parentId: number | null) =>
    call<Node[]>("get_children", { diskId, parentId }),
  getAncestors: (nodeId: number) => call<Node[]>("get_ancestors", { nodeId }),
  search: (
    query: string,
    diskId: number | null,
    groupId: number | null,
    limit: number | null,
  ) => call<SearchHit[]>("search", { query, diskId, groupId, limit }),
  getStats: (diskId: number) => call<DiskStats>("get_stats", { diskId }),
  cancelScan: () => call<void>("cancel_scan"),
  isScanning: () => call<boolean>("is_scanning"),
  /** Escribe un paquete `.zcbak` comprimido con todos los catálogos. */
  exportBackup: (path: string) =>
    call<ExportSummary>("export_backup", { path }),
  /** Fusiona un paquete `.zcbak` en la base de datos actual (no borra nada). */
  importBackup: (path: string) =>
    call<ImportSummary>("import_backup", { path }),
};

/** Miniatura vía protocolo custom: sirve el BLOB sin pasar por JSON. */
export function thumbSrc(nodeId: number): string {
  // En Windows Tauri lo expone como http://thumb.localhost/{id}
  return `http://thumb.localhost/${nodeId}`;
}
