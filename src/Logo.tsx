// Same artwork as src-tauri/icons/logo.svg: two chat bubbles in turn, a chat carried between accounts.
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 256 256" className={className} aria-hidden>
      <rect x="4" y="4" width="248" height="248" rx="60" fill="#17181c" />
      <rect x="100" y="36" width="122" height="100" rx="38" fill="#F4E4DA" />
      <path d="M178 40L232 22L222 86z" fill="#F4E4DA" />
      <rect x="26" y="96" width="140" height="116" rx="46" fill="#17181c" />
      <rect x="34" y="104" width="124" height="100" rx="38" fill="#D97757" />
      <path d="M36 166L26 232L96 204z" fill="#D97757" />
    </svg>
  );
}
