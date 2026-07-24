import { useCallback, useEffect, useRef, useState } from 'react'

const RESET_MS = 1400

/**
 * Copy-to-clipboard with a self-clearing "copied" flag.
 *
 * ponytail: no execCommand fallback. `navigator.clipboard` is available in
 * every secure context, and the site only ever ships over HTTPS (GitHub
 * Pages) or localhost. If a copy does fail, the flag stays off and the text
 * is still selectable by hand.
 */
export const useCopy = (): { copied: boolean; copy: (text: string) => void } => {
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current)
    },
    [],
  )

  const copy = useCallback((text: string) => {
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true)
        if (timer.current !== null) clearTimeout(timer.current)
        timer.current = setTimeout(() => {
          setCopied(false)
        }, RESET_MS)
      },
      () => {
        // Permission denied or no clipboard: leave the button untouched.
      },
    )
  }, [])

  return { copied, copy }
}
