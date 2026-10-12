export function SvgDefs() {
  return (
    <svg className="defs" aria-hidden>
      <defs>
        <filter id="rough">
          <feTurbulence type="fractalNoise" baseFrequency="0.8" numOctaves="2" result="n" />
          <feDisplacementMap in="SourceGraphic" in2="n" scale="1.2" />
        </filter>
        <pattern id="hatch" width="8" height="8" patternUnits="userSpaceOnUse" patternTransform="rotate(35)">
          <line x1="0" y1="0" x2="0" y2="8" stroke="#222" strokeWidth="1.5" />
        </pattern>
        <pattern id="paper" width="120" height="120" patternUnits="userSpaceOnUse">
          <rect width="120" height="120" fill="#f4f1ea" />
          <circle cx="18" cy="42" r="0.8" fill="#d9d4c8" />
          <circle cx="77" cy="19" r="0.6" fill="#d9d4c8" />
          <circle cx="101" cy="88" r="0.7" fill="#d9d4c8" />
          <circle cx="44" cy="97" r="0.5" fill="#d9d4c8" />
        </pattern>
      </defs>
    </svg>
  )
}
