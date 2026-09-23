<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { open as openFile, save as saveFile } from "@tauri-apps/plugin-dialog";
  import Sidebar from "./components/Sidebar.svelte";
  import Explorer from "./components/Explorer.svelte";
  import SearchView from "./components/SearchView.svelte";
  import NewCatalogDialog from "./components/NewCatalogDialog.svelte";
  import NewGroupDialog from "./components/NewGroupDialog.svelte";
  import ImageModal from "./components/ImageModal.svelte";
  import BackupResultDialog from "./components/BackupResultDialog.svelte";
  import { api } from "./lib/api";
  import type {
    BackupProgress,
    BackupResult,
    CatalogGroup,
    DiskMeta,
    GroupPatch,
    Node,
    SearchHit,
    SidebarLayoutEntry,
  } from "./lib/types";

  let catalogs = $state<DiskMeta[]>([]);
  let groups = $state<CatalogGroup[]>([]);
  let selectedDisk = $state<DiskMeta | null>(null);
  let view = $state<"browse" | "search">("browse");
  let showNewCatalog = $state(false);
  let showNewGroup = $state(false);
  let ancestors = $state<Node[]>([]);
  let children = $state<Node[]>([]);
  let previewNode = $state<Node | null>(null);
  let loadingFolder = $state(false);
  let error = $state("");
  /** Copia en curso: `null` cuando no hay ninguna. */
  let backupBusy = $state<"export" | "import" | null>(null);
  let backupProgress = $state<BackupProgress | null>(null);
  let backupResult = $state<BackupResult | null>(null);

  $effect(() => {
    void refreshSidebar();
  });

  async function refreshSidebar() {
    try {
      const [g, c] = await Promise.all([api.listGroups(), api.listCatalogs()]);
      groups = g;
      catalogs = c;
    } catch (e) {
      error = String(e);
    }
  }

  function clearError(e: unknown) {
    error = String(e).replace(/^Error:\s*/, "");
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
    void refreshSidebar();
  }

  function onGroupCreated() {
    showNewGroup = false;
    void refreshSidebar();
  }

  async function removeCatalog(id: number) {
    try {
      await api.deleteCatalog(id);
      if (selectedDisk?.id === id) await selectDisk(null);
      await refreshSidebar();
    } catch (e) {
      error = String(e);
    }
  }

  async function removeGroup(id: number) {
    try {
      await api.deleteGroup(id);
      await refreshSidebar();
    } catch (e) {
      clearError(e);
    }
  }

  /** Colapso, color y renombrado comparten comando: se mandan los tres campos. */
  async function updateGroup(id: number, patch: GroupPatch) {
    const current = groups.find((g) => g.id === id);
    if (!current) return;
    const name = patch.name ?? current.name;
    const color = patch.color !== undefined ? patch.color : current.color;
    const collapsed = patch.collapsed ?? current.collapsed;
    const before = groups;

    // Optimista: colapsar y cambiar el color deben verse al instante.
    groups = groups.map((g) => (g.id === id ? { ...g, name, color, collapsed } : g));
    try {
      const saved = await api.updateGroup(id, name, color, collapsed);
      groups = groups.map((g) => (g.id === id ? saved : g));
    } catch (e) {
      groups = before;
      clearError(e);
    }
  }

  /**
   * Recalcula el estado local a partir del layout que manda el sidebar, para
   * que el arrastre se vea sin esperar al backend, y luego lo persiste.
   */
  async function changeLayout(entries: SidebarLayoutEntry[]) {
    const before = { groups, catalogs };
    const groupById = new Map(groups.map((g) => [g.id, g]));
    const catalogById = new Map(catalogs.map((c) => [c.id, c]));
    const nextGroups: CatalogGroup[] = [];
    const nextCatalogs: DiskMeta[] = [];

    let pos = 0;
    for (const entry of entries) {
      if (entry.kind === "group") {
        const g = groupById.get(entry.id);
        if (g) nextGroups.push({ ...g, position: pos });
        entry.catalogs.forEach((cid, i) => {
          const c = catalogById.get(cid);
          if (c) nextCatalogs.push({ ...c, groupId: entry.id, position: i });
        });
      } else {
        const c = catalogById.get(entry.id);
        if (c) nextCatalogs.push({ ...c, groupId: null, position: pos });
      }
      pos += 1;
    }

    groups = nextGroups;
    catalogs = nextCatalogs;
    try {
      await api.setSidebarLayout(entries);
    } catch (e) {
      groups = before.groups;
      catalogs = before.catalogs;
      clearError(e);
    }
  }

  // --- Copias externas (.zcbak) ------------------------------------------

  /** Texto del progreso, según la etapa que esté en curso. */
  function backupStatus(p: BackupProgress | null, busy: "export" | "import" | null) {
    if (!busy) return "";
    if (!p) {
      return busy === "export" ? "Preparando la copia…" : "Preparando la importación…";
    }
    const pct = p.total > 0 ? Math.round((p.done / p.total) * 100) : 0;
    if (p.stage === "db") return `Reuniendo catálogos… ${p.done}/${p.total}`;
    if (p.stage === "compress") return `Comprimiendo… ${pct} %`;
    if (p.stage === "decompress") return `Descomprimiendo… ${pct} %`;
    return `Importando ${p.done}/${p.total}${p.current ? `: ${p.current}` : ""}`;
  }

  /** Escucha el progreso de una copia y devuelve la función para dejar de escuchar. */
  async function trackBackup(phase: "export" | "import") {
    return listen<BackupProgress>("backup-progress", (ev) => {
      if (ev.payload.phase === phase) backupProgress = ev.payload;
    });
  }

  async function doExport() {
    error = "";
    const hoy = new Date().toISOString().slice(0, 10);
    const path = await saveFile({
      title: "Exportar copia de ZorCatalog",
      defaultPath: `zorcatalog-${hoy}.zcbak`,
      filters: [{ name: "Copia de ZorCatalog", extensions: ["zcbak"] }],
    });
    if (!path) return; // cancelado

    backupBusy = "export";
    backupProgress = null;
    const un = await trackBackup("export");
    try {
      const data = await api.exportBackup(path);
      backupResult = { kind: "export", data };
    } catch (e) {
      clearError(e);
    } finally {
      un();
      backupBusy = null;
      backupProgress = null;
    }
  }

  async function doImport() {
    error = "";
    const sel = await openFile({
      title: "Importar copia de ZorCatalog",
      multiple: false,
      filters: [{ name: "Copia de ZorCatalog", extensions: ["zcbak"] }],
    });
    if (typeof sel !== "string") return; // cancelado

    backupBusy = "import";
    backupProgress = null;
    const un = await trackBackup("import");
    try {
      const data = await api.importBackup(sel);
      await refreshSidebar();
      backupResult = { kind: "import", data };
    } catch (e) {
      clearError(e);
    } finally {
      un();
      backupBusy = null;
      backupProgress = null;
    }
  }
