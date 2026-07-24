import { cn } from '@/lib/utils'

interface WrapProps {
  children: React.ReactNode
  className?: string
}

export const Wrap = ({ children, className }: WrapProps) => (
  <div className={cn('mx-auto w-full max-w-[1140px] px-[18px] sm:px-7', className)}>
    {children}
  </div>
)

interface SectionProps {
  children: React.ReactNode
  flush?: boolean
  band?: boolean
  id?: string
  className?: string
}

export const Section = ({ children, flush, band, id, className }: SectionProps) => (
  <section
    id={id}
    className={cn(
      'py-6 pb-10 sm:pt-[30px] sm:pb-13',
      band
        ? 'border-y border-hairline bg-panel'
        : !flush && 'border-t border-hairline',
      className,
    )}
  >
    <Wrap>{children}</Wrap>
  </section>
)

interface EyebrowProps {
  children: React.ReactNode
  className?: string
}

export const Eyebrow = ({ children, className }: EyebrowProps) => (
  <span
    className={cn(
      'text-label uppercase tracking-[var(--tracking-label)] text-accent-text',
      className,
    )}
  >
    {children}
  </span>
)

interface SectionHeadProps {
  eyebrow?: string
  title: React.ReactNode
  children?: React.ReactNode
  className?: string
}

export const SectionHead = ({
  eyebrow,
  title,
  children,
  className,
}: SectionHeadProps) => (
  <div className={cn('mb-11 flex max-w-[660px] flex-col gap-3.5', className)}>
    {eyebrow !== undefined && <Eyebrow>{eyebrow}</Eyebrow>}
    <h2 className="text-[26px] leading-[34px] font-medium tracking-[-0.005em] text-balance sm:text-title sm:leading-[var(--text-title--line-height)]">
      {title}
    </h2>
    {children !== undefined && (
      <p className="text-base leading-[26px] text-pretty text-secondary">{children}</p>
    )}
  </div>
)

interface PageHeadProps {
  eyebrow: string
  title: string
  children: React.ReactNode
}

export const PageHead = ({ eyebrow, title, children }: PageHeadProps) => (
  <Section flush className="pt-10">
    <div className="flex max-w-[660px] flex-col gap-3.5">
      <Eyebrow>{eyebrow}</Eyebrow>
      <h1 className="text-[26px] leading-[34px] font-medium tracking-[-0.005em] text-balance sm:text-title sm:leading-[var(--text-title--line-height)]">
        {title}
      </h1>
      <p className="text-base leading-[26px] text-pretty text-secondary">{children}</p>
    </div>
  </Section>
)

interface ColsProps {
  children: React.ReactNode
  start?: boolean
  center?: boolean
  className?: string
}

export const Cols = ({ children, start, center, className }: ColsProps) => (
  <div
    className={cn(
      'grid grid-cols-1 gap-7 md:grid-cols-2 md:gap-10',
      start === true && 'items-start',
      center === true && 'items-center',
      className,
    )}
  >
    {children}
  </div>
)

interface ProseProps {
  children: React.ReactNode
  className?: string
}

export const Prose = ({ children, className }: ProseProps) => (
  <div
    className={cn(
      '[&>p+p]:mt-4 [&>p]:leading-[26px] [&>p]:text-pretty [&>p]:text-secondary',
      className,
    )}
  >
    {children}
  </div>
)
