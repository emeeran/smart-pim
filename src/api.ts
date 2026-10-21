/**
 * Typed IPC bridge (Gate 3 command surface). Shapes mirror the serde structs
 * in src-tauri/src/commands.rs and the pfcore models they wrap.
 */

export interface DbStatus {
  locked: boolean;
  db_path: string;
  exists: boolean;
  title: string;
}

export interface UnlockOutcome {
  ok: boolean;
  created: boolean;
  error: string | null;
}

export interface Counts {
  notes: number;
  tasks: number;
  diary: number;
  contacts: number;
  passwords: number;
  events: number;
  messages: number;
  unread_messages: number;
  trash: number;
  pending_reminders: number;
}

export interface SearchHit {
  kind: "mail" | "message" | "task" | "note" | "diary";
  id: number;
  title: string;
  snippet: string | null;
}

export interface MessagePageRow {
  id: number;
  thread_id: number | null;
  subject: string;
  from_name: string;
  from_addr: string;
  date: string;
  flags: string;
  snippet: string;
}

export interface TaskPageRow {
  id: number;
  title: string;
  due_utc: string | null;
  priority: number;
  percent: number;
  completed_at: string | null;
}

export interface Item {
  id: number;
  kind: string;
  created_at: string;
  updated_at: string;
  deleted_at: string | null;
  private: boolean;
  list_id: number | null;
  sort_key: string | null;
}

export interface UndoOutcome {
  kind: string;
  applied: boolean;
}

export interface Account {
  id: number;
  name: string;
  kind: string;
  host: string | null;
  port: number | null;
  color: string | null;
  user: string | null;
}

export interface FolderNode {
  id: number;
  account_id: number;
  parent_id: number | null;
  name: string;
  role: string | null;
  last_seen_uid: number;
  total: number;
  unread: number;
}

export interface MessageDetail {
  id: number;
  item_id: number | null;
  account_id: number;
  folder_id: number;
  thread_id: number | null;
  subject: string;
  from_name: string;
  from_addr: string;
  to_addrs: string;
  cc_addrs: string;
  date: string;
  seen: boolean;
  body_text: string;
  /** Sanitized server-side; remote images still blocked by the UI layer. */
  body_html: string;
}

export interface NewAccount {
  name: string;
  kind: "imap" | "pop3";
  host: string;
  port: number;
  security: "none" | "starttls" | "tls";
  user: string;
  password: string;
  smtp_host: string | null;
  smtp_port: number | null;
}

export interface DraftInput {
  account_id: number;
  from_name: string;
  from_email: string;
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  text_body: string;
  html_body: string;
  send_at: string | null;
}

export interface NoteRow {
  id: number;
  item_id: number;
  title: string;
  snippet: string;
  favorite: boolean;
  updated_at: string;
}

export interface DiaryRow {
  item_id: number;
  entry_date: string;
  content: string;
  mood: string | null;
  word_count: number;
  updated_at: string;
}

export interface DiaryMonth {
  year: number;
  month: number;
  entry_count: number;
}

export interface NoteDetail {
  id: number;
  item_id: number;
  title: string;
  content: string;
  favorite: boolean;
  reminder_at: string | null;
}

type Window = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: (
    event: string,
    handler: (ev: { payload?: unknown }) => void,
  ) => Promise<() => void>;
};

function tauri(): Window {
  // The shell always runs inside Tauri; guard keeps `vite build` (which also
  // type-checks) honest and unit tests simple.
  const w = globalThis.window as unknown as { __TAURI_INTERNALS__?: Window } | undefined;
  if (!w?.__TAURI_INTERNALS__) {
    return {
      invoke: () => Promise.reject(new Error("not running inside Tauri")),
      listen: () => Promise.reject(new Error("not running inside Tauri")),
    };
  }
  return w.__TAURI_INTERNALS__;
}

const t = tauri();

