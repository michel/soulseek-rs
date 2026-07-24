import { Link } from 'react-router'

import { Wordmark } from '@/components/brand/wordmark'
import { Wrap } from '@/components/ui/layout'
import { LINKS, VERSION } from '@/lib/links'

interface FooterLink {
  label: string
  href: string
}

interface FooterColumn {
  heading: string
  links: readonly FooterLink[]
}

const COLUMNS: readonly FooterColumn[] = [
  {
    heading: 'Project',
    links: [
      { label: 'GitHub', href: LINKS.gh },
      { label: 'Issues', href: LINKS.issues },
      { label: 'Releases', href: LINKS.releases },
      { label: 'Changelog', href: LINKS.changelog },
      { label: 'soulfind server', href: LINKS.soulfind },
    ],
  },
  {
    heading: 'Packages',
    links: [
      { label: 'crates.io · client', href: LINKS.cratesClient },
      { label: 'crates.io · lib', href: LINKS.cratesLib },
      { label: 'docs.rs', href: LINKS.docsrs },
      { label: 'rustup', href: LINKS.rustup },
    ],
  },
  {
    heading: 'Read',
    links: [
      { label: 'Docs', href: '/docs' },
      { label: 'Install', href: '/install' },
      { label: 'Community', href: '/community' },
      { label: 'License · MIT', href: LINKS.license },
    ],
  },
]

const LINK_CLASS = 'text-[12.5px] text-secondary hover:text-primary sm:text-[13px]'

const FooterAnchor = ({ label, href }: FooterLink) =>
  href.startsWith('/') ? (
    <Link to={href} className={LINK_CLASS}>
      {label}
    </Link>
  ) : (
    <a href={href} target="_blank" rel="noopener noreferrer" className={LINK_CLASS}>
      {label}
    </a>
  )

export const Footer = () => (
  <footer className="border-t border-hairline bg-base pt-7 pb-6 sm:pt-11 sm:pb-10">
    <Wrap>
      <h2 className="sr-only">Site footer</h2>
      <div className="grid grid-cols-2 gap-x-4 gap-y-5 sm:gap-8 md:grid-cols-[1.6fr_1fr_1fr_1fr]">
        <div className="col-span-2 flex max-w-[320px] flex-col gap-2 sm:gap-3.5 md:col-span-1">
          <Wordmark size={22} />
          <p className="text-xs leading-[19px] text-secondary sm:text-[13px] sm:leading-[22px]">
            A Soulseek client and protocol library for Rust.
          </p>
          <p className="font-forum text-xs text-muted">
            Not affiliated with or endorsed by the Soulseek project.
          </p>
        </div>

        {COLUMNS.map((column) => (
          <div key={column.heading}>
            <h3 className="mb-2 text-label uppercase tracking-[var(--tracking-label)] text-secondary sm:mb-3.5">
              {column.heading}
            </h3>
            <ul>
              {column.links.map((link) => (
                <li key={link.label} className="mb-1.5 sm:mb-2.5">
                  <FooterAnchor {...link} />
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      <div className="mt-5.5 flex flex-wrap items-center gap-x-4 gap-y-1 border-t border-hairline pt-4 text-[11.5px] text-muted sm:mt-11 sm:gap-x-6 sm:gap-y-2 sm:pt-5.5 sm:text-[12.5px]">
        <span>MIT · © 2026 Michel de Graaf</span>
        <span className="text-success">No telemetry. Ever.</span>
        <span className="sm:ml-auto">{VERSION}</span>
      </div>
    </Wrap>
  </footer>
)
