import { ArrowRightLeft } from "lucide-react";
import { accountName, Button } from "./ui";
import { AccountProfile, SwitchSuggested } from "./types";
import { pct, t } from "./i18n";

/** Offered when the active account's quota nearly runs out and another account is clearly emptier. Never switches by itself. */
export default function SwitchSuggestion({
  suggestion,
  from,
  to,
  onSwitch,
  onDismiss,
}: {
  suggestion: SwitchSuggested;
  from: AccountProfile;
  to: AccountProfile;
  onSwitch: (acc: AccountProfile) => void;
  onDismiss: () => void;
}) {
  const toName = accountName(to).name;
  return (
    <div role="status" className="mb-6 flex items-center gap-3 rounded-xl border border-[#D97757]/40 dark:border-[#D97757]/35 bg-[#D97757]/[0.06] dark:bg-[#D97757]/10 px-4 py-3">
      <ArrowRightLeft className="h-5 w-5 shrink-0 text-[#C2603F] dark:text-[#E89A7E]" aria-hidden />
      <p className="min-w-0 flex-1 text-sm font-medium text-zinc-900 dark:text-zinc-100">
        {t("switch.suggest", {
          from: accountName(from).name,
          window: suggestion.window,
          pct: pct(suggestion.pct),
          to: toName,
          toPct: pct(suggestion.toPct),
        })}
      </p>
      <Button variant="primary" className="h-8 px-3 text-xs" onClick={() => onSwitch(to)} aria-label={t("acc.switchTo", { name: toName })}>
        {t("switch.suggestGo")}
      </Button>
      <Button className="h-8 px-3 text-xs" onClick={onDismiss}>
        {t("switch.suggestDismiss")}
      </Button>
    </div>
  );
}