export const api = {
  dbStatus: () => t.invoke("db_status") as Promise<DbStatus>,
  unlock: (passphrase: string) =>
    t.invoke("unlock", { passphrase }) as Promise<UnlockOutcome>,
  lock: () => t.invoke("lock") as Promise<void>,
  heartbeat: () => t.invoke("heartbeat") as Promise<void>,
  counts: () => t.invoke("counts") as Promise<Counts>,
  globalSearch: (query: string, limit = 50) =>
    t.invoke("global_search", { query, limit }) as Promise<SearchHit[]>,
  messagesPage: (
    folderId: number,
    cursor?: { date: string; id: number },
    limit = 100,
  ) =>
    t.invoke("messages_page", {
      folderId,
      cursorDate: cursor?.date ?? null,
      cursorId: cursor?.id ?? null,
      limit,
    }) as Promise<MessagePageRow[]>,
  tasksPage: (afterId?: number, limit = 100) =>
    t.invoke("tasks_page", { afterId: afterId ?? null, limit }) as Promise<
      TaskPageRow[]
    >,
  trash: (kind?: string, limit = 200) =>
    t.invoke("trash", { kind: kind ?? null, limit }) as Promise<Item[]>,
  trashRestore: (itemId: number) =>
    t.invoke("trash_restore", { itemId }) as Promise<void>,
  trashPurge: (itemId: number) =>
    t.invoke("trash_purge", { itemId }) as Promise<void>,
  undo: () => t.invoke("undo") as Promise<UndoOutcome | null>,
  itemLinks: (itemId: number) =>
    t.invoke("item_links", { itemId }) as Promise<
      { id: number; kind: string; title: string }[]
    >,
  itemLink: (itemId: number, otherId: number) =>
    t.invoke("item_link", { itemId, otherId }) as Promise<void>,
  itemUnlink: (itemId: number, otherId: number) =>
    t.invoke("item_unlink", { itemId, otherId }) as Promise<void>,
  tagItem: (itemId: number, tag: string) =>
    t.invoke("item_tag_add", { itemId, tag }) as Promise<void>,
  untagItem: (itemId: number, tag: string) =>
    t.invoke("item_tag_remove", { itemId, tag }) as Promise<void>,
  settingsGet: (key: string) =>
    t.invoke("settings_get", { key }) as Promise<string | null>,
  settingsSet: (key: string, value: string) =>
    t.invoke("settings_set", { key, value }) as Promise<void>,
  backupNow: () => t.invoke("backup_now") as Promise<string>,
  setTitle: (title: string) =>
    t.invoke("set_window_title", { title }) as Promise<void>,
  mailAccounts: () => t.invoke("mail_accounts") as Promise<Account[]>,
  mailAccountCreate: (account: NewAccount) =>
    t.invoke("mail_account_create", { account }) as Promise<number>,
  mailFolders: (accountId: number) =>
    t.invoke("mail_folders", { accountId }) as Promise<FolderNode[]>,
  /** Triggers a background sync; completion arrives via mail://synced. */
  mailSync: (accountId: number) =>
    t.invoke("mail_sync", { accountId }) as Promise<void>,
  messageFull: (messageId: number) =>
    t.invoke("message_full", { messageId }) as Promise<MessageDetail>,
  messageMarkRead: (messageId: number, seen: boolean) =>
    t.invoke("message_mark_read", { messageId, seen }) as Promise<void>,
  mailQueueDraft: (draft: DraftInput) =>
    t.invoke("mail_queue_draft", { draft }) as Promise<number>,
  outboxProcess: () =>
    t.invoke("outbox_process") as Promise<{ sent: number; failed: number }>,
  notesPage: (cursorDate?: string, cursorId?: number, limit = 200) =>
    t.invoke("notes_page", {
      cursorDate: cursorDate ?? null,
      cursorId: cursorId ?? null,
      limit,
    }) as Promise<NoteRow[]>,
  noteGet: (noteId: number) =>
    t.invoke("note_get", { noteId }) as Promise<NoteDetail>,
  noteCreate: (title: string, content: string) =>
    t.invoke("note_create", { title, content }) as Promise<number>,
  noteSave: (noteId: number, title: string, content: string) =>
    t.invoke("note_save", { noteId, title, content }) as Promise<void>,
  noteSetFavorite: (noteId: number, favorite: boolean) =>
    t.invoke("note_set_favorite", { noteId, favorite }) as Promise<void>,
  noteDelete: (noteId: number) =>
    t.invoke("note_delete", { noteId }) as Promise<void>,
  diaryEntries: (fromDate: string, toDate: string) =>
    t.invoke("diary_entries", { fromDate, toDate }) as Promise<DiaryRow[]>,
  diaryGet: (date: string) =>
    t.invoke("diary_get", { date }) as Promise<DiaryRow | null>,
  diarySave: (date: string, content: string, mood: string | null) =>
    t.invoke("diary_save", { date, content, mood }) as Promise<number>,
  diaryDelete: (date: string) => t.invoke("diary_delete", { date }) as Promise<void>,
  diaryMonths: (year: number) =>
    t.invoke("diary_months", { year }) as Promise<DiaryMonth[]>,
  diaryStreak: (today: string) => t.invoke("diary_streak", { today }) as Promise<number>,
  calendarsList: () => t.invoke("calendars_list") as Promise<CalendarInfo[]>,
  calendarCreate: (name: string, color: string | null) =>
    t.invoke("calendar_create", { name, color }) as Promise<number>,
  eventsWindow: (calendarId: number | null, from: string, to: string) =>
    t.invoke("events_window", { calendarId, from, to }) as Promise<Occurrence[]>,
  eventSave: (event: EventInputMsg) =>
    t.invoke("event_save", { event }) as Promise<number>,
  eventDelete: (eventId: number) =>
    t.invoke("event_delete", { eventId }) as Promise<void>,
  eventParticipants: (eventId: number) =>
    t.invoke("event_participants", { eventId }) as Promise<EventParticipant[]>,
  eventsImportIcs: (text: string) =>
    t.invoke("events_import_ics", { text }) as Promise<{ imported: number; warnings: string[] }>,
  eventsExportIcs: () => t.invoke("events_export_ics") as Promise<string>,
  tasksOpen: (limit?: number) =>
    t.invoke("tasks_open", { limit }) as Promise<TaskRow[]>,
  tasksCompleted: (limit?: number) =>
    t.invoke("tasks_completed", { limit }) as Promise<TaskRow[]>,
  tasksTree: (includeCompleted = false, limit = 500) =>
    t.invoke("tasks_tree", { includeCompleted, limit }) as Promise<TaskTreeRow[]>,
  taskSetParent: (taskId: number, parentTaskId: number | null) =>
    t.invoke("task_set_parent", { taskId, parentTaskId }) as Promise<void>,
  taskSave: (task: TaskInputMsg) =>
    t.invoke("task_save", { task }) as Promise<number>,
  taskDelete: (taskId: number) =>
    t.invoke("task_delete", { taskId }) as Promise<void>,
  contactsList: () =>
    t.invoke("contacts_list") as Promise<ContactRecord[]>,
  contactSave: (contact: ContactRecord) =>
    t.invoke("contact_save", { contact }) as Promise<number>,
  contactDelete: (contactId: number) =>
    t.invoke("contact_delete", { contactId }) as Promise<void>,
  contactsImportVcf: (text: string) =>
    t.invoke("contacts_import_vcf", { text }) as Promise<{ imported: number; warnings: string[] }>,
  contactsExportVcf: () => t.invoke("contacts_export_vcf") as Promise<string>,
  passwordsList: (limit?: number) =>
    t.invoke("passwords_list", { limit }) as Promise<PwRow[]>,
  passwordSave: (entry: PwInputMsg) =>
    t.invoke("password_save", { entry }) as Promise<number>,
  passwordDelete: (entryId: number) =>
    t.invoke("password_delete", { entryId }) as Promise<void>,
  passwordGenerate: (length?: number) =>
    t.invoke("password_generate", { length }) as Promise<string>,
  remindersTake: () =>
    t.invoke("reminders_take") as Promise<
      { id: number; item_id: number; due_at: string; missed: boolean }[]
    >,
  messageSetFlag: (messageId: number, flag: string, value: boolean) =>
    t.invoke("message_set_flag", { messageId, flag, value }) as Promise<void>,
  messageDelete: (messageId: number) =>
    t.invoke("message_delete", { messageId }) as Promise<void>,
  messagesUnified: (cursor?: { date: string; id: number }, limit = 100) =>
    t.invoke("messages_unified", {
      cursorDate: cursor?.date ?? null,
      cursorId: cursor?.id ?? null,
      limit,
    }) as Promise<MessagePageRow[]>,
  mailSearch: (query: string, filters: MailSearchFilters, limit = 50) =>
    t.invoke("mail_search", { query, ...filters, limit }) as Promise<MessagePageRow[]>,
  threadMessages: (threadId: number) =>
    t.invoke("thread_messages", { threadId }) as Promise<MessagePageRow[]>,
  messageConvert: (messageId: number, to: "task" | "note" | "event") =>
    t.invoke("message_convert", { messageId, to }) as Promise<number>,
  importCsvPreview: (text: string) =>
    t.invoke("import_csv_preview", { text }) as Promise<CsvPreview>,
  importCsvValidate: (text: string, mapping: Record<string, number>) =>
    t.invoke("import_csv_validate", { text, mapping }) as Promise<ImportReport>,
  importCsvCommit: (text: string, mapping: Record<string, number>) =>
    t.invoke("import_csv_commit", { text, mapping }) as Promise<ImportReport>,
  rulesList: () => t.invoke("rules_list") as Promise<RuleRow[]>,
  ruleCreate: (rule: {
    account_id: number | null;
    field: string;
    op: string;
    value: string;
    action: string;
    action_value: string | null;
  }) => t.invoke("rule_create", { rule }) as Promise<number>,
  ruleSetEnabled: (ruleId: number, enabled: boolean) =>
    t.invoke("rule_set_enabled", { ruleId, enabled }) as Promise<void>,
  ruleDelete: (ruleId: number) =>
    t.invoke("rule_delete", { ruleId }) as Promise<void>,
  // -------------------------------------------------- export / interchange
  exportModule: (kind: ExportKind, format: ExportFormat, path: string) =>
    t.invoke("export_module", { kind, format, path }) as Promise<ExportResult>,
  backupList: () => t.invoke("backup_list") as Promise<BackupInfo[]>,
  backupRestore: (name: string) =>
    t.invoke("backup_restore", { name }) as Promise<void>,
  // -------------------------------------------------- Wi-Fi device sync
  wifiStatus: () =>
    t.invoke("wifi_status") as Promise<{ advertising: boolean; port: number | null }>,
  wifiAdvertiseStart: () =>
    t.invoke("wifi_advertise_start") as Promise<{ port: number }>,
  wifiAdvertiseStop: () => t.invoke("wifi_advertise_stop") as Promise<void>,
  wifiPairCode: () => t.invoke("wifi_pair_code") as Promise<{ code: string }>,
  devicePair: (host: string, port: number, code: string, name: string) =>
    t.invoke("device_pair", { host, port, code, name }) as Promise<DeviceInfo>,
  devicesList: () => t.invoke("devices_list") as Promise<DeviceInfo[]>,
  deviceRemove: (deviceId: string) =>
    t.invoke("device_remove", { deviceId }) as Promise<void>,
  deviceSync: (deviceId: string) =>
    t.invoke("device_sync", { deviceId }) as Promise<SyncReport>,
  // -------------------------------------------------- cloud (DAV) sync
  davProfiles: () => t.invoke("dav_profiles") as Promise<DavProfile[]>,
  davSave: (profile: DavProfileInput) =>
    t.invoke("dav_save", { profile }) as Promise<number>,
  davRemove: (profileId: number) =>
    t.invoke("dav_remove", { profileId }) as Promise<void>,
  davSync: (profileId: number) =>
    t.invoke("dav_sync", { profileId }) as Promise<DavReport>,
  // -------------------------------------------------- AI (opt-in, local-first)
  aiStatus: () => t.invoke("ai_status") as Promise<AiStatus>,
  aiSettingsGet: () => t.invoke("ai_settings_get") as Promise<AiSettings>,
  aiSettingsSet: (patch: AiSettingsPatch) =>
    t.invoke("ai_settings_set", { patch }) as Promise<void>,
  aiRun: (kind: AiJobKind, params: Record<string, unknown>) =>
    t.invoke("ai_run", { kind, params }) as Promise<number>,
  aiJob: (jobId: number) => t.invoke("ai_job", { jobId }) as Promise<AiJobRow>,
  aiCancel: (jobId: number) => t.invoke("ai_cancel", { jobId }) as Promise<void>,
  aiReindex: () => t.invoke("ai_reindex") as Promise<number>,
  aiSearchSemantic: (query: string, limit = 20) =>
    t.invoke("ai_search_semantic", { query, limit }) as Promise<SearchHit[]>,
  freeSlots: (calendarId: number | null, from: string, to: string, durationMin: number) =>
    t.invoke("free_slots", { calendarId, from, to, durationMin }) as Promise<
      { start: string; end: string }[]
    >,
  dupeCandidates: () =>
    t.invoke("dupe_candidates") as Promise<
      { contact_ids: number[]; reason: string }[]
    >,
  mergeContacts: (primaryId: number, secondaryId: number) =>
    t.invoke("merge_contacts", { primaryId, secondaryId }) as Promise<number>,
  aiUsage: () => t.invoke("ai_usage") as Promise<AiUsage>,
  aiUsageReset: () => t.invoke("ai_usage_reset") as Promise<void>,
  /** Open a provider's app-password page (id whitelisted in the shell). */
  openProviderHelp: (provider: string) =>
    t.invoke("open_provider_help", { provider }) as Promise<void>,
  todayDashboard: (today: string) =>
    t.invoke("today_dashboard", { today }) as Promise<TodayDashboard>,
  relatedItems: (kind: string, id: number) =>
    t.invoke("related_items", { kind, id }) as Promise<RelatedItem[]>,
  listen: t.listen,
};

