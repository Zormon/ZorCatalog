<script lang="ts">
  import Sidebar from "./components/Sidebar.svelte";
  import Explorer from "./components/Explorer.svelte";
  import SearchView from "./components/SearchView.svelte";
  import NewCatalogDialog from "./components/NewCatalogDialog.svelte";
  import ImageModal from "./components/ImageModal.svelte";
  import { api } from "./lib/api";
  import type { DiskMeta, Node, SearchHit } from "./lib/types";

  let catalogs = $state<DiskMeta[]>([]);
  let selectedDisk = $state<DiskMeta | null>(null);
  let view = $state<"browse" | "search">("browse");
  let showNewCatalog = $state(false);
  let ancestors = $state<Node[]>([]);
  let children = $state<Node[]>([]);
  let previewNode = $state<Node | null>(null);
  let loadingFolder = $state(false);
  let error = $state("");

  $effect(() => {
    void refreshCatalogs();
  });

  async function refreshCatalogs() {
    try {
      catalogs = await api.listCatalogs();
    } catch (e) {
      error = String(e);
    }
  }

  async function selectDisk(d: DiskMeta | null) {
    selectedDisk = d;
    view = "browse";
    ancestors = [];
    children = [];
    error = "";
    if (d) {
      try {
        children = await api.getChildren(d.id, null);
      } catch (e) {
        error = String(e);
      }
    }
  }

  async function openNode(node: Node) {
    if (node.isDir) {
      await loadFolder(node);
    } else {
      previewNode = node;
    }
  }

  async function loadFolder(node: Node) {
    loadingFolder = true;
    try {
      const chain = await api.getAncestors(node.id);
      // La raíz del árbol ya se representa con el nombre del disco en el
      // breadcrumb: la filtramos para no duplicar el nombre.
      ancestors = chain.filter((a) => a.parentId != null);
      children = await api.getChildren(node.diskId, node.id);
      view = "browse";
    } catch (e) {
      error = String(e);
    } finally {
      loadingFolder = false;
    }
  }

  async function openSearchResult(hit: SearchHit) {
    const n = hit.node;
    // Para un archivo abrimos su carpeta contenedora; para una carpeta, ella misma.
    const targetId = n.isDir ? n.id : n.parentId;
    if (targetId == null) return;
    loadingFolder = true;
    try {
      const chain = await api.getAncestors(targetId);
      ancestors = chain.filter((a) => a.parentId != null);
      children = await api.getChildren(n.diskId, targetId);
      view = "browse";
    } catch (e) {
      error = String(e);
    } finally {
      loadingFolder = false;
    }
  }

  function crumb(target: Node | null) {
    if (target === null) {
      void openRoot();
    } else {
      void loadFolder(target);
    }
  }

  async function openRoot() {
    if (!selectedDisk) return;
    loadingFolder = true;
    try {
      ancestors = [];
      children = await api.getChildren(selectedDisk.id, null);
    } catch (e) {
      error = String(e);
    } finally {
      loadingFolder = false;
    }
  }

  function onCatalogCreated(meta: DiskMeta) {
    showNewCatalog = false;
    void selectDisk(meta);
    void refreshCatalogs();
  }

  async function removeCatalog(id: number) {
    try {
      await api.deleteCatalog(id);
      if (selectedDisk?.id === id) await selectDisk(null);
      await refreshCatalogs();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<div class="layout">
  <Sidebar
    {catalogs}
    selected={selectedDisk}
    onSelect={(d) => void selectDisk(d)}
    onNew={() => (showNewCatalog = true)}
    onDelete={(id) => void removeCatalog(id)}
  />

  <main class="content">
    <div class="topbar">
      <button
        class="btn view-toggle {view === 'browse' ? 'primary' : ''}"
        type="button"
        onclick={() => (view = "browse")}
      >
        Explorar
      </button>
      <button
        class="btn view-toggle {view === 'search' ? 'primary' : ''}"
        type="button"
        onclick={() => (view = "search")}
      >
        Buscar
      </button>
      {#if error}
        <span class="err">{error}</span>
      {/if}
    </div>

    {#if view === "search"}
      <SearchView {catalogs} onOpen={(h) => void openSearchResult(h)} />
    {:else if selectedDisk}
      <Explorer
        disk={selectedDisk}
        {ancestors}
        {children}
        loading={loadingFolder}
        onOpen={(n) => void openNode(n)}
        onCrumb={crumb}
      />
    {:else}
      <div class="body">
        <div class="welcome">
          <h1>ZorCatalog</h1>
          <p>
            Cataloga tus discos externos y carpetas con un nombre, explora su
            árbol de directorios y ve las miniaturas de tus imágenes sin
            necesidad de conectar el disco.
          </p>
          <button class="btn primary" type="button" onclick={() => (showNewCatalog = true)}>
            + Crear primer catálogo
          </button>
        </div>
      </div>
    {/if}
  </main>
</div>

{#if showNewCatalog}
  <NewCatalogDialog
    onCreated={onCatalogCreated}
    onCancel={() => (showNewCatalog = false)}
  />
{/if}

{#if previewNode}
  <ImageModal node={previewNode} onClose={() => (previewNode = null)} />
{/if}
