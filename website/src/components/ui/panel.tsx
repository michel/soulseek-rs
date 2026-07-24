import { cn } from '@/lib/utils'

interface PanelProps {
  num?: number | undefined
  title: string
  children: React.ReactNode
  className?: string
}

export const Panel = ({ num, title, children, className }: PanelProps) => (
  <div
    className={cn(
      'relative rounded-md border border-hairline bg-panel px-[18px] py-[22px] sm:px-6 sm:py-[26px]',
      className,
    )}
  >
    <span className="absolute -top-2.5 left-4 bg-base px-2 text-label uppercase tracking-[var(--tracking-label)] text-secondary">
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
        {tone === 'warn' && <span aria-hidden="true">! </span>}
        {title}
      </div>
    )}
    {children}
  </div>
)
