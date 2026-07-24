import { useEffect, useRef, useState, type RefObject } from 'react'

interface FitScale {
  wrapRef: RefObject<HTMLDivElement | null>
  innerRef: RefObject<HTMLDivElement | null>
  scale: number
  height: number
  ready: boolean
}

/**
 * Scales a fixed-width design down to whatever width it is given.
 *
 * The TUI is laid out on a 1360px character grid; reflowing it would stop it
 * looking like a terminal. So it renders at native width and gets a CSS
 * transform, and the wrapper is told the resulting height so the page below
 * does not sit under it.
 */
export const useFitScale = (designWidth: number): FitScale => {
  const wrapRef = useRef<HTMLDivElement>(null)
  const innerRef = useRef<HTMLDivElement>(null)
  const [{ scale, height }, setState] = useState({ scale: 0, height: 0 })

  useEffect(() => {
    // ResizeObserver is browser-only; prerendering never reaches this effect.
    if (typeof ResizeObserver === 'undefined') return

    const measure = () => {
      const wrap = wrapRef.current
      const inner = innerRef.current
      if (wrap === null || inner === null) return

      const width = wrap.clientWidth
      if (width === 0) return

      const next = Math.min(1, width / designWidth)
      const nextHeight = inner.offsetHeight * next
      if (nextHeight === 0) return

      setState((prev) =>
        Math.abs(prev.scale - next) < 0.001 && Math.abs(prev.height - nextHeight) < 0.5
          ? prev
          : { scale: next, height: nextHeight },
      )
    }

    measure()

    // Both boxes matter: the wrapper for available width, the inner for the
    // height that width produces once the webfont has swapped in.
    const observer = new ResizeObserver(measure)
    if (wrapRef.current !== null) observer.observe(wrapRef.current)
    if (innerRef.current !== null) observer.observe(innerRef.current)

    return () => {
      observer.disconnect()
    }
  }, [designWidth])

  return { wrapRef, innerRef, scale, height, ready: scale > 0 }
}
