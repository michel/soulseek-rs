import { CopyButton } from '@/components/ui/copy-button'
import { cn } from '@/lib/utils'

export type TermLineKind = 'cmd' | 'cm' | 'code' | 'out'

export interface TermLine {
  t: TermLineKind
  text: string
}

const LINE_TONE: Record<TermLineKind, string> = {
  cmd: '',
  cm: 'text-[var(--tm-cm)]',
  code: '',
  out: '',
}

interface TerminalProps {
  label?: string
  lines: readonly TermLine[]
  copy?: string
  className?: string
}

export const Terminal = ({ label, lines, copy, className }: TerminalProps) => {
  const copyText =
    copy ??
    lines
      .filter((line) => line.t === 'cmd')
      .map((line) => line.text)
      .join('\n')

  return (
    <div
      className={cn(
        'overflow-hidden rounded-md border border-[var(--tm-bd)] bg-[var(--tm-bg)] font-mono tabular-nums',
        className,
      )}
    >
      <div className="flex items-center justify-between border-b border-[var(--tm-bd2)] bg-[var(--tm-hd)] px-3 py-2">
        <span className="text-label uppercase tracking-[var(--tracking-label)] text-[var(--tm-dim)]">
          {label ?? 'terminal'}
        </span>
        <CopyButton text={copyText} />
      </div>
      <div className="overflow-x-auto px-4 py-4 text-[12.5px] leading-[1.75] text-[var(--tm-fg)] sm:text-[13.5px]">
        {lines.map((line, i) => (
          <span
            key={i}
            className={cn('block whitespace-pre', LINE_TONE[line.t])}
          >
            {line.t === 'cmd' && (
              <>
                <span className="text-[var(--tm-pr)]">$</span>{' '}
              </>
            )}
            {line.text}
          </span>
        ))}
      </div>
    </div>
  )
}
