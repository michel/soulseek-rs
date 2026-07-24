import { StrictMode } from 'react'
import { renderToString } from 'react-dom/server'
import { StaticRouter } from 'react-router'

import { App } from '@/App'
import { ThemeProvider } from '@/theme/theme-provider'

/**
 * Renders one route to a static HTML string.
 *
 * `location` is the app-relative path (no base prefix) — the client mounts
 * with `basename`, so the two see the same paths and hydration matches.
 */
export const render = (location: string): string =>
  renderToString(
    <StrictMode>
      <ThemeProvider>
        <StaticRouter location={location}>
          <App />
        </StaticRouter>
      </ThemeProvider>
    </StrictMode>,
  )
