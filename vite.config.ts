import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

// https://vite.dev/config/
export default defineConfig(({ command }) => ({
  // Dev runs under bun (system Node is too old for Vite). Under bun, the
  // plugin's extracted-CSS virtual module (?svelte&type=style) serves the
  // raw .svelte source instead of compiled CSS, dropping every scoped style.
  // Injected CSS sidesteps that in dev; production builds extract normally.
  plugins: [svelte({ emitCss: command === 'build' })],
}))
