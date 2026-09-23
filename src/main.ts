import { mount } from "svelte";
import App from "./App.svelte";
import NoBackend from "./components/NoBackend.svelte";
import { isDesktop } from "./lib/api";
import "./app.css";

const target = document.getElementById("app")!;

// `pnpm dev:web` sirve el frontend en un navegador normal, sin el puente con
// Rust: montamos un aviso en vez de una app condenada a fallar con TypeError.
const app = isDesktop() ? mount(App, { target }) : mount(NoBackend, { target });

export default app;
