import { NavLink } from 'react-router'

import { Wordmark } from '@/components/brand/wordmark'
import { ThemeToggle } from '@/components/layout/theme-toggle'
import { LINKS } from '@/lib/links'
import { cn } from '@/lib/utils'

const ITEMS = [
  { label: 'home', to: '/' },
  { label: 'install', to: '/install' },
  { label: 'docs', to: '/docs' },
  { label: 'community', to: '/community' },
] as const

export const Nav = () => (
  <header className="sticky top-0 z-50 border-b border-hairline bg-[color-mix(in_srgb,var(--bg-base)_88%,transparent)] backdrop-blur-[8px]">
    <div className="mx-auto flex max-w-[1140px] flex-wrap items-center gap-x-4 gap-y-3 px-[18px] pt-2.5 sm:h-[60px] sm:flex-nowrap sm:gap-6 sm:px-7 sm:pt-0">
      <NavLink
        to="/"
        aria-label="soulseek-rs home"
        className="mr-auto flex min-h-11 items-center"
      >
        <Wordmark size={19} blink />
      </NavLink>

      <nav
        aria-label="Main"
        className="order-3 flex w-full items-center justify-center gap-0.5 overflow-x-auto border-t border-hairline py-1 [-ms-overflow-style:none] [scrollbar-width:none] sm:order-none sm:w-auto sm:justify-start sm:border-t-0 sm:py-0 [&::-webkit-scrollbar]:hidden"
      >
        {ITEMS.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === '/'}
            className={({ isActive }) =>
              cn(
                'relative inline-flex shrink-0 items-center rounded-md px-3.5 py-1.5 text-label uppercase tracking-[var(--tracking-label)] max-sm:min-h-11 sm:px-3',
                isActive
                  ? "bg-raised text-primary before:text-accent-text before:content-['›_'] after:absolute after:inset-x-3 after:bottom-px after:h-0.5 after:bg-accent"
                  : 'text-secondary hover:bg-[var(--bg-selection)] hover:text-primary',
              )
            }
          >
            {item.label}
          </NavLink>
        ))}
      </nav>

      <div className="flex items-center gap-2">
        <a
          href={LINKS.gh}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex h-[38px] items-center gap-1.5 rounded-md border border-hairline px-2.5 font-mono text-xs text-secondary hover:border-line hover:bg-[var(--bg-selection)] hover:text-primary sm:h-8"
        >
          GitHub
          <span aria-hidden="true" className="text-[0.85em] opacity-60">
            ↗
          </span>
        </a>
        <ThemeToggle />
      </div>
    </div>
  </header>
)