export type AiJobKind =
  | "summarize_note"
  | "thread_digest"
  | "draft_reply"
  | "nl_task"
  | "action_items"
  | "auto_tag"
  | "diary_digest"
  | "triage_suggest"
  | "subtask_split"
  | "signature_contact";

export interface AiSettings {
  provider: "local" | "cloud";
  endpoint: string;
  model: string;
  cloud_enabled: boolean;
  api_key_set: boolean;
  features: Record<string, boolean>;
}

export interface AiSettingsPatch {
  provider?: "local" | "cloud";
  endpoint?: string;
  model?: string;
  cloud_enabled?: boolean;
  api_key?: string | null;
  features?: Record<string, boolean>;
  /** Per-1k-token cost rates for the usage estimate (null = unset). */
  cost_per_1k_in?: number | null;
  cost_per_1k_out?: number | null;
}

export interface AiStatus extends AiSettings {
  reachable: boolean;
  models: string[];
  error: string | null;
  embedded: number;
  embeddable: number;
}

export interface AiJobRow {
  id: number;
  kind: string;
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  result: unknown;
  error: string | null;
}

/** Token/cost accounting (plan §4): counters only, never content. */
export interface AiUsage {
  jobs: number;
  tokens_in: number;
  tokens_out: number;
  by_kind: Record<string, { jobs: number; tokens_in: number; tokens_out: number }>;
  cost_per_1k_in: number | null;
  cost_per_1k_out: number | null;
  estimated_cost: number | null;
}

