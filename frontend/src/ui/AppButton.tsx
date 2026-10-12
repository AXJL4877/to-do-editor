import type { ReactNode } from 'react'

export function AppButton({
  children,
  onClick,
  disabled,
  variant = 'default',
}: {
  children: ReactNode
  onClick?: () => void
  disabled?: boolean
  variant?: 'default' | 'primary' | 'ghost'
}) {
  return (
    <button
      type="button"
      className={`app-btn app-btn-${variant}`}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  )
}
