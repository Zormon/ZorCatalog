<script lang="ts">
  // El frontend también se puede servir en un navegador (`pnpm dev:web`), donde
  // no existe el puente con Rust. Ahí no montamos la app —todo serían errores
  // de `invoke`— sino este aviso.
</script>

<div class="content">
  <div class="body">
    <div class="welcome">
      <h1>ZorCatalog</h1>
      <p>
        Esta vista necesita la app de escritorio: sin el backend de Rust no hay
        catálogos que mostrar.
      </p>
      <p>
        Arráncala con <code>pnpm dev</code>. Si estás viendo esto en el
        navegador, ten en cuenta que <code>localhost:1420</code> solo sirve el
        frontend y no tiene backend.
      </p>
    </div>
  </div>
</div>
