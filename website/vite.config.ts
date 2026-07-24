import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

/*
 * Read the version out of the workspace manifest rather than keeping a copy in
 * TypeScript. release-plz bumps Cargo.toml; a hand-written constant here would
 * silently advertise the previous release forever.
 */
const cargoToml = readFileSync(
  fileURLToPath(new URL('../Cargo.toml', import.meta.url)),
  'utf8',
)
const versionMatch = /^\s*version\s*=\s*"([^"]+)"/m.exec(cargoToml)
if (versionMatch?.[1] === undefined)
  throw new Error('could not read version from ../Cargo.toml')
const VERSION = `v${versionMatch[1]}`

// Project sites are served from https://<user>.github.io/<repo>/, so every
// asset URL needs that prefix baked in at build time.
// hardcoded. Point a custom domain at the site and this becomes '/'
// — one edit here plus the matching `basename` in src/main.tsx.
const BASE = '/soulseek-rs/'

export default defineConfig({
  base: BASE,
  define: {
    __APP_VERSION__: JSON.stringify(VERSION),
  },
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  build: {
    // Small site; a warning at the default 500 kB would be noise. A jump past
    // this means something heavy got imported by accident.
    chunkSizeWarningLimit: 600,
  },
})
