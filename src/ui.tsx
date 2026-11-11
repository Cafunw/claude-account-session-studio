import React, { useEffect, useRef, useState } from "react";
import { MoreVertical } from "lucide-react";
import { AccountProfile, UsageWindow } from "./types";
import { locale, pct, t } from "./i18n";

export const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#D97757]/70 focus-visible:ring-offset-1 dark:focus-visible:ring-offset-[#17181c]";

export function formatResetTime(iso: string | null): string {
  if (!iso) return "";
  const target = new Date(iso).getTime();
  if (isNaN(target)) return "";
  const diffMs = target - Date.now();
  if (diffMs <= 0) return t("fmt.resetting");
  const mins = Math.floor(diffMs / 60000);
  const days = Math.floor(mins / (60 * 24));
  const hours = Math.floor((mins % (60 * 24)) / 60);
  if (days > 0) return t("fmt.resetsDays", { d: days, h: hours });
  if (hours > 0) return t("fmt.resetsHours", { h: hours, m: mins % 60 });
  return t("fmt.resetsMins", { m: mins % 60 });
}

export function formatAgo(iso: string | null): string {
  if (!iso) return t("fmt.never");
  const mins = Math.floor((Date.now() - new Date(iso).getTime()) / 60000);
  if (isNaN(mins)) return t("fmt.never");
  if (mins < 1) return t("fmt.justNow");
  if (mins < 60) return t("fmt.minsAgo", { n: mins });
  if (mins < 1440) return t("fmt.hoursAgo", { n: Math.floor(mins / 60) });
  return t("fmt.daysAgo", { n: Math.floor(mins / 1440) });
}

