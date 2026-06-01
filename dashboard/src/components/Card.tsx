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
        'relative overflow-hidden rounded-xl border border-(--color-arb-line) bg-(--color-arb-surface)/70 p-5 shadow-lg shadow-black/20 backdrop-blur',
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
        <h2 className="text-[11px] font-medium uppercase tracking-[0.2em] text-(--color-arb-text-dim)">
          {title}
        </h2>
        {hint && <p className="mt-0.5 text-[11px] text-(--color-arb-text-faint)">{hint}</p>}
      </div>
      {right && <div className="text-[11px] text-(--color-arb-text-faint)">{right}</div>}
    </div>
  )
}