export type ExportKind =
  | "tasks"
  | "notes"
  | "events"
  | "contacts"
  | "diary"
  | "messages";
export type ExportFormat =
  | "csv"
  | "ics"
  | "vcf"
  | "eml"
  | "html"
  | "rtf"
  | "xml"
  | "md";

export interface ExportResult {
  path: string;
  count: number;
  format: string;
}

export interface BackupInfo {
  name: string;
  created: string;
  size: number;
}

export interface DeviceInfo {
  id: string;
  name: string;
  platform: string;
  paired_at: string;
  last_sync: string | null;
}

export interface SyncReport {
  device: string;
  pushed: number;
  pulled: number;
  conflicts: number;
}

export interface DavProfile {
  id: number;
  kind: "caldav" | "carddav" | "webdav";
  url: string;
  user: string | null;
  last_sync: string | null;
}

export interface DavProfileInput {
  id: number | null;
  kind: "caldav" | "carddav" | "webdav";
  url: string;
  user: string | null;
  password: string | null;
}

export interface DavReport {
  pulled: number;
  pushed: number;
  errors: string[];
}

export interface MailSearchFilters {
  folder_id: number | null;
  account_id: number | null;
  unread_only: boolean;
  flagged_only: boolean;
  from: string | null;
}

export interface CsvPreview {
  headers: string[];
  sample: string[][];
  total_rows: number;
  mapping: Record<string, number>;
  contact_fields: string[];
}

