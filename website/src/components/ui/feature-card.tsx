import { useCopy } from '@/hooks/use-copy'
import { cn } from '@/lib/utils'

interface FeatureCommandProps {
  text: string
}

const FeatureCommand = ({ text }: FeatureCommandProps) => {
  const { copied, copy } = useCopy()

  return (
    <button
      type="button"
      title="Copy command"
      onClick={() => {
        copy(text)
      }}
      className={cn(
        'mt-auto flex w-full items-center gap-2 rounded-md border bg-[var(--cmd-bg)] px-2.5 py-2 text-left font-mono text-[11.5px] leading-[1.5] text-[var(--tm-fg)]',
        copied
          ? 'border-[var(--tm-ok)]'
          : 'border-[var(--tm-bd)] hover:border-[var(--tm-bd3)]',
      )}
    >
      <span aria-hidden="true" className="shrink-0 text-[var(--tm-pr)]">
        $
      </span>
      <code className="grow font-mono break-words whitespace-pre-wrap">{text}</code>
      <span
        aria-hidden="true"
        className={cn('shrink-0', copied ? 'text-[var(--tm-ok)]' : 'text-[var(--tm-dim)]')}
      >
        {copied ? '✓' : '⧉'}
      </span>
    </button>
  )
}

interface FeatureCardProps {
  glyph?: string
  color?: string
  title: string
  children: React.ReactNode
  cmd?: string
  className?: string
}

export const FeatureCard = ({
  glyph,
  color,
  title,
  children,
  cmd,
  className,
}: FeatureCardProps) => (
  <div
    className={cn(
      'flex flex-col gap-2.5 rounded-md border border-hairline bg-panel p-[18px] transition-[border-color,background] duration-[120ms] hover:border-line hover:bg-raised sm:p-[22px]',
      className,
    )}
  >
    <div className="flex items-center gap-2.5">
      {glyph !== undefined && (
        <span
          aria-hidden="true"
          className="shrink-0 text-lg leading-none"
          style={color !== undefined ? { color } : undefined}
        >
          {glyph}
        </span>
      )}
      <div className="text-[15px] font-medium text-primary">{title}</div>
    </div>
    <div className="text-[13px] leading-[21px] text-pretty text-secondary [&_code]:font-mono [&_code]:text-primary">
      {children}
    </div>
    {cmd !== undefined && <FeatureCommand text={cmd} />}
  </div>
)
