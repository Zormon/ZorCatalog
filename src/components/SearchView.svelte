<script lang="ts">
  import { api } from "../lib/api";
  import { fmtSize } from "../lib/format";
  import { bucketByGroup } from "../lib/tree";
  import type { CatalogGroup, DiskMeta, SearchHit } from "../lib/types";

  let {
    catalogs,
    groups,
    onOpen,
  }: {
    catalogs: DiskMeta[];
    groups: CatalogGroup[];
    onOpen: (h: SearchHit) => void;
  } = $props();

  let query = $state("");
  /**
   * Filtro activo, como texto porque un `<select>` solo guarda cadenas:
   * "0" = todos, "g:<id>" = grupo completo, "<id>" = catálogo concreto.
   */
  let filter = $state("0");
  let results = $state<SearchHit[]>([]);
  let searching = $state(false);

  const tree = $derived(bucketByGroup(groups, catalogs));

  function parseFilter(v: string): { diskId: number | null; groupId: number | null } {
    if (v.startsWith("g:")) return { diskId: null, groupId: Number(v.slice(2)) };
    const id = Number(v);
    return id === 0 ? { diskId: null, groupId: null } : { diskId: id, groupId: null };
  }

  $effect(() => {
    const q = query;
    const f = filter;
    const t = setTimeout(() => {
      void runSearch(q, f);
    }, 250);
    return () => clearTimeout(t);
  });

  async function runSearch(q: string, f: string) {
    if (q.trim().length < 2) {
      results = [];
      return;
    }
    searching = true;
    try {
      const { diskId, groupId } = parseFilter(f);
      results = await api.search(q, diskId, groupId, 200);
    } catch (e) {
      console.error(e);
      results = [];
    } finally {
      searching = false;
    }
  }
</script>

<div class="search-view">
  <div class="search-bar">
    <input
      type="search"
      placeholder="Buscar archivos y carpetas… (≥ 2 caracteres)"
      bind:value={query}
    />
    <select bind:value={filter} title="Ámbito de la búsqueda">
      <option value="0">Todos los catálogos</option>
      {#each tree.nodes as node (node.group.id)}
        <optgroup label={node.group.name}>
          <option value={`g:${node.group.id}`}>↳ Todo el grupo</option>
          {#each node.catalogs as c (c.id)}
            <option value={String(c.id)}>{c.name}</option>
          {/each}
        </optgroup>
      {/each}
      {#if tree.loose.length > 0}
        <optgroup label="Sin grupo">
          {#each tree.loose as c (c.id)}
            <option value={String(c.id)}>{c.name}</option>
          {/each}
        </optgroup>
      {/if}
    </select>
  </div>
  <div class="search-body">
    {#if query.trim().length < 2}
      <div class="empty-view">Escribe al menos 2 caracteres para buscar.</div>
    {:else if searching}
      <div class="loading">Buscando…</div>
    {:else if results.length === 0}
      <div class="empty-view">Sin resultados para “{query}”.</div>
    {:else}
      <div class="hits">
        {#each results as h (h.node.id)}
          <button class="hit" type="button" onclick={() => onOpen(h)}>
            <span class="hit-icon">{h.node.isDir ? "📁" : "📄"}</span>
            <div class="hit-info">
              <div class="hit-name">{h.node.name}</div>
              <div class="hit-path">{h.node.path || h.node.name}</div>
            </div>
            {#if !h.node.isDir}
              <span class="hit-path">{fmtSize(h.node.size ?? 0)}</span>
            {/if}
            <span class="disk-tag">{h.diskName}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>
