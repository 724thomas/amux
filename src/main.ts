import { mount } from 'svelte'
import './app.css'

// The files pop-out window loads this same page; its window label picks
// what to show (see src-tauri/src/popout.rs).
type Internals = { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }
const label = (window as unknown as Internals).__TAURI_INTERNALS__?.metadata?.currentWindow?.label
const target = document.getElementById('app')!

const app =
  label === 'files'
    ? import('./lib/chat/FilesBoard.svelte').then(({ default: Board }) => mount(Board, { target }))
    : import('./App.svelte').then(({ default: App }) => mount(App, { target }))

export default app
