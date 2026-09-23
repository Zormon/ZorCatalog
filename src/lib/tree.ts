import type { CatalogGroup, DiskMeta } from "./types";

export interface GroupNode {
  group: CatalogGroup;
  /** Catálogos del grupo, ya ordenados por `position`. */
  catalogs: DiskMeta[];
}

/** Orden natural de los elementos del panel: por posición y, si empatan, por nombre. */
export const byPosition = (
  a: { position: number; name: string },
  b: { position: number; name: string },
) => a.position - b.position || a.name.localeCompare(b.name);

/**
 * Reparte los catálogos en grupos. Los que apuntan a un grupo que no existe
 * (no debería pasar: hay clave foránea) se tratan como sueltos en lugar de
 * desaparecer de la interfaz.
 */
export function bucketByGroup(
  groups: CatalogGroup[],
  catalogs: DiskMeta[],
): { nodes: GroupNode[]; loose: DiskMeta[] } {
  const ids = new Set(groups.map((g) => g.id));
  const byGroup = new Map<number, DiskMeta[]>();
  const loose: DiskMeta[] = [];

  for (const c of catalogs) {
    if (c.groupId != null && ids.has(c.groupId)) {
      const list = byGroup.get(c.groupId);
      if (list) list.push(c);
      else byGroup.set(c.groupId, [c]);
    } else {
      loose.push(c);
    }
  }

  const nodes: GroupNode[] = groups.map((group) => ({
    group,
    catalogs: [...(byGroup.get(group.id) ?? [])].sort(byPosition),
  }));

  return { nodes, loose };
}
