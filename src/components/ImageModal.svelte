<script lang="ts">
  import { thumbSrc } from "../lib/api";
  import { fmtDate, fmtSize } from "../lib/format";
  import type { Node } from "../lib/types";

  let {
    node,
    onClose,
  }: {
    node: Node;
    onClose: () => void;
  } = $props();
</script>

<div
  class="modal-backdrop"
  role="presentation"
  onclick={onClose}
  onkeydown={(e) => e.key === "Escape" && onClose()}
>
  <div class="modal" role="dialog" aria-modal="true">
    <button class="icon-btn close" type="button" onclick={onClose}>✕</button>
    <img class="preview" src={thumbSrc(node.id)} alt={node.name} />
    <div class="meta">
      <strong>{node.name}</strong>
      <span>
        {node.path || node.name} · {fmtSize(node.size ?? 0)}
        {#if node.modified}
          · {fmtDate(node.modified)}
        {/if}
      </span>
    </div>
  </div>
</div>
