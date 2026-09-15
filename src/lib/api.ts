import { invoke } from "@tauri-apps/api/core";
import type { DiskMeta, DiskStats, Node, SearchHit } from "./types";

export const api = {
  listCatalogs: () => invoke<DiskMeta[]>("list_catalogs"),
  createCatalog: (name: string, path: string) =>
    invoke<DiskMeta>("create_catalog", { name, path }),
  deleteCatalog: (id: number) => invoke<void>("delete_catalog", { id }),
  getChildren: (diskId: number, parentId: number | null) =>
    invoke<Node[]>("get_children", { diskId, parentId }),
  getAncestors: (nodeId: number) => invoke<Node[]>("get_ancestors", { nodeId }),
  search: (query: string, diskId: number | null, limit: number | null) =>
    invoke<SearchHit[]>("search", { query, diskId, limit }),
  getStats: (diskId: number) => invoke<DiskStats>("get_stats", { diskId }),
  cancelScan: () => invoke<void>("cancel_scan"),
  isScanning: () => invoke<boolean>("is_scanning"),
};

/** Miniatura vía protocolo custom: sirve el BLOB sin pasar por JSON. */
export function thumbSrc(nodeId: number): string {
  // En Windows Tauri lo expone como http://thumb.localhost/{id}
  return `http://thumb.localhost/${nodeId}`;
}
