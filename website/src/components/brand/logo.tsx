import { cn } from '@/lib/utils'

interface LogoProps {
  /** Height in px. Width follows the artwork's aspect ratio. */
  size?: number
  className?: string
}

/** Native aspect ratio of the logo artwork (186.407 × 211). */
const ASPECT = 186.407 / 211

/*
 * Two assets rather than one recoloured file — the ink and light cuts differ
 * in more than colour — swapped by CSS.
 *
 * Background images, not <img>: a hidden <img> is still downloaded, so the
 * two-element version fetched both cuts on every visit (~12-23 kB gzip wasted
 * above the fold). A background on a non-painted rule is never requested.
 */
export const Logo = ({ size = 180, className }: LogoProps) => (
  <div
    role="img"
    aria-label="soulseek-rs"
    className={cn('logo-mark shrink-0 bg-contain bg-no-repeat', className)}
    style={{ height: size, width: size * ASPECT }}
  />
)
