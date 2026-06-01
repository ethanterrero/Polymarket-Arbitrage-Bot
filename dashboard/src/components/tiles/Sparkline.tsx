import { useMemo } from 'react'

interface SparklineProps {
  values: number[]
  /** SVG width — pixels. */
  width?: number
  /** SVG height — pixels. */
  height?: number
  /** Stroke color (CSS var or hex). */
  color?: string
  /** Whether to fill under the line. */
  fill?: boolean
  className?: string
}

/**
 * Tiny inline area chart for KPI tiles. No external deps — just a hand-rolled
 * path so we don't pay another recharts container per tile.
 */
export function Sparkline({
  values,
  width = 96,
  height = 28,
  color = 'var(--color-arb-primary)',
  fill = true,
  className,
}: SparklineProps) {
  const { d, area } = useMemo(() => {
    if (values.length < 2) return { d: '', area: '' }
    const min = Math.min(...values)
    const max = Math.max(...values)
    const span = max - min || 1
    const step = width / (values.length - 1)
    const points = values.map((v, i) => {
      const x = i * step
      const y = height - ((v - min) / span) * height
      return [x, y] as const
    })
    const d = points.reduce(
      (acc, [x, y], i) => acc + (i === 0 ? `M${x},${y}` : ` L${x},${y}`),
      '',
    )
    const area =
      d +
      ` L${points[points.length - 1][0]},${height} L${points[0][0]},${height} Z`
    return { d, area }
  }, [values, width, height])

  if (!d) {
    return (
      <svg width={width} height={height} className={className} aria-hidden="true">
        <line
          x1={0}
          y1={height / 2}
          x2={width}
          y2={height / 2}
          stroke="var(--color-arb-line)"
          strokeDasharray="2 4"
        />
      </svg>
    )
  }

  return (
    <svg width={width} height={height} className={className} aria-hidden="true">
      {fill && <path d={area} fill={color} fillOpacity={0.18} />}
      <path d={d} fill="none" stroke={color} strokeWidth={1.5} strokeLinecap="round" />
    </svg>
  )
}