export interface ImportReport {
  total_rows: number;
  inserted: number;
  updated: number;
  skipped: number;
  issues: { row: number; severity: string; message: string }[];
}

export interface ContactRecord {
  uid: string | null;
  id: number;
  item_id: number;
  first_name: string;
  last_name: string;
  display_name: string;
  organization: string | null;
  emails: string[];
  phones: string[];
  addresses: string[];
  urls: string[];
  birthday: string | null;
  notes: string | null;
  categories: string[];
}

export interface PwRow {
  id: number;
  item_id: number;
  title: string;
  username: string | null;
  password: string | null;
  url: string | null;
  notes: string | null;
  strength: number | null;
}

export interface RuleRow {
  id: number;
  account_id: number | null;
  enabled: boolean;
  conditions: { field: string; op: string; value: string }[];
  actions: { type: string; folder?: string; value?: string }[];
}

export interface PwInputMsg {
  id: number | null;
  title: string;
  username: string | null;
  password: string | null;
  url: string | null;
  notes: string | null;
}

export interface TaskRow {
  id: number;
  item_id: number;
  title: string;
  notes: string | null;
  due_utc: string | null;
  priority: number;
  percent: number;
  completed_at: string | null;
}

export interface TaskTreeRow extends TaskRow {
  depth: number;
  parent_id: number | null;
}

