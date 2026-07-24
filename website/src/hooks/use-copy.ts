import { useCallback, useEffect, useRef, useState } from 'react'

const RESET_MS = 1400

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
        setCopied(false)
      },
    )
  }, [])

  return { copied, copy }
}
