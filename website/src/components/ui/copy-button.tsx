import { useCopy } from '@/hooks/use-copy'
import { cn } from '@/lib/utils'

interface CopyButtonProps {
  text: string
  className?: string
}

export const CopyButton = ({ text, className }: CopyButtonProps) => {
  const { copied, copy } = useCopy()

  return (
    <button
      type="button"
      onClick={() => {
        copy(text)
      }}
      className={cn(
        'inline-flex items-center gap-1.5 rounded-md border px-2 py-[3px] font-mono text-[11px] transition-colors',
        copied
          ? 'border-[var(--tm-ok)] text-[var(--tm-ok)]'
          : 'border-[var(--tm-bd)] text-[var(--tm-dim)] hover:border-[var(--tm-bd3)] hover:text-[var(--tm-fg)]',
        className,
      )}
    >
      <span aria-hidden="true">{copied ? '✓' : '⧉'}</span>
      {copied ? 'copied' : 'copy'}
    </button>
  )
}
