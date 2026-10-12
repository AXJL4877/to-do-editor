import type { ReactNode } from 'react'

export function SketchButton({
  children,
  onClick,
  disabled,
}: {
  children: ReactNode
  onClick?: () => void
  disabled?: boolean
}) {
  return (
    <button type="button" className="sketch-btn" disabled={disabled} onClick={onClick}>
      <svg className="btn-frame" viewBox="0 0 120 36" preserveAspectRatio="none" aria-hidden>
        <path
          d="M3.2,4.1 L116.5,2.8 L117.8,32.4 L2.4,33.6 Z"
          fill="#fff"
          stroke="#1a1a1a"
          strokeWidth="2"
          filter="url(#rough)"
        />
      </svg>
      <span>{children}</span>
    </button>
  )
}