export function formatDateShort(msOrIso: number | string | null): string {
  if (!msOrIso) return "—";
  const d = new Date(msOrIso);
  if (isNaN(d.getTime())) return "—";
  return d.toLocaleDateString(locale(), { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

// Desktop-only logins have no readable e-mail; the backend fills in "account-<id>@claude.ai".
export function accountName(acc: AccountProfile): { name: string; hint: string | null } {
  const desktopOnly = acc.email.startsWith("account-") && acc.email.endsWith("@claude.ai");
  const base = desktopOnly
    ? { name: t("account.desktopOnly"), hint: acc.accountUuid.slice(0, 8) }
    : { name: acc.email, hint: null };
  // A custom name takes the title; the original identity moves to the small line.
  return acc.customLabel ? { name: acc.customLabel, hint: [base.name, base.hint].filter(Boolean).join(" ") } : base;
}

export function Card({ className = "", ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={`rounded-xl border border-zinc-200 bg-white dark:border-white/10 dark:bg-[#17181c] ${className}`} {...props} />;
}

const badgeTones = {
  outline: "border border-zinc-200 bg-white text-zinc-700 dark:border-white/10 dark:bg-transparent dark:text-zinc-300",
  green: "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-200 dark:bg-emerald-400/10 dark:text-emerald-300 dark:ring-emerald-400/25",
  sky: "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-200 dark:bg-sky-400/10 dark:text-sky-300 dark:ring-sky-400/25",
  orange: "bg-[#D97757]/10 text-[#A8492A] ring-1 ring-inset ring-[#D97757]/30 dark:bg-[#D97757]/15 dark:text-[#EBA488] dark:ring-[#D97757]/35",
};

export function Badge({ tone = "outline", children }: { tone?: keyof typeof badgeTones; children: React.ReactNode }) {
  return (
    <span className={`inline-flex shrink-0 items-center rounded-md px-1.5 py-[3px] text-[11px] font-medium leading-none ${badgeTones[tone]}`}>
      {children}
    </span>
  );
}

const buttonVariants = {
  primary: "bg-zinc-900 text-white hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white",
  outline: "border border-zinc-200 bg-white text-zinc-800 hover:bg-zinc-50 dark:border-white/10 dark:bg-white/[0.03] dark:text-zinc-200 dark:hover:bg-white/[0.08]",
  ghost: "text-zinc-600 hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-white/[0.06] dark:hover:text-zinc-100",
  danger: "bg-rose-600 text-white hover:bg-rose-700 dark:bg-rose-500 dark:hover:bg-rose-400",
};

export function Button({
  variant = "outline",
  className = "",
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: keyof typeof buttonVariants }) {
  return (
    <button
      type="button"
      className={`inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-lg px-3.5 text-sm font-medium transition active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50 disabled:active:scale-100 ${buttonVariants[variant]} ${focusRing} ${className}`}
      {...props}
    />
  );
}

export function Segmented<T extends string | number | boolean>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex shrink-0 rounded-lg border border-zinc-200 bg-zinc-50 p-0.5 dark:border-white/10 dark:bg-white/[0.03]">
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => value !== o.value && onChange(o.value)}
          className={`rounded-md px-3 py-1 text-[13px] font-medium transition ${focusRing} ${
            value === o.value ? "bg-white text-zinc-900 shadow-sm ring-1 ring-zinc-200 dark:bg-white/10 dark:text-zinc-100 dark:ring-white/10"
              : "text-zinc-500 hover:text-zinc-800 dark:text-zinc-400 dark:hover:text-zinc-200"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

const statTones = {
  sky: "border-sky-200/70 bg-sky-50 text-sky-700 dark:border-sky-400/15 dark:bg-sky-400/[0.07] dark:text-sky-300",
  green: "border-emerald-200/70 bg-emerald-50 text-emerald-700 dark:border-emerald-400/15 dark:bg-emerald-400/[0.07] dark:text-emerald-300",
  amber: "border-amber-200/70 bg-amber-50 text-amber-700 dark:border-amber-400/15 dark:bg-amber-400/[0.07] dark:text-amber-300",
  white: "border-zinc-200 bg-white text-zinc-500 dark:border-white/10 dark:bg-[#17181c] dark:text-zinc-400",
};

export function StatCard({
  label,
  value,
  icon: Icon,
  tone,
  small,
  sub,
  children,
}: {
  label: string;
  value: React.ReactNode;
  icon: React.ComponentType<{ className?: string }>;
  tone: keyof typeof statTones;
  small?: boolean;
  /** A small muted line under the value. */
  sub?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className={`flex min-w-0 items-center gap-2 rounded-xl border px-3.5 py-3.5 ${statTones[tone]}`}>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] leading-snug text-zinc-600 dark:text-zinc-400">{label}</p>
        <div className="mt-1.5 flex h-8 items-center gap-3">
          <p className={`font-semibold tabular-nums text-zinc-900 dark:text-zinc-100 ${small ? "truncate text-base" : "shrink-0 text-2xl"}`}>{value}</p>
          {children}
        </div>
        {sub && <p className="mt-0.5 truncate text-xs text-zinc-500 dark:text-zinc-400">{sub}</p>}
      </div>
      <Icon className="h-5 w-5 shrink-0" aria-hidden />
    </div>
  );
}

function barColor(pct: number): string {
  if (pct >= 90) return "bg-rose-500";
  if (pct >= 70) return "bg-amber-400";
  return "bg-emerald-600";
}

/** `pct` is the used share (drives the color); `fill` overrides the drawn width, e.g. to show what is left. */
export function Meter({ pct, label, fill = pct }: { pct: number; label: string; fill?: number }) {
  return (
    <div
      className="h-1.5 w-full overflow-hidden rounded-full bg-zinc-200/70 dark:bg-white/10"
      role="progressbar"
      aria-label={label}
      aria-valuenow={Math.round(fill)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div
        className={`h-full rounded-full transition-[width] duration-500 ${barColor(pct)}`}
        style={{ width: `${Math.min(100, Math.max(2, fill))}%` }}
      />
    </div>
  );
}

export function QuotaBar({ label, ariaLabel, win }: { label: string; ariaLabel: string; win: UsageWindow }) {
  const reset = formatResetTime(win.resetsAt);
  return (
    <div className="min-w-0 space-y-1">
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="truncate text-zinc-600 dark:text-zinc-400">{label}</span>
        <span className="font-semibold tabular-nums text-zinc-900 dark:text-zinc-100">{pct(win.utilization)}</span>
      </div>
      <Meter pct={win.utilization} label={ariaLabel} />
      {reset && <p className="truncate text-[11px] tabular-nums text-zinc-400">{reset}</p>}
    </div>
  );
}

const avatarColors = ["bg-[#C2410C]", "bg-violet-600", "bg-emerald-700", "bg-sky-700", "bg-rose-600", "bg-amber-600"];

export function Avatar({ name, seed }: { name: string; seed: string }) {
  const color = avatarColors[[...seed].reduce((s, c) => s + c.charCodeAt(0), 0) % avatarColors.length];
  return (
    <span aria-hidden className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-sm font-semibold text-white ${color}`}>
      {name.trim().charAt(0).toLocaleUpperCase(locale()) || "?"}
    </span>
  );
}

/** Kebab menu: closes on Esc (focus back to the button) and on a click outside. */
export type MenuItem = { label: string; onSelect: () => void; disabled?: boolean; danger?: boolean; title?: string };

export function Menu({ label, items }: { label: string; items: MenuItem[] }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    wrap.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus();
    const onDown = (e: MouseEvent) => !wrap.current?.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
      trigger.current?.focus();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={wrap} className="relative">
      <button
        ref={trigger}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={`flex h-8 w-8 items-center justify-center rounded-lg text-zinc-500 transition hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-white/[0.06] dark:hover:text-zinc-100 ${focusRing}`}
      >
        <MoreVertical className="h-4 w-4" />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-9 z-20 min-w-[9rem] whitespace-nowrap rounded-lg border border-zinc-200 bg-white p-1 shadow-lg dark:border-white/10 dark:bg-[#1e1f24] dark:shadow-black/40">
          {items.map((it) => (
            <button
              key={it.label}
              type="button"
              role="menuitem"
              disabled={it.disabled}
              title={it.title}
              onClick={() => {
                setOpen(false);
                it.onSelect();
              }}
              className={`block w-full rounded-md px-3 py-1.5 text-left text-sm hover:bg-zinc-100 disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent dark:hover:bg-white/[0.08] ${
                it.danger ? "text-rose-600 dark:text-rose-400" : "text-zinc-700 dark:text-zinc-200"
              } ${focusRing}`}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
