import { VERSION } from '@/lib/links'
import { cn } from '@/lib/utils'

import { CLOCK, DATE, HOST, SESSION, SHORTCUTS, WINDOWS } from './data'

interface TuiPaneProps {
  title: string
  num?: number
  active?: boolean
  children: React.ReactNode
  className?: string
  bodyClassName?: string
}

/**
 * The TUI's bordered pane: a legend that breaks the top border, green when
 * the pane has focus. Mirrors `pane_block()` in the Rust client.
 */
export const TuiPane = ({
  title,
  num,
  active = false,
  children,
  className,
  bodyClassName,
}: TuiPaneProps) => (
  <div
    className={cn(
      'relative border bg-base',
      active ? 'border-[var(--color-phosphor)]' : 'border-[color-mix(in_srgb,var(--color-dust)_45%,transparent)]',
      className,
    )}
  >
    <span
      className={cn(
        'absolute -top-[9px] left-3 bg-base px-1.5 text-[12.5px] whitespace-nowrap',
        active ? 'text-[var(--color-phosphor)]' : 'text-[var(--color-dust)]',
      )}
    >
      {num !== undefined && <span className="text-[var(--color-signal)]">[{num}] </span>}
      {title}
    </span>
    <div className={bodyClassName}>{children}</div>
  </div>
)

/** The macOS window bezel the whole demo sits in. */
export const MacWindow = ({ children }: { children: React.ReactNode }) => (
  <div
    className="w-[1360px] overflow-hidden rounded-xl border bg-base font-mono tabular-nums"
    style={{ borderColor: 'var(--tui-bezel)' }}
  >
    {children}
  </div>
)

export const TmuxBar = () => (
  <div className="flex h-[26px] items-center border-b border-[color-mix(in_srgb,var(--color-dust)_25%,transparent)] bg-[var(--tmux-bg)] text-[12.5px] text-[var(--color-phosphor)]">
    <span className="flex h-full items-center bg-[var(--color-phosphor)] px-2.5 font-bold text-[var(--tmux-bg)]">
      {SESSION}
    </span>
    {WINDOWS.map((w) => (
      <span
        key={w.n}
        className={cn(
          'flex h-full items-center gap-1.5 px-3',
          w.active === true
            ? 'bg-[var(--tmux-hl)] font-bold text-[var(--tmux-hl-fg)]'
            : 'text-[var(--color-phosphor)]',
        )}
      >
        {w.n} <span className="opacity-55">›</span> {w.name}
      </span>
    ))}
    <span className="ml-auto flex gap-4 px-3 text-[var(--color-paper)]">
      <span className="bg-[var(--tmux-hl)] px-2 font-semibold text-[var(--tmux-hl-fg)]">
        {DATE} · {CLOCK}
      </span>
      <span className="font-bold">{HOST}</span>
    </span>
  </div>
)

export interface StatusCounts {
  active: number
  done: number
  fail: number
  queued: number
}

interface StatusLineProps {
  counts: StatusCounts
  pct: number
}

export const StatusLine = ({ counts, pct }: StatusLineProps) => (
  <TuiPane title="Status" bodyClassName="px-3 py-2">
    <div className="flex items-center gap-3.5 text-[13px] whitespace-nowrap text-[var(--color-paper)]">
      <span>
        <b className="font-semibold">soulseek-rs</b> 🦀 {VERSION}
      </span>
      <span className="text-[var(--color-dust)]">
        Downloads: <span className="text-[var(--color-oxide)]">{counts.active} active</span>
        , <span className="text-[var(--color-phosphor)]">{counts.done} completed</span>,{' '}
        <span className="text-[var(--color-alarm)]">{counts.fail} failed</span>,{' '}
        <span className="text-[var(--color-tape)]">{counts.queued} queued</span>, 0 paused
      </span>
      <span className="ml-auto flex items-center gap-2.5 text-[var(--color-paper)]">
        67.7/205.2 MB · 33.38 MB/s
        <span className="relative inline-block h-[13px] w-[340px] border border-[var(--tm-bd)] bg-[var(--tui-track)]">
          <span
            className="absolute inset-y-0 left-0 bg-[var(--color-signal)] opacity-55 transition-[width] duration-300"
            style={{ width: `${String(pct)}%` }}
          />
        </span>
        <b>{pct}%</b>
      </span>
    </div>
  </TuiPane>
)

export const ShortcutBar = ({ share }: { share: string }) => (
  <TuiPane title="Shortcuts · Sharing" className="mt-2.5" bodyClassName="px-3 py-2">
    <div className="mb-1.5 text-xs text-[var(--color-dust)]">
      Sharing: <span className="text-[var(--color-phosphor)]">{share}</span>
    </div>
    <div className="flex flex-wrap gap-3.5 text-[12.5px] text-[var(--color-dust)]">
      {SHORTCUTS.map(([key, action]) => (
        <span key={key}>
          [<span className="text-[var(--color-signal)]">{key}</span> → {action}]
        </span>
      ))}
    </div>
  </TuiPane>
)
