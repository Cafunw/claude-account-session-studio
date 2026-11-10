import React, { useMemo, useState } from "react";
import { Folder, FolderOpen, Search, Star } from "lucide-react";
import { AccountProfile, AutomationSettings, FullDashboardPayload, SessionRecord } from "../types";
import { accountName, Card, formatDateShort, focusRing } from "../ui";

export type Selection = Pick<
  AutomationSettings,
  "transferMode" | "selectedChats" | "selectedGroups" | "selectedFolders" | "selectedTargets"
>;

const toggleIn = (list: string[], id: string) => (list.includes(id) ? list.filter((x) => x !== id) : [...list, id]);
// Same rule as `folder_key` in sync_engine.rs: Windows paths, case-insensitive, no trailing slash.
const folderKey = (path: string) => path.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
const chatFolder = (s: SessionRecord) => s.cwd || s.originCwd;
const checkboxClass = `h-4 w-4 shrink-0 cursor-pointer rounded accent-[#D97757] disabled:cursor-default disabled:opacity-40 ${focusRing}`;

export default function ChatsPage({
  data,
  accounts,
  saveSelection,
  onOpenSettings,
}: {
  data: FullDashboardPayload;
  accounts: AccountProfile[];
  saveSelection: (patch: Partial<Selection>) => void;
  onOpenSettings: () => void;
}) {
  const [searchQuery, setSearchQuery] = useState("");
  const filteredSessions = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return data.sessions;
    return data.sessions.filter((s) => s.title.toLowerCase().includes(q) || (s.cwd && s.cwd.toLowerCase().includes(q)));
  }, [data, searchQuery]);

  const sel = data.automation;
  // In "ask" mode the boxes pre-approve chats; in "all" mode everything is copied, so there is nothing to pick.
  const selecting = sel.transferMode === "ask";
  const groupIds = new Set(data.sidebarGroups.map((g) => g.id));
  const sections = data.sidebarGroups
    .map((g) => ({ g, items: filteredSessions.filter((s) => s.groupId === g.id) }))
    .filter((x) => x.items.length > 0);
  const looseSessions = filteredSessions.filter((s) => !s.groupId || !groupIds.has(s.groupId));
  // Picking groups chats outside Desktop sections by folder; chats without one stay flat.
  const folders = new Map<string, { path: string; items: SessionRecord[] }>();
  if (selecting) {
    for (const s of looseSessions) {
      const path = chatFolder(s);
      if (!path) continue;
      const f = folders.get(folderKey(path)) ?? { path, items: [] };
      f.items.push(s);
      folders.set(folderKey(path), f);
    }
  }
  const flatSessions = looseSessions.filter((s) => !selecting || !chatFolder(s));

  const withoutTarget = (key: string) => Object.fromEntries(Object.entries(sel.selectedTargets).filter(([k]) => k !== key));
  // Toggling an item also drops its account choice, so a new pick starts at "all accounts".
  const togglePick = (field: "selectedChats" | "selectedGroups" | "selectedFolders", id: string, key: string) =>
    saveSelection({ [field]: toggleIn(sel[field], id), selectedTargets: withoutTarget(key) } as Partial<Selection>);

  // Accounts a picked item goes to; a missing entry means all of them, and the last one cannot be turned off.
  const targetChips = (key: string) => {
    if (accounts.length < 2) return null;
    const all = accounts.map((a) => a.accountUuid);
    const stored = sel.selectedTargets[key];
    const on = stored ? all.filter((u) => stored.includes(u)) : all;
    const toggle = (uuid: string) => {
      const next = all.filter((u) => (u === uuid) !== on.includes(u));
      if (next.length === 0) return;
      saveSelection({
        selectedTargets: next.length === all.length ? withoutTarget(key) : { ...sel.selectedTargets, [key]: next },
      });
    };
    return (
      <span role="group" aria-label="Aktarılacak hesaplar" className="flex shrink-0 gap-1">
        {accounts.map((acc) => {
          const active = on.includes(acc.accountUuid);
          const name = accountName(acc).name;
          return (
            <button
              key={acc.accountUuid}
              type="button"
              aria-pressed={active}
              disabled={active && on.length === 1}
              onClick={() => toggle(acc.accountUuid)}
              title={active && on.length === 1 ? `${name}: en az bir hesap seçili kalmalı` : name}
              className={`max-w-[7rem] truncate rounded-md px-1.5 py-0.5 text-[11px] font-medium ring-1 ring-inset transition disabled:cursor-default ${focusRing} ${
                active
                  ? "bg-[#D97757]/10 text-[#A8492A] dark:text-[#EBA488] ring-[#D97757]/35"
                  : "bg-white dark:bg-[#17181c] text-zinc-400 ring-zinc-200 dark:ring-white/10 hover:text-zinc-700 dark:hover:text-zinc-200"
              }`}
            >
              {name}
            </button>
          );
        })}
      </span>
    );
  };

  const sessionRow = (s: SessionRecord, inSection: boolean) => {
    // A chat in a picked section or folder goes with it, so its own box shows checked and locked.
    const viaGroup = !!s.groupId && sel.selectedGroups.includes(s.groupId);
    const folder = chatFolder(s);
    const viaFolder = (!s.groupId || !groupIds.has(s.groupId)) && !!folder && sel.selectedFolders.includes(folderKey(folder));
    const own = !!s.cliSessionId && sel.selectedChats.includes(s.cliSessionId);
    const checked = viaGroup || viaFolder || own;
    return (
      <li key={`${s.accountUuid}-${s.sessionId}`} className={`flex items-center gap-3 px-4 py-2.5 ${inSection ? "pl-11" : ""}`}>
        {selecting && (
          <input
            type="checkbox"
            checked={checked}
            disabled={viaGroup || viaFolder || !s.cliSessionId}
            onChange={() => togglePick("selectedChats", s.cliSessionId!, `chat:${s.cliSessionId}`)}
            aria-label={`"${s.title}" sohbetini önceden onayla`}
            className={checkboxClass}
          />
        )}
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 truncate text-sm text-zinc-800 dark:text-zinc-200">
            {s.isStarred && <Star className="h-3 w-3 shrink-0 fill-amber-400 text-amber-400" aria-label="Yıldızlı" />}
            <span className="truncate">{s.title}</span>
          </p>
          <p className="truncate font-mono text-[11px] text-zinc-400">{s.cwd || "klasör yok"}</p>
        </div>
        {selecting && own && !viaGroup && !viaFolder && targetChips(`chat:${s.cliSessionId}`)}
        <span className="w-24 shrink-0 text-right text-xs tabular-nums text-zinc-400">{formatDateShort(s.lastActivityAt)}</span>
      </li>
    );
  };

  const groupRow = (key: string, label: React.ReactNode, count: number, checkbox: React.ReactNode, chips: React.ReactNode, title?: string) => (
    <li key={key} className="flex items-center gap-3 bg-zinc-50/70 dark:bg-white/[0.03] px-4 py-2.5" title={title}>
      {checkbox}
      {label}
      <span className="flex-1 text-xs tabular-nums text-zinc-400">{count}</span>
      {chips}
    </li>
  );

  return (
    <div className="space-y-4">
      <Card>
        <div className="flex items-center gap-3 px-4 py-3">
          <div className="relative min-w-0 flex-1">
            <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-zinc-400" aria-hidden />
            <input
              type="search"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Başlık veya klasör ara"
              aria-label="Sohbet ara"
              className="h-9 w-full rounded-lg border border-zinc-200 dark:border-white/10 bg-white dark:bg-[#17181c] pl-9 pr-3 text-sm text-zinc-900 dark:text-zinc-100 placeholder:text-zinc-400 focus:border-[#D97757] focus:outline-none focus:ring-2 focus:ring-[#D97757]/25"
            />
          </div>
          {selecting ? (
            <span className="max-w-[45%] text-pretty text-right text-[13px] leading-snug text-zinc-600 dark:text-zinc-400">
              Önceden onayla: işaretlenenler sormadan aktarılır
            </span>
          ) : (
            <span className="max-w-[50%] text-pretty text-right text-[13px] leading-snug text-zinc-600 dark:text-zinc-400">
              Tüm sohbetler otomatik aktarılıyor ·{" "}
              <button
                type="button"
                onClick={onOpenSettings}
                className={`rounded font-medium text-[#A8492A] underline-offset-2 hover:underline dark:text-[#EBA488] ${focusRing}`}
              >
                Ayarlar'dan değiştir
              </button>
            </span>
          )}
        </div>
        {selecting && (
          <p className="text-pretty border-t border-zinc-100 dark:border-white/[0.06] px-4 py-2.5 text-xs leading-relaxed text-zinc-500 dark:text-zinc-400">
            <span className="font-medium text-zinc-700 dark:text-zinc-300">
              {sel.selectedChats.length} sohbet, {sel.selectedGroups.length} bölüm, {sel.selectedFolders.length} klasör önceden onaylı.
            </span>{" "}
            İşaretlediklerin sormadan aktarılır; bir bölümü ya da klasörü işaretleyince içindeki tüm sohbetler gider. Yanındaki hesap
            etiketleri nereye gideceğini belirler. Diğer yeni sohbetler hesap değiştirirken sorulur; daha önce aktarılanlar silinmez.
          </p>
        )}
      </Card>

      <Card className="overflow-hidden">
        <ul className="divide-y divide-zinc-100 dark:divide-white/[0.06]">
          {sections.map(({ g, items }) => (
            <React.Fragment key={g.id}>
              {groupRow(
                `g-${g.id}`,
                <>
                  <Folder className="h-4 w-4 shrink-0 text-zinc-400" aria-hidden />
                  <span className="truncate text-sm font-medium text-zinc-800 dark:text-zinc-200">{g.name}</span>
                </>,
                items.length,
                selecting && (
                  <input
                    type="checkbox"
                    checked={sel.selectedGroups.includes(g.id)}
                    onChange={() => togglePick("selectedGroups", g.id, `group:${g.id}`)}
                    aria-label={`"${g.name}" bölümünü önceden onayla`}
                    className={checkboxClass}
                  />
                ),
                selecting && sel.selectedGroups.includes(g.id) && targetChips(`group:${g.id}`)
              )}
              {items.map((s) => sessionRow(s, true))}
            </React.Fragment>
          ))}
          {[...folders].map(([key, { path, items }]) => {
            const picked = sel.selectedFolders.includes(key);
            const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
            return (
              <React.Fragment key={key}>
                {groupRow(
                  `f-${key}`,
                  <>
                    <FolderOpen className="h-4 w-4 shrink-0 text-zinc-400" aria-hidden />
                    <span className="truncate font-mono text-sm text-zinc-800 dark:text-zinc-200">{name}</span>
                  </>,
                  items.length,
                  <input
                    type="checkbox"
                    checked={picked}
                    onChange={() => togglePick("selectedFolders", key, `folder:${key}`)}
                    aria-label={`"${path}" klasöründeki sohbetleri önceden onayla`}
                    className={checkboxClass}
                  />,
                  picked && targetChips(`folder:${key}`),
                  path
                )}
                {items.map((s) => sessionRow(s, true))}
              </React.Fragment>
            );
          })}
          {flatSessions.map((s) => sessionRow(s, false))}
          {filteredSessions.length === 0 && <li className="py-8 text-center text-sm text-zinc-500 dark:text-zinc-400">Sonuç yok.</li>}
        </ul>
      </Card>
    </div>
  );
}
