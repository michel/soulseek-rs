import { cn } from '@/lib/utils'

interface PanelProps {
  /** Pane number, rendered in signal blue like the TUI's `[2]` tags. */
  num?: number | undefined
  title: string
  children: React.ReactNode
  className?: string
}

/**
 * The fieldset/legend motif lifted from the TUI: a bordered box whose title
 * breaks the top border. `bg-base` on the legend is what punches the gap, so
 * it has to match whatever surface the panel sits on.
 */
export const Panel = ({ num, title, children, className }: PanelProps) => (
  <div
    className={cn(
      'relative rounded-md border border-hairline bg-panel px-[18px] py-[22px] sm:px-6 sm:py-[26px]',
      className,
    )}
  >
    <span className="absolute -top-2.5 left-4 bg-base px-2 text-label uppercase tracking-[var(--tracking-label)] text-secondary">
      {/* --color-info, not the raw --color-signal: this panel renders on the
          page, outside .tui-content, where the raw palette never re-cuts. */}
      {num !== undefined && <span className="text-info">[{num}] </span>}
      {title}
    </span>
    {children}
  </div>
)

interface CalloutProps {
  title?: string
  tone?: 'accent' | 'warn'
  children: React.ReactNode
  className?: string
}

/**
 * An aside. `warn` carries the tape amber.
 *
 * The tone lives in the label, not in a coloured left rule — one ribbon per
 * card is decoration, and the label was already saying the same thing twice.
 */
export const Callout = ({ title, tone = 'accent', children, className }: CalloutProps) => (
  <div
    className={cn(
      'rounded-md border border-hairline bg-panel px-[18px] py-5 sm:px-[22px]',
      '[&>p]:leading-[25px] [&>p]:text-secondary',
      className,
    )}
  >
    {title !== undefined && (
      <div
        className={cn(
          'mb-2 text-label uppercase tracking-[var(--tracking-label)]',
          tone === 'warn' ? 'text-warning' : 'text-accent-text',
        )}
      >
        {title}
      </div>
    )}
    {children}
  </div>
)
