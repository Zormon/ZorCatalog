<script lang="ts">
  import { fmtCount, fmtSize } from "../lib/format";
  import type { BackupResult } from "../lib/types";

  let {
    result,
    onClose,
  }: {
    result: BackupResult;
    onClose: () => void;
  } = $props();

  let closeBtn = $state<HTMLButtonElement | null>(null);

  // A diferencia de `ConfirmDialog`, aquí no hay nada destructivo: el foco va
  // al único botón y Escape/backdrop cierran.
  $effect(() => {
    closeBtn?.focus();
  });

  const title = $derived(
    result.kind === "export" ? "Copia exportada" : "Copia importada",
  );

  /** Porcentaje de ahorro del paquete comprimido. */
  const ahorro = $derived.by(() => {
    if (result.kind !== "export") return 0;
    const { payloadBytes, fileBytes } = result.data;
    if (payloadBytes <= 0) return 0;
    return Math.max(0, Math.round((1 - fileBytes / payloadBytes) * 100));
  });
</script>

<div
  class="dialog-backdrop"
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && onClose()}
  onkeydown={(e) => e.key === "Escape" && onClose()}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="backup-title">
    <h2 id="backup-title">{title}</h2>

    {#if result.kind === "export"}
      <p class="dialog-text">
        Se guardaron {fmtCount(result.data.catalogs)} catálogo(s) con
        {fmtCount(result.data.nodes)} elementos y
        {fmtCount(result.data.thumbs)} miniaturas.
      </p>
      <ul class="dialog-details">
        <li>{result.data.filePath}</li>
        <li>
          {fmtSize(result.data.payloadBytes)} sin comprimir →
          <b>{fmtSize(result.data.fileBytes)}</b>
          {#if ahorro > 0}({ahorro} % menos){/if}
        </li>
      </ul>
    {:else}
      <p class="dialog-text">
        Se importaron {fmtCount(result.data.catalogsImported)} catálogo(s) con
        {fmtCount(result.data.nodes)} elementos.
      </p>
      <ul class="dialog-details">
        {#if result.data.groupsCreated > 0}
          <li>Grupos nuevos: {fmtCount(result.data.groupsCreated)}.</li>
        {/if}
        {#if result.data.groupsReused > 0}
          <li>Grupos reutilizados: {fmtCount(result.data.groupsReused)}.</li>
        {/if}
        {#each result.data.catalogsRenamed as r (r.from)}
          <li>«{r.from}» ya existía: se importó como «{r.to}».</li>
        {/each}
        {#each result.data.catalogsSkipped as name (name)}
          <li>«{name}» ya estaba en la lista: no se ha duplicado.</li>
        {/each}
      </ul>
    {/if}

    <div class="dialog-actions">
      <button class="btn primary" type="button" bind:this={closeBtn} onclick={onClose}>
        Cerrar
      </button>
    </div>
  </div>
</div>
