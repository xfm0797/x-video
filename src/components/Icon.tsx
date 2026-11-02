import type { ReactNode } from "react";

const P: Record<string, ReactNode> = {
  film: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="2.5" />
      <path d="M7 4v16M17 4v16M3 9h4M3 15h4M17 9h4M17 15h4" />
    </>
  ),
  shorts: (
    <>
      <rect x="7" y="2.5" width="10" height="19" rx="2.6" />
      <path d="M11 18.5h2" />
      <path d="M10.2 10.4l3.6 2.1-3.6 2.1z" fill="currentColor" stroke="none" />
    </>
  ),
  music: (
    <>
      <path d="M9 18V6.5l10-2v11" />
      <circle cx="6.5" cy="18" r="2.5" />
      <circle cx="16.5" cy="15.5" r="2.5" />
    </>
  ),
  folder: (
    <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5h4l2 2.5h7A2.5 2.5 0 0 1 21 10v7.5A2.5 2.5 0 0 1 18.5 20h-13A2.5 2.5 0 0 1 3 17.5z" />
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.05A1.7 1.7 0 0 0 9 19.4a1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.05A1.7 1.7 0 0 0 4.6 9a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6h.1A1.7 1.7 0 0 0 10 3.05V3a2 2 0 1 1 4 0v.05A1.7 1.7 0 0 0 15 4.6a1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.7 1.7 0 0 0 19.4 9v.1a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.05a1.7 1.7 0 0 0-1.55 1z" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="M16.5 16.5 21 21" />
    </>
  ),
  refresh: (
    <>
      <path d="M20 11a8 8 0 1 0-2.3 6.3" />
      <path d="M20 4.5V11h-6.5" />
    </>
  ),
  play: <path d="M8 5.5v13l11-6.5z" fill="currentColor" stroke="none" />,
  pause: (
    <>
      <rect x="7" y="5" width="3.6" height="14" rx="1" fill="currentColor" stroke="none" />
      <rect x="13.4" y="5" width="3.6" height="14" rx="1" fill="currentColor" stroke="none" />
    </>
  ),
  next: (
    <>
      <path d="M6 5.5v13l9-6.5z" fill="currentColor" stroke="none" />
      <path d="M18 5.5v13" />
    </>
  ),
  prev: (
    <>
      <path d="M18 5.5v13l-9-6.5z" fill="currentColor" stroke="none" />
      <path d="M6 5.5v13" />
    </>
  ),
  back10: (
    <>
      <path d="M4 10a8 8 0 1 1 1.6 6" />
      <path d="M4 4.5V10h5.5" />
    </>
  ),
  fwd10: (
    <>
      <path d="M20 10a8 8 0 1 0-1.6 6" />
      <path d="M20 4.5V10h-5.5" />
    </>
  ),
  volume: (
    <>
      <path d="M4 9.5h3l4.5-3.8v12.6L7 14.5H4z" />
      <path d="M16 9a4.5 4.5 0 0 1 0 6" />
    </>
  ),
  mute: (
    <>
      <path d="M4 9.5h3l4.5-3.8v12.6L7 14.5H4z" />
      <path d="M16 9.5l4 5M20 9.5l-4 5" />
    </>
  ),
  fullscreen: (
    <path d="M4 9V4h5M20 15v5h-5M20 9V4h-5M4 15v5h5" />
  ),
  close: <path d="M6 6l12 12M18 6 6 18" />,
  list: <path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01" />,
  chevron: <path d="M9 5l7 7-7 7" />,
  check: <path d="M4.5 12.5l5 5 10-11" />,
  plus: <path d="M12 5v14M5 12h14" />,
  trash: (
    <>
      <path d="M4 7h16M9 7V4.5h6V7M6.5 7l1 13h9l1-13" />
    </>
  ),
  spark: (
    <>
      <path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z" />
    </>
  ),
  monitor: (
    <>
      <rect x="3" y="4" width="18" height="12" rx="2" />
      <path d="M8 20h8M12 16v4" />
    </>
  ),
};

export type IconName = keyof typeof P;

export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {P[name]}
    </svg>
  );
}
