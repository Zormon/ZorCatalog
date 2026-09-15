<script lang="ts">
  import { thumbSrc } from "../lib/api";
  import { fmtDate, fmtSize } from "../lib/format";
  import type { DiskMeta, Node } from "../lib/types";

  let {
    disk,
    ancestors,
    children,
    loading,
    onOpen,
    onCrumb,
  }: {
    disk: DiskMeta;
    ancestors: Node[];
    children: Node[];
    loading: boolean;
    onOpen: (n: Node) => void;
    onCrumb: (n: Node | null) => void;
  } = $props();

  let grid = $state(true);
  const lastIndex = $derived(ancestors.length - 1);
</script>

<header class="topbar">
  <div class="crumbs">
    <button class="crumb {ancestors.length === 0 ? 'current' : ''}" type="button" onclick={() => onCrumb(null)}>
      {disk.name}
    </button>
    {#each ancestors as a, i (a.id)}
      <span class="sep">›</span>
      <button
        class="crumb {i === lastIndex ? 'current' : ''}"
        type="button"
        onclick={() => i < lastIndex && onCrumb(a)}
      >
        {a.name}
      </button>
    {/each}
  </div>
  <button class="btn view-toggle" type="button" onclick={() => (grid = !grid)}>
    {grid ? "☰ Lista" : "▦ Cuadrícula"}
  </button>
</header>

<div class="body">
  {#if loading}
    <div class="loading">Cargando…</div>
  {:else if children.length === 0}
    <div class="empty-view">Esta carpeta está vacía.</div>
  {:else if grid}
    <div class="thumb-grid">
      {#each children as n (n.id)}
        <button class="thumb-card" type="button" onclick={() => onOpen(n)} title={n.name}>
          <div class="tile">
            <span class="fallback">{n.isDir ? "📁" : "📄"}</span>
            {#if !n.isDir}
              <img
                src={thumbSrc(n.id)}
                alt={n.name}
                loading="lazy"
                onerror={(e) => e.currentTarget.classList.add("hidden")}
              />
            {/if}
          </div>
          <span class="thumb-name">{n.name}</span>
          {#if !n.isDir}
            <span class="thumb-size">{fmtSize(n.size ?? 0)} · {fmtDate(n.modified)}</span>
          {/if}
        </button>
      {/each}
    </div>
  {:else}
    <table class="file-list">
      <thead>
        <tr>
          <th>Nombre</th>
          <th>Tamaño</th>
          <th>Modificado</th>
        </tr>
      </thead>
      <tbody>
        {#each children as n (n.id)}
          <tr>
            <td>
              <button class="name-cell" type="button" onclick={() => onOpen(n)}>
                <span>{n.isDir ? "📁" : "📄"}</span>
                <span title={n.path}>{n.name}</span>
              </button>
            </td>
            <td>{n.isDir ? "—" : fmtSize(n.size ?? 0)}</td>
            <td>{fmtDate(n.modified)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>
