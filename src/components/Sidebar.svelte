<script lang="ts">
  import type { DiskMeta } from "../lib/types";
  import { fmtCount, fmtSize } from "../lib/format";

  let {
    catalogs,
    selected,
    onSelect,
    onNew,
    onDelete,
  }: {
    catalogs: DiskMeta[];
    selected: DiskMeta | null;
    onSelect: (d: DiskMeta) => void;
    onNew: () => void;
    onDelete: (id: number) => void;
  } = $props();

  function remove(d: DiskMeta) {
    if (confirm(`¿Eliminar el catálogo "${d.name}"?`)) onDelete(d.id);
  }
</script>

<aside class="sidebar">
  <div class="sidebar-head">
    <span class="logo">ZorCatalog</span>
    <button class="btn primary" type="button" onclick={onNew}>+ Nuevo</button>
  </div>
  <nav>
    {#each catalogs as c (c.id)}
      <div
        class="catalog-item {selected?.id === c.id ? 'active' : ''}"
        role="button"
        tabindex="0"
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
            remove(c);
          }}
        >
          ✕
        </button>
      </div>
    {/each}
    {#if catalogs.length === 0}
      <p class="empty">Aún no hay catálogos.<br />Crea uno para empezar.</p>
    {/if}
  </nav>
</aside>
