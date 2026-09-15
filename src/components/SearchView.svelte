<script lang="ts">
  import { api } from "../lib/api";
  import { fmtSize } from "../lib/format";
  import type { DiskMeta, SearchHit } from "../lib/types";

  let {
    catalogs,
    onOpen,
  }: {
    catalogs: DiskMeta[];
    onOpen: (h: SearchHit) => void;
  } = $props();

  let query = $state("");
  let diskFilter = $state(0);
  let results = $state<SearchHit[]>([]);
  let searching = $state(false);

  $effect(() => {
    const q = query;
    const d = diskFilter;
    const t = setTimeout(() => {
      void runSearch(q, d);
    }, 250);
    return () => clearTimeout(t);
  });

  async function runSearch(q: string, d: number) {
    if (q.trim().length < 2) {
      results = [];
      return;
    }
    searching = true;
    try {
      results = await api.search(q, d === 0 ? null : d, 200);
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
    <select bind:value={diskFilter}>
      <option value={0}>Todos los catálogos</option>
      {#each catalogs as c (c.id)}
        <option value={c.id}>{c.name}</option>
      {/each}
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
