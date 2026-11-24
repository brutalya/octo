/** The Octo mark: O, a gold c inside, t, and a second o linked to the first. Ink = currentColor. */
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="-82 -82 222 164" className={className} aria-hidden="true" fill="none">
      <circle r="70" stroke="currentColor" strokeWidth="20" />
      <path d="M35.36 -35.36 A50 50 0 1 0 35.36 35.36" stroke="#d4af37" strokeWidth="20" />
      <path d="M-1.5 -28 V17 Q-1.5 27.5 10.5 27.5 M-13.5 -8.5 H12" stroke="currentColor" strokeWidth="9" />
      <circle cx="78" r="50" stroke="currentColor" strokeWidth="20" />
    </svg>
  );
}
