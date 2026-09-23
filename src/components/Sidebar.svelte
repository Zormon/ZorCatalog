<script lang="ts">
  import type {
    CatalogGroup,
    DiskMeta,
    GroupPatch,
    SidebarLayoutEntry,
  } from "../lib/types";
  import { GROUP_COLORS } from "../lib/types";
  import { bucketByGroup } from "../lib/tree";
  import { fmtCount, fmtSize } from "../lib/format";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let {
    groups,
    catalogs,
    selected,
    onSelect,
    onNewCatalog,
    onNewGroup,
    onDeleteCatalog,
    onDeleteGroup,
    onUpdateGroup,
    onLayoutChange,
  }: {
    groups: CatalogGroup[];
    catalogs: DiskMeta[];
    selected: DiskMeta | null;
    onSelect: (d: DiskMeta) => void;
    onNewCatalog: () => void;
    onNewGroup: () => void;
    onDeleteCatalog: (id: number) => void;
    onDeleteGroup: (id: number) => void;
    onUpdateGroup: (id: number, patch: GroupPatch) => void;
    onLayoutChange: (entries: SidebarLayoutEntry[]) => void;
  } = $props();

  /** Copia mutable del layout, la que se retoca al soltar un arrastre. */
  interface GroupEntry {
    kind: "group";
    id: number;
    catalogs: number[];
  }
  interface CatalogEntry {
    kind: "catalog";
    id: number;
  }
  type MutableEntry = GroupEntry | CatalogEntry;

  type DragItem = { kind: "catalog" | "group"; id: number };

  type DropTarget =
    | { type: "catalog"; id: number; where: "before" | "after" }
    | { type: "group-into"; id: number }
    | { type: "group-head"; id: number; where: "before" | "after" }
    | { type: "root" };

  /** Borrado a la espera de confirmación: la ✕ abre el diálogo, nunca borra directo. */
  type PendingDelete =
    | { kind: "catalog"; id: number; name: string; fileCount: number; rootPath: string }
    | { kind: "group"; id: number; name: string; count: number };

  let drag = $state<DragItem | null>(null);
  let dropTarget = $state<DropTarget | null>(null);
  let renamingId = $state<number | null>(null);
  let renameValue = $state("");
  let renameInput = $state<HTMLInputElement | null>(null);
  let colorPickerId = $state<number | null>(null);
  let pendingDelete = $state<PendingDelete | null>(null);
  let expandTimer: ReturnType<typeof setTimeout> | null = null;
  let expandPendingId: number | null = null;

  /**
   * Lista de nivel raíz: grupos y catálogos sin grupo mezclados. Grupos y
   * catálogos sueltos comparten su `position` en un mismo espacio de
   * coordenadas (lo garantiza el backend, así que no hay empates).
   */
  const topItems = $derived.by(() => {
    const { nodes, loose } = bucketByGroup(groups, catalogs);

    type Item =
      | { kind: "group"; key: string; group: CatalogGroup; children: DiskMeta[] }
      | { kind: "catalog"; key: string; catalog: DiskMeta };

    const items: Item[] = [];
    for (const n of nodes) {
      items.push({
        kind: "group",
        key: `g${n.group.id}`,
        group: n.group,
        children: n.catalogs,
      });
    }
    for (const c of loose) {
      items.push({ kind: "catalog", key: `c${c.id}`, catalog: c });
    }
    items.sort((a, b) => {
      const pa = a.kind === "group" ? a.group.position : a.catalog.position;
      const pb = b.kind === "group" ? b.group.position : b.catalog.position;
      if (pa !== pb) return pa - pb;
      const na = a.kind === "group" ? a.group.name : a.catalog.name;
      const nb = b.kind === "group" ? b.group.name : b.catalog.name;
      return na.localeCompare(nb);
    });
    return items;
  });

  // --- Renombrado y selector de color ------------------------------------

  $effect(() => {
    if (renamingId == null) return;
    renameInput?.focus();
    renameInput?.select();
  });

  // Cerrar el selector de color al hacer clic fuera. El setTimeout evita que el
  // propio clic que lo abre lo cierre en el mismo tick.
  $effect(() => {
    if (colorPickerId == null) return;
    const onDoc = (e: MouseEvent) => {
      const t = e.target as HTMLElement | null;
      if (!t?.closest?.(".swatches") && !t?.closest?.(".color-btn")) {
        colorPickerId = null;
      }
    };
    const id = setTimeout(() => document.addEventListener("click", onDoc), 0);
    return () => {
      clearTimeout(id);
      document.removeEventListener("click", onDoc);
    };
  });

  function startRename(g: CatalogGroup) {
    renamingId = g.id;
    renameValue = g.name;
  }

  function commitRename(g: CatalogGroup) {
    if (renamingId !== g.id) return;
    renamingId = null;
    const name = renameValue.trim();
    if (!name || name === g.name) return;
    onUpdateGroup(g.id, { name });
  }

  function onRenameKey(e: KeyboardEvent, g: CatalogGroup) {
    if (e.key === "Enter") commitRename(g);
    else if (e.key === "Escape") renamingId = null;
  }

  function removeCatalog(d: DiskMeta) {
    pendingDelete = {
      kind: "catalog",
      id: d.id,
      name: d.name,
      fileCount: d.fileCount,
      rootPath: d.rootPath,
    };
  }

  function removeGroup(g: CatalogGroup, count: number) {
    pendingDelete = { kind: "group", id: g.id, name: g.name, count };
  }

  /** Textos del diálogo según lo que se vaya a borrar. */
  const confirmText = $derived.by(() => {
    const p = pendingDelete;
    if (!p) return { title: "", message: "", details: [] as string[], label: "" };
    if (p.kind === "catalog") {
      return {
        title: "Eliminar catálogo",
        message: `¿Seguro que quieres eliminar el catálogo «${p.name}»?`,
        details: [
          `${fmtCount(p.fileCount)} archivos indexados · ${p.rootPath}`,
          "Solo se borra el índice: los archivos del disco no se tocan.",
        ],
        label: "Eliminar catálogo",
      };
    }
    return {
      title: "Eliminar grupo",
      message: `¿Seguro que quieres eliminar el grupo «${p.name}»?`,
      details: p.count ? [`Sus ${p.count} catálogo(s) no se borrarán: quedarán sin grupo.`] : [],
      label: "Eliminar grupo",
    };
  });

  function confirmDelete() {
    const p = pendingDelete;
    pendingDelete = null;
    if (!p) return;
    if (p.kind === "catalog") onDeleteCatalog(p.id);
    else onDeleteGroup(p.id);
  }

  // --- Auto-expandir un grupo colapsado al arrastrar encima ----------------

  function cancelAutoExpand() {
    if (expandTimer) clearTimeout(expandTimer);
    expandTimer = null;
    expandPendingId = null;
  }

  function autoExpand(groupId: number) {
    const g = groups.find((x) => x.id === groupId);
    if (!g?.collapsed) {
      cancelAutoExpand();
      return;
    }
    if (expandPendingId === groupId) return;
    cancelAutoExpand();
    expandPendingId = groupId;
    expandTimer = setTimeout(() => {
      expandTimer = null;
      expandPendingId = null;
      onUpdateGroup(groupId, { collapsed: false });
    }, 600);
  }

  // --- Drag & drop --------------------------------------------------------

  function startDrag(e: DragEvent, item: DragItem) {
    drag = item;
    // Firefox no inicia el arrastre sin datos en el dataTransfer.
    e.dataTransfer?.setData("text/plain", `${item.kind}:${item.id}`);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }

  function resetDrag() {
    drag = null;
    dropTarget = null;
    cancelAutoExpand();
  }

  function midpoint(e: DragEvent) {
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    return e.clientY > r.top + r.height / 2 ? "after" : "before";
  }

  function overCatalog(e: DragEvent, c: DiskMeta) {
    if (drag?.kind !== "catalog") return;
    // Sin esto, el `dragover` del contenedor del grupo burbujearía después y
    // pisaría la posición `before`/`after` que acabamos de calcular.
    e.stopPropagation();
    if (drag.id === c.id) {
      dropTarget = null;
      return;
    }
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dropTarget = { type: "catalog", id: c.id, where: midpoint(e) };
    if (c.groupId != null) autoExpand(c.groupId);
    else cancelAutoExpand();
  }

  function overGroupHead(e: DragEvent, g: CatalogGroup) {
    if (!drag) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    if (drag.kind === "catalog") {
      dropTarget = { type: "group-into", id: g.id };
      autoExpand(g.id);
    } else if (drag.id !== g.id) {
      // Los grupos solo se reordenan entre sí.
      cancelAutoExpand();
      dropTarget = { type: "group-head", id: g.id, where: midpoint(e) };
    } else {
      dropTarget = null;
    }
  }

  function overGroupBody(e: DragEvent, g: CatalogGroup) {
    if (drag?.kind !== "catalog") return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dropTarget = { type: "group-into", id: g.id };
  }

  function overRoot(e: DragEvent) {
    if (drag?.kind !== "catalog") return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dropTarget = { type: "root" };
  }

  /** Instantánea del orden actual, ya en el formato que espera el backend. */
  function currentLayout(): MutableEntry[] {
    return topItems.map((it) =>
      it.kind === "group"
        ? { kind: "group", id: it.group.id, catalogs: it.children.map((c) => c.id) }
        : { kind: "catalog", id: it.catalog.id },
    );
  }

  function locate(
    entries: MutableEntry[],
    id: number,
  ): { groupIdx: number | null; index: number } | null {
    for (let i = 0; i < entries.length; i++) {
      const en = entries[i];
      if (en.kind === "catalog" && en.id === id) return { groupIdx: null, index: i };
      if (en.kind === "group") {
        const j = en.catalogs.indexOf(id);
        if (j >= 0) return { groupIdx: i, index: j };
      }
    }
    return null;
  }

  function removeId(entries: MutableEntry[], id: number) {
    const loc = locate(entries, id);
    if (!loc) return;
    if (loc.groupIdx === null) entries.splice(loc.index, 1);
    else (entries[loc.groupIdx] as GroupEntry).catalogs.splice(loc.index, 1);
  }

  function insertCatalog(entries: MutableEntry[], id: number, target: DropTarget) {
    if (target.type === "group-into") {
      const g = entries.find((en) => en.kind === "group" && en.id === target.id);
      if (g?.kind === "group") {
        g.catalogs.push(id);
        return;
      }
    } else if (target.type === "catalog") {
      const loc = locate(entries, target.id);
      if (loc) {
        const at = loc.index + (target.where === "after" ? 1 : 0);
        if (loc.groupIdx === null) entries.splice(at, 0, { kind: "catalog", id });
        else (entries[loc.groupIdx] as GroupEntry).catalogs.splice(at, 0, id);
        return;
      }
    }
    // Destino "root" (o destino desaparecido): al final, sin grupo.
    entries.push({ kind: "catalog", id });
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    const src = drag;
    const target = dropTarget;
    resetDrag();
    if (!src || !target) return;

    const entries = currentLayout();

    if (src.kind === "group") {
      if (target.type !== "group-head" || target.id === src.id) return;
      const from = entries.findIndex((en) => en.kind === "group" && en.id === src.id);
      if (from < 0) return;
      const [moved] = entries.splice(from, 1);
      let to = entries.findIndex((en) => en.kind === "group" && en.id === target.id);
      if (to < 0) {
        entries.push(moved);
      } else {
        if (target.where === "after") to += 1;
        entries.splice(to, 0, moved);
      }
      onLayoutChange(entries);
      return;
    }

    // Catálogo: nunca soltarse sobre sí mismo (el índice quedaría inválido).
    if (target.type === "catalog" && target.id === src.id) return;
    // Soltar un catálogo sobre su propio encabezado de grupo: ya está dentro.
    if (target.type === "group-head") return;

    removeId(entries, src.id);
    insertCatalog(entries, src.id, target);
    onLayoutChange(entries);
  }
