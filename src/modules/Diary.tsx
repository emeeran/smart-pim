/**
 * Diary module: year/month navigator | entry list | per-day editor with
 * debounced autosave and mood. Dates are LOCAL calendar dates (YYYY-MM-DD)
 * end to end — converting through UTC shifts entries a day.
 */
import { useEffect, useRef, useState } from "preact/hooks";
import { api, type DiaryMonth, type DiaryRow } from "../api";
import { ThreePane } from "../shell/ThreePane";
import { useToolbar } from "../shell/toolbar";
import { DetailTabs } from "../components/DetailTabs";
import { RichEditor } from "../components/RichEditor";
import { VirtualList } from "../components/VirtualList";
import { AiPreview } from "../components/AiPreview";
import { errText, featureEnabled, resultText, runAiJob, useAiStatus } from "../lib/ai";
import { refreshCounts, viewCount } from "../store";

const ROW_H = 56;

interface ReflectionState {
  open: boolean;
  body: string;
  busy: boolean;
  error: string | null;
}

const REFLECTION_IDLE: ReflectionState = { open: false, body: "", busy: false, error: null };

/** Local date string for a Date — no UTC conversion anywhere. */
function localYmd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate(),
  ).padStart(2, "0")}`;
}

const MONTH_NAMES = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

export function DiaryModule() {
  const today = localYmd(new Date());
  const [year, setYear] = useState(new Date().getFullYear());
  const [selectedDate, setSelectedDate] = useState<string>(today);
  const [months, setMonths] = useState<DiaryMonth[]>([]);
  const [streak, setStreak] = useState(0);
  const [entries, setEntries] = useState<DiaryRow[]>([]);
  const [detail, setDetail] = useState<DiaryRow | null>(null);
  const [content, setContent] = useState("");
  const [mood, setMood] = useState("");
  const [dirty, setDirty] = useState(false);
  const saveTimer = useRef<number | null>(null);

  const reloadMonths = (y: number) => {
    void api.diaryMonths(y).then(setMonths).catch(() => setMonths([]));
    void api.diaryStreak(today).then(setStreak).catch(() => setStreak(0));
  };

  useEffect(() => {
    reloadMonths(year);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [year]);

  useEffect(() => {
    const from = `${year}-01-01`;
    const to = `${year}-12-31`;
    void api
      .diaryEntries(from, to)
      .then(setEntries)
      .catch(() => setEntries([]));
  }, [year, dirty]);

  useEffect(() => {
    viewCount.value = entries.length;
  }, [entries.length]);

  const openDate = (date: string) => {
    setSelectedDate(date);
    setDirty(false);
    void api.diaryGet(date).then((row) => {
      setDetail(row);
      setContent(row?.content ?? "");
      setMood(row?.mood ?? "");
    });
  };

  useEffect(() => {
    openDate(today);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const scheduleSave = (nextContent: string, nextMood: string) => {
    setDirty(true);
    if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(async () => {
      await api.diarySave(selectedDate, nextContent, nextMood || null);
      setDirty(false);
      reloadMonths(year);
      void refreshCounts();
    }, 900);
  };

  const deleteEntry = async () => {
    if (!confirm(`Move the ${selectedDate} entry to trash?`)) return;
    await api.diaryDelete(selectedDate);
    setDetail(null);
    setContent("");
    setMood("");
    setDirty(false);
    reloadMonths(year);
    void refreshCounts();
  };

  const jump = (days: number) => {
    const d = new Date(`${selectedDate}T12:00:00`); // noon avoids DST edges
    d.setDate(d.getDate() + days);
    openDate(localYmd(d));
  };

  // ------------------------------------------------------- AI reflection
  // Diary is the most sensitive text in the app, so this is its own opt-in
  // (ai_integ_plan §3): the digest is preview-only, never written into an
  // entry, and the button is absent unless the feature is enabled.
  const status = useAiStatus();
  const canReflect = featureEnabled(status, "diary");
  const [reflection, setReflection] = useState<ReflectionState>(REFLECTION_IDLE);

  const runReflection = async () => {
    const from = `${year}-01-01`;
    const to = `${year}-12-31`;
    setReflection({ open: true, body: "", busy: true, error: null });
    try {
      const text = resultText(await runAiJob("diary_digest", { from, to }));
      setReflection({ open: true, body: text, busy: false, error: null });
    } catch (e) {
      setReflection({ open: true, body: "", busy: false, error: errText(e) });
    }
  };

  useToolbar(
    <div class="mail-toolbar">
      <button class="primary" onClick={() => openDate(today)}>
        Today
      </button>
      <button onClick={() => jump(-1)}>‹ Prev day</button>
      <button onClick={() => jump(1)}>Next day ›</button>
      <span class="muted">{dirty ? "Saving…" : "Saved"}</span>
      {canReflect && (
        <button
          title={`Reflect over the entries shown (${year})`}
          onClick={() => void runReflection()}
        >
          Weekly Reflection
        </button>
      )}
      {detail && (
        <button class="danger-text" onClick={() => void deleteEntry()}>
          Delete Entry
        </button>
      )}
    </div>,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [detail, selectedDate, dirty, canReflect, year],
  );

  const monthCounts = new Map(months.map((m) => [m.month, m.entry_count]));
  const yearEntries = [...entries].sort((a, b) => (a.entry_date < b.entry_date ? 1 : -1));

  return (
    <div class="diary-module">
      <ThreePane
        module="diary"
        tree={
          <nav class="diary-tree" aria-label="Diary">
            <div class="diary-year">
              <button onClick={() => setYear((y) => y - 1)} aria-label="Previous year">‹</button>
              <span>{year}</span>
              <button onClick={() => setYear((y) => y + 1)} aria-label="Next year">›</button>
            </div>
            {streak > 0 && <div class="diary-streak">{streak}-day streak</div>}
            <button class="smartlist-entry" data-active={selectedDate === today} onClick={() => openDate(today)}>
              <span>Today</span>
            </button>
            {MONTH_NAMES.map((name, i) => {
              const month = i + 1;
              const count = monthCounts.get(month) ?? 0;
              if (count === 0) return null;
              return (
                <button
                  key={month}
                  class="smartlist-entry"
                  data-active={selectedDate.startsWith(`${year}-${String(month).padStart(2, "0")}`)}
                  onClick={() => openDate(`${year}-${String(month).padStart(2, "0")}-01`)}
                >
                  <span>{name}</span>
                  <span class="smartlist-count">{count}</span>
                </button>
              );
            })}
          </nav>
        }
        list={
          <div class="diary-list">
            {yearEntries.length === 0 ? (
              <p class="muted empty-pane">No entries in {year} yet.</p>
            ) : (
              <VirtualList
                rowCount={yearEntries.length}
                rowHeight={ROW_H}
                renderRow={(i) => {
                  const e = yearEntries[i];
                  const preview = e.content.replace(/<[^>]*>/g, " ").trim().slice(0, 80);
                  return (
                    <div
                      key={e.entry_date}
                      class="note-card diary-day"
                      data-selected={e.entry_date === selectedDate}
                      style={{ height: `${ROW_H}px` }}
                      onClick={() => openDate(e.entry_date)}
                    >
                      <span class="note-card-main">
                        <span class="note-card-title">{e.entry_date}</span>
                        <span class="note-card-snippet">{preview || "—"}</span>
                      </span>
                      <span class="muted">{e.word_count}w</span>
                    </div>
                  );
                }}
              />
            )}
          </div>
        }
        detail={
          <DetailTabs
            itemId={detail?.item_id ?? null}
            canEdit={detail !== null}
            notes={
              <div class="note-editor-pane">
                <div class="note-editor-header">
                  <span class="diary-date-header">{selectedDate}</span>
                  <input
                    class="diary-mood-input"
                    placeholder="Mood"
                    value={mood}
                    onInput={(e) => {
                      const v = (e.target as HTMLInputElement).value;
                      setMood(v);
                      scheduleSave(content, v);
                    }}
                  />
                </div>
                <RichEditor
                  key={selectedDate}
                  html={content}
                  onChange={(next) => {
                    setContent(next);
                    scheduleSave(next, mood);
                  }}
                />
              </div>
            }
          />
        }
      />
      <AiPreview
        open={reflection.open}
        title={`Reflection — ${year}`}
        onClose={() => setReflection(REFLECTION_IDLE)}
        body={reflection.body}
        busy={reflection.busy}
        busyLabel="Reflecting over your entries…"
        error={reflection.error}
        actions={[
          {
            label: "Copy",
            disabled: !reflection.body,
            onClick: () => void navigator.clipboard?.writeText(reflection.body),
          },
        ]}
      >
        <div class="ai-note">
          Preview only — this reflection is never written into a diary entry.
        </div>
      </AiPreview>
    </div>
  );
}
