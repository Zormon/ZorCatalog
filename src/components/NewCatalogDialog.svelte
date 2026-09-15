<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { open as openFolder } from "@tauri-apps/plugin-dialog";
  import { api } from "../lib/api";
  import { fmtCount } from "../lib/format";
  import type { DiskMeta, ScanProgress } from "../lib/types";

  let {
    onCreated,
    onCancel,
  }: {
    onCreated: (d: DiskMeta) => void;
    onCancel: () => void;
  } = $props();

  let name = $state("");
  let folder = $state("");
  let busy = $state(false);
  let progress = $state<ScanProgress | null>(null);
  let err = $state("");

  async function pick() {
    const sel = await openFolder({ directory: true, multiple: false });
    if (typeof sel === "string") folder = sel;
  }

  async function start() {
    err = "";
    if (!name.trim()) {
      err = "Escribe un nombre para el catálogo.";
      return;
    }
    if (!folder) {
      err = "Selecciona una carpeta o disco para escanear.";
      return;
    }
    busy = true;
    const un = await listen<ScanProgress>("scan-progress", (ev) => {
      progress = ev.payload;
    });
    try {
      const meta = await api.createCatalog(name.trim(), folder);
      un();
      onCreated(meta);
    } catch (e) {
      un();
      err = String(e).replace(/^Error:\s*/, "");
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-backdrop">
  <div class="dialog" role="dialog" aria-modal="true">
    <h2>Nuevo catálogo</h2>

    {#if !busy}
      <div class="field">
        <label for="catalog-name">Nombre del disco</label>
        <input
          id="catalog-name"
          type="text"
          placeholder="p. ej. Fotos 2025"
          bind:value={name}
        />
      </div>

      <div class="field">
        <label for="catalog-folder">Carpeta o disco a catalogar</label>
        <div class="folder-row">
          <input id="catalog-folder" type="text" bind:value={folder} placeholder="E:\Fotos" />
          <button class="btn" type="button" onclick={pick}>Elegir…</button>
        </div>
      </div>

      {#if err}
        <p class="err">{err}</p>
      {/if}

      <div class="dialog-actions">
        <button class="btn" type="button" onclick={onCancel}>Cancelar</button>
        <button class="btn primary" type="button" onclick={start}>
          Catalogar
        </button>
      </div>
    {:else}
      <div class="progress">
        <div class="progress-bar"></div>
        <div class="progress-text">
          Escaneando… <b>{progress ? fmtCount(progress.processed) : "0"}</b> elementos
        </div>
        {#if progress?.current}
          <div class="current-path">{progress.current}</div>
        {/if}
      </div>
      <div class="dialog-actions">
        <button class="btn danger" type="button" onclick={() => api.cancelScan()}>
          Cancelar escaneo
        </button>
      </div>
    {/if}
  </div>
</div>
