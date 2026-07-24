import { readFileSync } from 'node:fs'
import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const cargoToml = readFileSync(
  fileURLToPath(new URL('../Cargo.toml', import.meta.url)),
  'utf8',
)
const versionMatch = /^\s*version\s*=\s*"([^"]+)"/m.exec(cargoToml)
if (versionMatch?.[1] === undefined)
  throw new Error('could not read version from ../Cargo.toml')
const VERSION = `v${versionMatch[1]}`

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
    chunkSizeWarningLimit: 600,
  },
})
