import { cn } from '@/lib/utils'

interface CodeProps {
  children: React.ReactNode
  className?: string
}

/** Anything typeable: flags, commands, env vars, crate names, paths. */
export const Code = ({ children, className }: CodeProps) => (
  <code
    className={cn(
      'whitespace-nowrap rounded-md border border-hairline bg-raised px-1.5 py-px font-mono text-[0.92em] text-primary',
      className,
    )}
  >
    {children}
  </code>
)
