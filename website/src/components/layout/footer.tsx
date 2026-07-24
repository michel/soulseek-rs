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

const LINK_CLASS = 'text-[13px] text-secondary hover:text-primary'

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
  <footer className="border-t border-hairline bg-base pt-11 pb-10">
    <Wrap>
      <div className="grid grid-cols-1 gap-8 min-[520px]:grid-cols-2 md:grid-cols-[1.6fr_1fr_1fr_1fr]">
        <div className="flex max-w-[320px] flex-col gap-3.5">
          <Wordmark size={22} />
          <p className="text-[13px] leading-[22px] text-secondary">
            A Soulseek client and protocol library for Rust.
          </p>
          <p className="font-forum text-xs text-muted">
            Not affiliated with or endorsed by the Soulseek project.
          </p>
        </div>

        {COLUMNS.map((column) => (
          <div key={column.heading}>
            <h2 className="mb-3.5 text-label uppercase tracking-[var(--tracking-label)] text-secondary">
              {column.heading}
            </h2>
            <ul>
              {column.links.map((link) => (
                <li key={link.label} className="mb-2.5">
                  <FooterAnchor {...link} />
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      <div className="mt-11 flex flex-wrap items-center gap-x-6 gap-y-2 border-t border-hairline pt-5.5 text-[12.5px] text-muted">
        <span>MIT · © 2026 Michel de Graaf</span>
        <span className="text-success">No telemetry. Ever.</span>
        <span className="ml-auto">{VERSION}</span>
      </div>
    </Wrap>
  </footer>
)
