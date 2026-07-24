import { StrictMode } from 'react'
import { createRoot, hydrateRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router'

import { App } from '@/App'
import { ThemeProvider } from '@/theme/theme-provider'

import './index.css'

const container = document.getElementById('root')
if (container === null) throw new Error('Root element #root is missing')

const tree = (
  <StrictMode>
    <ThemeProvider>
      {/* Must match `base` in vite.config.ts — the site is served from a
          subpath, so the router has to strip it before matching. */}
      <BrowserRouter basename={import.meta.env.BASE_URL}>
        <App />
      </BrowserRouter>
    </ThemeProvider>
  </StrictMode>
)

// Every route ships prerendered markup, so the normal path is to hydrate it.
// createRoot is the fallback for a container that somehow arrived empty.
if (container.hasChildNodes()) {
  hydrateRoot(container, tree)
} else {
  createRoot(container).render(tree)
}