</script>

{#snippet catalogRow(c: DiskMeta, nested: boolean)}
  <div
    class="catalog-item"
    class:nested
    class:active={selected?.id === c.id}
    class:dragging={drag?.kind === "catalog" && drag.id === c.id}
    class:drop-before={dropTarget?.type === "catalog" &&
      dropTarget.id === c.id &&
      dropTarget.where === "before"}
    class:drop-after={dropTarget?.type === "catalog" &&
      dropTarget.id === c.id &&
      dropTarget.where === "after"}
    role="button"
    tabindex="0"
    draggable="true"
    title="Arrastra para reordenar o mover de grupo"
    ondragstart={(e) => startDrag(e, { kind: "catalog", id: c.id })}
    ondragend={resetDrag}
    ondragover={(e) => overCatalog(e, c)}
    ondrop={onDrop}
    onclick={() => onSelect(c)}
    onkeydown={(e) => e.key === "Enter" && onSelect(c)}
  >
    <div class="catalog-info">
      <div class="catalog-name">{c.name}</div>
      <div class="catalog-meta">
        {fmtCount(c.fileCount)} archivos · {fmtSize(c.totalSize)}
      </div>
    </div>
    <button
      class="icon-btn"
      type="button"
      title="Eliminar catálogo"
      onclick={(e) => {
        e.stopPropagation();
        removeCatalog(c);
      }}
    >
      ✕
    </button>
  </div>
{/snippet}

<aside class="sidebar">
  <div class="sidebar-head">
    <span class="logo">ZorCatalog</span>
    <div class="head-actions">
      <button class="btn" type="button" onclick={onNewGroup}>+ Grupo</button>
      <button class="btn primary" type="button" onclick={onNewCatalog}>+ Nuevo</button>
    </div>
  </div>

  <nav ondrop={onDrop} ondragover={(e) => e.preventDefault()}>
    {#each topItems as item (item.key)}
      {#if item.kind === "catalog"}
        {@render catalogRow(item.catalog, false)}
      {:else}
        <div
          class="group"
          class:into={dropTarget?.type === "group-into" && dropTarget.id === item.group.id}
          class:collapsed={item.group.collapsed}
        >
          <div
            class="group-head"
            role="group"
            aria-label="Grupo {item.group.name}"
            class:dragging={drag?.kind === "group" && drag.id === item.group.id}
            class:drop-before={dropTarget?.type === "group-head" &&
              dropTarget.id === item.group.id &&
              dropTarget.where === "before"}
            class:drop-after={dropTarget?.type === "group-head" &&
              dropTarget.id === item.group.id &&
              dropTarget.where === "after"}
            draggable={renamingId !== item.group.id}
            ondragstart={(e) => startDrag(e, { kind: "group", id: item.group.id })}
            ondragend={resetDrag}
            ondragover={(e) => overGroupHead(e, item.group)}
            ondrop={onDrop}
          >
            <button
              class="chevron"
              type="button"
              title={item.group.collapsed ? "Expandir grupo" : "Colapsar grupo"}
              onclick={() => onUpdateGroup(item.group.id, { collapsed: !item.group.collapsed })}
            >
              {item.group.collapsed ? "▸" : "▾"}
            </button>

            {#if renamingId === item.group.id}
              <input
                class="group-rename"
                bind:this={renameInput}
                bind:value={renameValue}
                onblur={() => commitRename(item.group)}
                onkeydown={(e) => onRenameKey(e, item.group)}
              />
            {:else}
              <span
                class="color-dot"
                class:no-color={!item.group.color}
                style={item.group.color ? `background:${item.group.color}` : ""}
              ></span>
              <button
                class="group-name"
                type="button"
                title="Doble clic para renombrar"
                ondblclick={() => startRename(item.group)}
              >
                {item.group.name}
              </button>
              <span class="group-count">{item.children.length}</span>
              <button
                class="icon-btn color-btn"
                type="button"
                title="Color del grupo"
                onclick={(e) => {
                  e.stopPropagation();
                  colorPickerId = colorPickerId === item.group.id ? null : item.group.id;
                }}
              >
                🎨
              </button>
              <button
                class="icon-btn"
                type="button"
                title="Eliminar grupo"
                onclick={(e) => {
                  e.stopPropagation();
                  removeGroup(item.group, item.children.length);
                }}
              >
                ✕
              </button>
            {/if}
          </div>

          {#if colorPickerId === item.group.id}
            <div class="swatches">
              {#each GROUP_COLORS as col (col)}
                <button
                  class="swatch"
                  type="button"
                  title={col}
                  style={`background:${col}`}
                  onclick={() => {
                    onUpdateGroup(item.group.id, { color: col });
                    colorPickerId = null;
                  }}
                ></button>
              {/each}
              <button
                class="swatch none"
                type="button"
                title="Sin color"
                onclick={() => {
                  onUpdateGroup(item.group.id, { color: null });
                  colorPickerId = null;
                }}
              ></button>
            </div>
          {/if}

          {#if !item.group.collapsed}
            <div
              class="group-children"
              role="group"
              aria-label="Catálogos de {item.group.name}"
              ondragover={(e) => overGroupBody(e, item.group)}
              ondrop={onDrop}
            >
              {#each item.children as c (c.id)}
                {@render catalogRow(c, true)}
              {/each}
              {#if item.children.length === 0}
                <div class="group-empty">Sin catálogos · arrastra uno aquí</div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    {/each}

    {#if drag?.kind === "catalog"}
      <div
        class="root-drop"
        role="group"
        aria-label="Quitar del grupo"
        class:active={dropTarget?.type === "root"}
        ondragover={overRoot}
        ondrop={onDrop}
      >
        Suelta aquí para dejarlo sin grupo
      </div>
    {/if}

    {#if catalogs.length === 0 && groups.length === 0}
      <p class="empty">Aún no hay catálogos.<br />Crea uno para empezar.</p>
    {/if}
  </nav>

  {#if pendingDelete}
    <ConfirmDialog
      title={confirmText.title}
      message={confirmText.message}
      details={confirmText.details}
      confirmLabel={confirmText.label}
      onConfirm={confirmDelete}
      onCancel={() => (pendingDelete = null)}
    />
  {/if}
</aside>
