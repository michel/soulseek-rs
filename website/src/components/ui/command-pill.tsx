import { useCopy } from '@/hooks/use-copy'
import { cn } from '@/lib/utils'

interface CommandPillProps {
  text: string
  className?: string
}

/** The one-line install command, click-to-copy. */
export const CommandPill = ({ text, className }: CommandPillProps) => {
  const { copied, copy } = useCopy()

  return (
    <button
      type="button"
      title="Copy command"
      onClick={() => {
        copy(text)
      }}
      className={cn(
        'inline-flex h-11 items-center gap-2.5 rounded-md border border-line bg-panel py-0 pr-2.5 pl-3.5 font-mono text-[13.5px] tabular-nums text-primary hover:border-line-strong hover:bg-raised max-sm:w-full max-sm:justify-between',
        className,
      )}
    >
      <span aria-hidden="true" className="text-accent">
        $
      </span>
      <code className="font-mono">{text}</code>
      <span
        aria-hidden="true"
        className={cn(
          'border-l border-hairline pl-2.5 text-[11px]',
          copied ? 'text-success' : 'text-secondary',
        )}
      >
        {copied ? '✓ copied' : '⧉ copy'}
      </span>
      <span className="sr-only" aria-live="polite">
        {copied ? 'Copied to clipboard' : ''}
      </span>
    </button>
  )
}
