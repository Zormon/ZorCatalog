<script lang="ts">
  let {
    title,
    message,
    details = [],
    confirmLabel = "Eliminar",
    onConfirm,
    onCancel,
  }: {
    title: string;
    message: string;
    /** Líneas extra de contexto (qué se borra y qué no). */
    details?: string[];
    confirmLabel?: string;
    onConfirm: () => void;
    onCancel: () => void;
  } = $props();

  let cancelBtn = $state<HTMLButtonElement | null>(null);

  // El foco arranca en Cancelar: un Enter reflejo no confirma el borrado.
  $effect(() => {
    cancelBtn?.focus();
  });
</script>

<div
  class="dialog-backdrop"
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && onCancel()}
  onkeydown={(e) => e.key === "Escape" && onCancel()}
>
  <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title">
    <h2 id="confirm-title">{title}</h2>
    <p class="dialog-text">{message}</p>

    {#if details.length > 0}
      <ul class="dialog-details">
        {#each details as line (line)}
          <li>{line}</li>
        {/each}
      </ul>
    {/if}

    <div class="dialog-actions">
      <button class="btn" type="button" bind:this={cancelBtn} onclick={onCancel}>Cancelar</button>
      <button class="btn danger-solid" type="button" onclick={onConfirm}>{confirmLabel}</button>
    </div>
  </div>
</div>