export interface TaskInputMsg {
  id: number | null;
  title: string;
  notes: string | null;
  due_utc: string | null;
  priority: number;
  percent: number;
}

export interface CalendarInfo {
  id: number;
  name: string;
  color: string | null;
}

export interface Occurrence {
  event_id: number;
  item_id: number;
  title: string;
  start: string;
  end: string;
  all_day: boolean;
  location: string | null;
  calendar_id: number;
  calendar_color: string | null;
}

// -------------------------------------------------- today dashboard (T3.43)

export interface AccountUnread {
  id: number;
  name: string;
  unread: number;
}

export interface Anniversary {
  contact_id: number;
  name: string;
  kind: "birthday" | "anniversary";
  /** Next occurrence (YYYY-MM-DD). */
  date: string;
  days_ahead: number;
  /** Age / years being celebrated at the next occurrence. */
  years: number;
}

export interface TodayDashboard {
  events: Occurrence[];
  /** Open tasks due today or overdue. */
  tasks: TaskRow[];
  accounts: AccountUnread[];
  anniversaries: Anniversary[];
  unread_messages: number;
}

/** One cross-module related row (Related Items panel). */
export interface RelatedItem {
  kind: string; // message | task | event | note | diary | contact | other
  id: number;
  item_id: number;
  title: string;
  detail: string | null;
  when: string | null;
}

export interface EventInputMsg {
  id: number | null;
  calendar_id: number;
  title: string;
  start: string;
  end: string | null;
  all_day: boolean;
  location: string | null;
  description: string | null;
  rrule: string | null;
  reminder_minutes: number | null;
  /** Attendees; replaced wholesale on save. */
  participants: EventParticipantInput[];
}

export interface EventParticipantInput {
  contact_id: number | null;
  email: string;
}

export interface EventParticipant extends EventParticipantInput {
  status: string;
}