</script>

<div class="layout">
  <Sidebar
    {catalogs}
    {groups}
    selected={selectedDisk}
    onSelect={(d) => void selectDisk(d)}
    onNewCatalog={() => (showNewCatalog = true)}
    onNewGroup={() => (showNewGroup = true)}
    onDeleteCatalog={(id) => void removeCatalog(id)}
    onDeleteGroup={(id) => void removeGroup(id)}
    onUpdateGroup={(id, patch) => void updateGroup(id, patch)}
    onLayoutChange={(entries) => void changeLayout(entries)}
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
      <span class="spacer"></span>
      {#if backupBusy}
        <span class="backup-status">{backupStatus(backupProgress, backupBusy)}</span>
      {/if}
      <button
        class="btn"
        type="button"
        disabled={backupBusy !== null}
        title="Guardar una copia externa de todos los catálogos"
        onclick={() => void doExport()}
      >
        Exportar
      </button>
      <button
        class="btn"
        type="button"
        disabled={backupBusy !== null}
        title="Añadir los catálogos de una copia externa"
        onclick={() => void doImport()}
      >
        Importar
      </button>
    </div>

    {#if view === "search"}
      <SearchView
        {catalogs}
        {groups}
        onOpen={(h) => void openSearchResult(h)}
      />
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
    {groups}
    onCreated={onCatalogCreated}
    onCancel={() => (showNewCatalog = false)}
  />
{/if}

{#if showNewGroup}
  <NewGroupDialog
    onCreated={onGroupCreated}
    onCancel={() => (showNewGroup = false)}
  />
{/if}

{#if previewNode}
  <ImageModal node={previewNode} onClose={() => (previewNode = null)} />
{/if}

{#if backupResult}
  <BackupResultDialog result={backupResult} onClose={() => (backupResult = null)} />
{/if}
