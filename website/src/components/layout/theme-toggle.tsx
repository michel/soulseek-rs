import { useTheme } from '@/theme/use-theme'
import type { Theme } from '@/theme/theme-context'

const NEXT: Record<Theme, Theme> = {
  system: 'light',
  light: 'dark',
  dark: 'system',
}

const LABEL: Record<Theme, string> = {
  system: 'system',
  light: 'light',
  dark: 'dark',
}

export const ThemeToggle = () => {
  const { theme, resolvedTheme, setTheme } = useTheme()

  return (
    <button
      type="button"
      onClick={() => {
        setTheme(NEXT[theme])
      }}
      aria-label={`Theme: ${LABEL[theme]}. Switch to ${LABEL[NEXT[theme]]}.`}
      title={`Theme: ${LABEL[theme]}`}
      className="inline-flex h-[38px] items-center justify-center gap-1.5 rounded-md border border-hairline bg-transparent px-2.5 font-mono text-xs text-secondary hover:border-line hover:bg-[var(--bg-selection)] hover:text-primary sm:h-8"
    >
      <span aria-hidden="true">{resolvedTheme === 'light' ? '☀' : '☾'}</span>
      {theme === 'system' && (
        <span aria-hidden="true" className="text-[10px] opacity-70">
          auto
        </span>
      )}
    </button>
  )
}
