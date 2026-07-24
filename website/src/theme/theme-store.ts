import { isTheme, THEME_STORAGE_KEY, type ResolvedTheme, type Theme } from './theme-context'

const DARK_QUERY = '(prefers-color-scheme: dark)'

const listeners = new Set<() => void>()

const emit = () => {
  for (const listener of listeners) listener()
}

export const subscribe = (onChange: () => void): (() => void) => {
  listeners.add(onChange)
  const query = window.matchMedia(DARK_QUERY)
  query.addEventListener('change', onChange)
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
    return 'system'
  }
}

export const readResolvedTheme = (): ResolvedTheme => {
  const theme = readTheme()
  if (theme !== 'system') return theme
  return window.matchMedia(DARK_QUERY).matches ? 'dark' : 'light'
}

export const serverTheme = (): Theme => 'system'
export const serverResolvedTheme = (): ResolvedTheme => 'dark'

export const writeTheme = (next: Theme): boolean => {
  let persisted = true
  try {
    localStorage.setItem(THEME_STORAGE_KEY, next)
  } catch {
    persisted = false
  }
  emit()
  return persisted
}
