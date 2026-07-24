import { useEffect, useRef, useState, type RefObject } from 'react'

interface FitScale {
  wrapRef: RefObject<HTMLDivElement | null>
  innerRef: RefObject<HTMLDivElement | null>
  scale: number
  height: number
  ready: boolean
}

export const useFitScale = (designWidth: number): FitScale => {
  const wrapRef = useRef<HTMLDivElement>(null)
  const innerRef = useRef<HTMLDivElement>(null)
  const [{ scale, height }, setState] = useState({ scale: 0, height: 0 })

  useEffect(() => {
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

    const observer = new ResizeObserver(measure)
    if (wrapRef.current !== null) observer.observe(wrapRef.current)
    if (innerRef.current !== null) observer.observe(innerRef.current)

    return () => {
      observer.disconnect()
    }
  }, [designWidth])

  return { wrapRef, innerRef, scale, height, ready: scale > 0 }
}
