import { cn } from '@/lib/utils'

interface WordmarkProps {
  /** Font size in px. */
  size?: number
  blink?: boolean
  className?: string
}

/**
 * The primary soulseek-rs mark: lowercase mono wordmark plus a solid block
 * cursor. Live text rather than an asset, so it blinks for free and stays
 * crisp at any size.
 */
export const Wordmark = ({ size = 28, blink = false, className }: WordmarkProps) => (
  <span
    className={cn(
      'inline-flex items-center whitespace-nowrap font-mono font-medium leading-none tracking-[-0.01em] text-primary',
      className,
    )}
    style={{ fontSize: size }}
  >
    soulseek-rs
    <span
      aria-hidden="true"
      className={cn(
        'ml-[0.06em] inline-block h-[1.02em] w-[0.58em] translate-y-[0.04em] rounded-[1.5px] bg-accent',
        blink && 'motion-safe:animate-cursor-blink',
      )}
    />
  </span>
)
