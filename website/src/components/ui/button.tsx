import { cva, type VariantProps } from 'class-variance-authority'
import { Link } from 'react-router'

import { cn } from '@/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center gap-2 rounded-md border font-mono font-medium transition-[background,border-color] duration-[120ms] disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        default:
          'border-line bg-transparent text-primary hover:border-line-strong hover:bg-[var(--bg-selection)]',
        accent:
          'border-accent bg-accent text-white hover:border-accent-hover hover:bg-accent-hover',
      },
      size: {
        default: 'h-11 px-5 text-sm max-sm:h-[42px] max-sm:px-4 max-sm:text-[13px]',
        sm: 'h-8 px-2.5 text-xs',
      },
    },
    defaultVariants: { variant: 'default', size: 'default' },
  },
)

type ButtonVariants = VariantProps<typeof buttonVariants>

interface ButtonProps extends ButtonVariants {
  /** Internal route path, or an absolute URL for an off-site link. */
  href: string
  children: React.ReactNode
  className?: string
}

const isExternal = (href: string): boolean => /^https?:\/\//.test(href)

/**
 * Every call to action on the site is a link, so this renders an anchor
 * rather than a `<button>`. External links open in a new tab and carry
 * `rel="noopener"`; internal ones route through React Router so navigation
 * stays client-side.
 */
export const Button = ({ href, variant, size, children, className }: ButtonProps) => {
  const classes = cn(buttonVariants({ variant, size }), className)

  if (isExternal(href))
    return (
      <a href={href} target="_blank" rel="noopener noreferrer" className={classes}>
        {children}
      </a>
    )

  return (
    <Link to={href} className={classes}>
      {children}
    </Link>
  )
}
