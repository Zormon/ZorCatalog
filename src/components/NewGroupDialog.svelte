<script lang="ts">
  import { api } from "../lib/api";
  import type { CatalogGroup } from "../lib/types";
  import { GROUP_COLORS } from "../lib/types";

  let {
    onCreated,
    onCancel,
  }: {
    onCreated: (g: CatalogGroup) => void;
    onCancel: () => void;
  } = $props();

  let name = $state("");
  let color = $state<string | null>(GROUP_COLORS[0]);
  let busy = $state(false);
  let err = $state("");

  async function create() {
    err = "";
    if (!name.trim()) {
      err = "Escribe un nombre para el grupo.";
      return;
    }
    busy = true;
    try {
      onCreated(await api.createGroup(name.trim(), color));
    } catch (e) {
      err = String(e).replace(/^Error:\s*/, "");
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-backdrop">
  <div class="dialog" role="dialog" aria-modal="true">
    <h2>Nuevo grupo</h2>

    <div class="field">
      <label for="group-name">Nombre del grupo</label>
      <input
        id="group-name"
        type="text"
        placeholder="p. ej. Discos de fotos"
        bind:value={name}
        onkeydown={(e) => e.key === "Enter" && create()}
      />
    </div>

    <div class="field">
      <span class="field-label" id="group-color-label">Color</span>
      <div class="swatches" role="group" aria-labelledby="group-color-label">
        {#each GROUP_COLORS as col (col)}
          <button
            class="swatch"
            class:selected={color === col}
            type="button"
            title={col}
            aria-pressed={color === col}
            style={`background:${col}`}
            onclick={() => (color = col)}
          ></button>
        {/each}
        <button
          class="swatch none"
          class:selected={color === null}
          type="button"
          title="Sin color"
          aria-pressed={color === null}
          onclick={() => (color = null)}
        ></button>
      </div>
    </div>

    {#if err}
      <p class="err">{err}</p>
    {/if}

    <div class="dialog-actions">
      <button class="btn" type="button" onclick={onCancel}>Cancelar</button>
      <button class="btn primary" type="button" onclick={create} disabled={busy}>
        Crear grupo
      </button>
    </div>
  </div>
</div>
