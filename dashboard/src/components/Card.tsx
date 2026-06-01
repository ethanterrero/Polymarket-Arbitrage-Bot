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
        'rounded-xl border border-zinc-800 bg-zinc-900/60 p-5 shadow-lg shadow-black/20 backdrop-blur',
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
        <h2 className="text-sm font-medium uppercase tracking-wider text-zinc-400">
          {title}
        </h2>
        {hint && <p className="mt-0.5 text-xs text-zinc-500">{hint}</p>}
      </div>
      {right && <div className="text-xs text-zinc-500">{right}</div>}
    </div>
  )
}
