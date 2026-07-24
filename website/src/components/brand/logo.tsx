import { cn } from '@/lib/utils'

interface LogoProps {
  size?: number
  className?: string
}

const ASPECT = 186.407 / 211

export const Logo = ({ size = 180, className }: LogoProps) => (
  <div
    role="img"
    aria-label="soulseek-rs"
    className={cn('logo-mark shrink-0 bg-contain bg-no-repeat', className)}
    style={{ height: size, width: size * ASPECT }}
  />
)
