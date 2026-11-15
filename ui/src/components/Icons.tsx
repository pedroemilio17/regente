// Inline icons from the design canvas.

const base = {
  width: 18,
  height: 18,
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.8,
  'aria-hidden': true,
} as const;

export const PlusIcon = () => (
  <svg {...base} strokeLinecap="round">
    <path d="M12 5v14M5 12h14" />
  </svg>
);

export const SearchIcon = () => (
  <svg {...base} strokeLinecap="round">
    <circle cx="11" cy="11" r="6.5" />
    <path d="M20 20l-4-4" />
  </svg>
);

export const BoltIcon = () => (
  <svg {...base} strokeLinejoin="round">
    <path d="M13 3L5 13h6l-1 8 8-10h-6l1-8z" />
  </svg>
);

export const PluginIcon = () => (
  <svg {...base} strokeLinejoin="round">
    <path d="M9 4h6v4h4v6h-4v6H9v-6H5V8h4z" />
  </svg>
);

export const ProjectIcon = () => (
  <svg {...base} strokeLinejoin="round">
    <path d="M4 6h16v4H4zM5 10v9h14v-9M10 14h4" />
  </svg>
);

export const SlidersIcon = () => (
  <svg {...base} stroke="var(--muted)" strokeLinecap="round">
    <path d="M4 7h9M17 7h3M4 17h3M11 17h9" />
    <circle cx="15" cy="7" r="2" />
    <circle cx="9" cy="17" r="2" />
  </svg>
);

export const Logo = () => (
  <svg width="22" height="22" viewBox="0 0 16 16" shapeRendering="crispEdges" aria-hidden="true">
    <rect x="2" y="2" width="12" height="12" fill="var(--warning)" />
    <rect x="5" y="6" width="2" height="2" fill="var(--app)" />
    <rect x="9" y="6" width="2" height="2" fill="var(--app)" />
    <rect x="5" y="10" width="6" height="1" fill="var(--app)" />
  </svg>
);
