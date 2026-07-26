interface ExtLinkProps {
  href: string
  children: React.ReactNode
}

export const ExtLink = ({ href, children }: ExtLinkProps) => (
  <a
    href={href}
    target="_blank"
    rel="noopener noreferrer"
    className="text-link hover:text-link-hover"
  >
    {children}
  </a>
)
