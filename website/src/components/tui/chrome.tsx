import { VERSION } from '@/lib/links'
import { cn } from '@/lib/utils'

import { SHORTCUTS } from './data'

interface TuiPaneProps {
  title: string
  num?: number
  active?: boolean
  children: React.ReactNode
  className?: string
  bodyClassName?: string
}

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

export const MacWindow = ({ children }: { children: React.ReactNode }) => (
  <div
    className="w-[1360px] overflow-hidden rounded-xl border bg-base font-mono tabular-nums"
    style={{ borderColor: 'var(--tui-bezel)' }}
  >
    {children}
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
