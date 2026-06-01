import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

interface CardProps {
  className?: string
  children: ReactNode
}

export function Card({ className, children }: CardProps) {
  return (
    <div
      className={cn(
        'rounded-xl border border-(--color-arb-line) bg-(--color-arb-surface)/60 p-5',
        className,
      )}
    >
      {children}
    </div>
  )
}

interface CardHeaderProps {
  title: string
  hint?: string
  right?: ReactNode
}

export function CardHeader({ title, hint, right }: CardHeaderProps) {
  return (
    <div className="mb-4 flex items-baseline justify-between gap-3">
      <div>
        <h2 className="text-sm font-medium text-(--color-arb-text)">{title}</h2>
        {hint && <p className="mt-0.5 text-xs text-(--color-arb-text-faint)">{hint}</p>}
      </div>
      {right && <div className="text-xs text-(--color-arb-text-faint)">{right}</div>}
    </div>
  )
}
