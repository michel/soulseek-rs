import { isTheme, THEME_STORAGE_KEY, type ResolvedTheme, type Theme } from './theme-context'

const DARK_QUERY = '(prefers-color-scheme: dark)'

/*
 * The theme lives outside React: the boot script in index.html resolves and
 * paints it before the bundle loads, and the OS can change it at any time.
 * That makes it an external store, which is what useSyncExternalStore is for
 * — and it is the piece that gives correct hydration for free.
 *
 * The server snapshots below must match what the prerendered HTML ships
 * (`<html data-theme="dark">`). React renders the server snapshot for the
 * hydration pass, then immediately re-renders with the real one, so a visitor
 * whose actual theme is light never sees a mismatch, only a corrected render.
 */

const listeners = new Set<() => void>()

/** Notifies subscribers after a local write; OS changes come via matchMedia. */
const emit = () => {
  for (const listener of listeners) listener()
}

export const subscribe = (onChange: () => void): (() => void) => {
  listeners.add(onChange)
  const query = window.matchMedia(DARK_QUERY)
  query.addEventListener('change', onChange)
  // Another tab changing the preference should move this one too.
  window.addEventListener('storage', onChange)
  return () => {
    listeners.delete(onChange)
    query.removeEventListener('change', onChange)
    window.removeEventListener('storage', onChange)
  }
}

export const readTheme = (): Theme => {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY)
    return isTheme(stored) ? stored : 'system'
  } catch {
    // Safari in private mode throws on localStorage access.
    return 'system'
  }
}

export const readResolvedTheme = (): ResolvedTheme => {
  const theme = readTheme()
  if (theme !== 'system') return theme
  return window.matchMedia(DARK_QUERY).matches ? 'dark' : 'light'
}

/** Both must agree with the `data-theme` baked into index.html. */
export const serverTheme = (): Theme => 'system'
export const serverResolvedTheme = (): ResolvedTheme => 'dark'

export const writeTheme = (next: Theme): void => {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, next)
  } catch {
    // Not persisting is survivable; the theme still applies this session.
  }
  emit()
}
