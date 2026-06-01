import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

/** shadcn-style class-name merger. Tailwind classes win over earlier ones. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/** "$1,234.56" — used for balance, exposure, total cost. */
export function formatUsd(n: number | null, opts: { digits?: number } = {}): string {
  if (n === null) return '—'
  const digits = opts.digits ?? 2
  return n.toLocaleString('en-US', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  })
}

/** "0.485" — used for prices and spreads. */
export function formatPrice(n: number | null, digits = 3): string {
  if (n === null) return '—'
  return n.toFixed(digits)
}

/** "1,234" — used for counts. */
export function formatInt(n: number): string {
  return n.toLocaleString('en-US')
}

/** Shorten a condition_id / token id for display. */
export function shortHash(s: string | null, head = 6, tail = 4): string {
  if (!s) return '—'
  if (s.length <= head + tail + 1) return s
  return `${s.slice(0, head)}…${s.slice(-tail)}`
}
