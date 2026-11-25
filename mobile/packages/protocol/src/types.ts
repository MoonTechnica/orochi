// Mirrors the serialized structs in src/activity.rs and desktop/src-tauri/src/view.rs.
// Field names are the wire names (snake_case); timestamps are milliseconds unless noted.

export type ThreadStatus =
  | 'needs_you'
  | 'working'
  | 'background'
  | 'queued'
  | 'held'
  | 'interrupted'
  | 'failed'
  | 'unread'
  | 'idle';

export interface Overrides {
  agent: string | null;
  model: string | null;
  reasoning: string | null;
  mode: string | null;
}

export interface AgentUse {
  role: string;
  agent: string;
  provider: string;
  model: string;
  seats: number;
  live: number;
  lead: boolean;
  background: boolean;
  last_at: number;
  manager?: string | null;
}

export interface ThreadRow {
  id: string;
  project: string;
  project_id: string;
  project_root: string;
  title: string;
  cwd: string;
  branch: string | null;
  origin: string;
  worktree: boolean;
  pinned: boolean;
  archived: boolean;
  updated_at: number;
  status: ThreadStatus;
  seats: number;
  asking: number;
  unread: number;
  held_until?: number | null;
  terminal: boolean;
  doing: string | null;
  background: number;
  running_since: number | null;
  overrides: Overrides;
  agent: string | null;
  provider: string | null;
  agents: AgentUse[];
  /** Added by the window's sidebar view; absent in older recordings. */
  mentions?: number;
}

export interface ProjectRow {
  id: string;
  name: string;
  root: string;
  pinned: boolean;
  collapsed: boolean;
  missing: boolean;
  branch?: string;
  threads: ThreadRow[];
}

export interface ItemRow {
  seq: number;
  turn: string;
  turn_ordinal: number;
  lane: number | null;
  role: string | null;
  agent: string | null;
  model: string | null;
  kind: string;
  status: string | null;
  text: string;
  data: Record<string, any> | null;
  truncated: boolean;
  patches: number;
  at: number;
  failed: boolean;
  turn_origin: string | null;
  provider: string | null;
  key?: string | null;
  /** The member whose thread the item belongs to (team channel). */
  member?: string | null;
}

export interface SeatRow {
  seat_id: string;
  turn: string;
  ordinal: number;
  role: string;
  lead: boolean;
  read_only: boolean;
  phase: string | null;
  state: string;
  agent: string | null;
  model: string | null;
  reasoning: string | null;
  outcome: string | null;
  peer: string | null;
  peer_status: string | null;
  doing: string | null;
  title: string | null;
  background: boolean;
  reported: boolean;
  origin?: string | null;
  member?: string | null;
}

export interface FileRow {
  turn: string;
  path: string;
  change: string;
  added: number;
  removed: number;
  latest_patch: number;
}

export interface Thread {
  thread: ThreadRow;
  running_turn?: string | null;
  items: ItemRow[];
  seats: SeatRow[];
  files: FileRow[];
}

/** One line in the room (view.rs `Said`). */
export interface Said {
  seq: number;
  kind: string;
  who: string;
  whom: string | null;
  via: string;
  text: string;
  at: number;
  role: string | null;
  model: string | null;
  agent: string | null;
  provider: string | null;
  parent: number | null;
  member: string | null;
  mentions: string[];
  broadcast: boolean;
}

export interface Member {
  thread_id: string;
  name: string;
  display_name?: string | null;
  seat_id: string;
  role: string;
  read_only: boolean;
  state: string;
  live: boolean;
  agent: string | null;
  provider: string | null;
  model: string | null;
  reasoning: string | null;
  doing: string | null;
  reported: boolean;
  title: string | null;
  started_at: number;
  ended_at: number | null;
  seats: number;
  replies: number;
  /** Seconds, as the store keeps it. */
  last_reply_at: number | null;
  workspace_state: string | null;
  trial_state: string | null;
  needs: string | null;
  manager?: string | null;
  org_role?: string | null;
}

export interface Mention {
  message_id: number;
  thread_id: string;
  project_id: string;
  thread_title: string;
  project: string;
  who: string;
  member: string | null;
  parent: number | null;
  text: string;
  /** Seconds in the recorded fixture, milliseconds elsewhere; use `toMs`. */
  at: number;
}

export interface Prompt {
  id: string;
  thread_id: string;
  thread_title: string;
  project: string;
  kind: string;
  agent: string | null;
  model: string | null;
  role: string | null;
  title: string;
  detail: string | null;
  /** (option id, label) pairs the agent itself offered. */
  options: [string, string][];
  created_at: number;
  read_only: boolean;
  lead: boolean;
  workspace: string | null;
}

export interface LiveSeat {
  seat_id: string;
  thread_id: string;
  project_id: string;
  turn: string;
  ordinal: number;
  role: string;
  lead: boolean;
  read_only: boolean;
  title: string | null;
  background: boolean;
  state: string;
  started_at: number;
  agent: string | null;
  model: string | null;
  doing: string | null;
  usage?: unknown;
  provider: string | null;
}

export interface LiveTask {
  item_id: number;
  thread_id: string;
  project_id: string;
  turn: string;
  seat_id: string;
  role: string;
  key: string | null;
  title: string | null;
  started_at: number;
  calls: number;
  doing: string | null;
}

export interface Talk {
  seq: number;
  project_id: string;
  via: string;
  from: string;
  from_thread: string | null;
  from_role: string | null;
  to: string;
  to_thread: string | null;
  to_role: string | null;
  text: string;
  at: number;
  mentions: string[];
  read: boolean | null;
}

export interface RouteStats {
  agent: string;
  model: string;
  task_type: string;
  reasoning: string | null;
  verified: number;
  failures: number;
  weak: number;
  success_rate: number;
  mean_tokens: number;
  mean_duration_ms: number;
  last_at: number | null;
}

export interface SearchHit {
  thread_id: string;
  title: string;
  project: string;
  snippet: string | null;
}

export interface WorkspaceRow {
  id: string;
  name: string;
  title: string;
  root: string;
  base: string;
  state: string;
  seat_id: string | null;
  seat_state: string | null;
  agent: string | null;
  model: string | null;
  reported: boolean;
  doing: string | null;
  trial: string | null;
  trial_state: string | null;
  trials: number;
}

export interface MergeRow {
  id: string;
  workspace: string;
  n: number;
  state: string;
  files: string[];
  overlap: string[];
  conflicts: string[];
  checks: { name: string; passed: boolean }[];
  error: string | null;
}

export interface Team {
  plan: unknown;
  workspaces: WorkspaceRow[];
  merges: MergeRow[];
  members?: Member[];
}

/** A file read through the bounded, traversal-checked reader (files.rs `FileText`). */
export interface FileText {
  path: string;
  bytes: number;
  modified_at: number;
  text: string;
  lines: number;
  truncated: boolean;
  binary: boolean;
  image: string | null;
  status: string | null;
}

export interface Folder {
  name: string;
  root: string;
  threads?: number;
  updated_at?: number | null;
}

/** A folder the phone may open: `browse` lists the visible folders under `path`. */
export interface Browse {
  path: string;
  parent: string | null;
  /** The folder itself is a git checkout. */
  git: boolean;
  /** Where "clone" puts a repository; null until it is chosen in the desktop's Settings. */
  clone_dir: string | null;
  entries: { name: string; path: string; git: boolean }[];
}

/** Where a project's conversations may run (the desktop's `view::Placement`). */
export interface Placement {
  /** The project's own setting: a conversation started without a choice runs there. */
  mode: string;
  tier: string | null;
  docker: boolean;
  name: string | null;
  options: string[];
  /** Of `options`, the ones already made: only these can be chosen from a phone. */
  ready: string[];
}

export interface ProjectAdded {
  id: string;
  root: string;
  name?: string;
}

export type FileState =
  'conflict' | 'modified' | 'added' | 'deleted' | 'renamed' | 'untracked' | 'ignored';

/** One directory entry of a thread's tree (files.rs `Entry`). */
export interface TreeEntry {
  name: string;
  path: string;
  kind: 'dir' | 'file' | 'symlink';
  size: number | null;
  status: FileState | null;
  within: FileState | null;
  missing: boolean;
}

export interface TreeHit {
  path: string;
  line: number | null;
  text: string | null;
}

export interface GitCommit {
  hash: string;
  parents: string[];
  refs: string[];
  subject: string;
  at: number;
}

export interface GitWorktree {
  path: string;
  head: string;
  branch: string | null;
  current: boolean;
  primary: boolean;
}

export interface GitBranch {
  name: string;
  head: string;
  current: boolean;
  subject: string;
}

export interface GitOverview {
  branch: string | null;
  staged: number;
  unstaged: number;
  untracked: number;
  worktrees: GitWorktree[];
  branches: GitBranch[];
  commits: GitCommit[];
}

/** A part of the plan (`board` rows). */
export interface BoardPart {
  id: string;
  brief: string;
  paths: string[];
  after: string[];
  wave: number | null;
  state: string;
  strays: number | null;
  seat: string | null;
}

export type DiffScope = 'unstaged' | 'staged' | 'branch';

export interface AgentRow {
  agent: string;
  model: string;
  status: string;
  cooling: number;
  reset_at: number | null;
  quota_estimate: number | null;
  failures: number;
  windows: [string, number, number | null, boolean][];
  /** Pool and provider-reported balance. Absent on older desktops. */
  credits?: [string, string][];
}

export interface Health {
  ok: boolean;
  name: string;
  /** Stable per-desktop id; absent on a server older than multi-desktop support. */
  id?: string;
  version: string;
}

export interface PairResult {
  token: string;
  name: string;
  id?: string;
}

export interface Me {
  device: string;
  paired_at: number;
  id?: string;
  name?: string;
}

/** Media item `data` (kind "media"): the orochi-media answer's identity of a file. */
export interface MediaData {
  sha: string;
  kind: 'image' | 'video' | 'audio';
  mime: string;
  bytes: number;
  name: string;
  kept: boolean;
}

export type ServerEvent =
  { type: 'hello' } | { type: 'changed'; threads: string[] } | { type: 'prompts' };

// ---- residents, routines and notices (docs/resident-agent-design.md §14) ------------------

export interface ResidentRow {
  thread_id: string;
  name: string;
  home: string;
  instruction: string;
  access: 'read' | 'home';
  paused: boolean;
  created_at: number;
  /** Unix seconds; null when nothing is scheduled. */
  next_at: number | null;
  routines: number;
  asking: number;
  working: boolean;
}

export interface RoutineRow {
  id: string;
  thread_id: string;
  thread_title: string | null;
  name: string;
  instruction: string;
  schedule: string | null;
  watch: string | null;
  /** How often a watch looks, in seconds. */
  every_secs: number | null;
  access: string;
  notify: 'always' | 'mention' | 'never';
  catch_up: 'once' | 'skip';
  made_by: 'user' | 'agent';
  expires_at: number | null;
  paused: boolean;
  /** An agent proposed it in the conversation; it runs only once the person allows it. */
  proposed: boolean;
  /** The member of the team that runs it, by name; none: the lead. */
  member?: string | null;
  next_at: number | null;
  last_at: number | null;
  last_skipped: string | null;
  last_state: string | null;
  tokens_today: number;
}

export interface NoticeRow {
  id: number;
  kind: 'ask' | 'report' | 'mention' | 'trouble' | 'scheduled' | 'budget' | 'keeper';
  thread_id: string | null;
  routine_id: string | null;
  prompt_id: string | null;
  title: string;
  body: string | null;
  /** What became of it on each sink: sent, off, held, skipped_window, coalesced, failed, … */
  mac: string | null;
  phone: string | null;
  /** Milliseconds. */
  created_at: number;
  read_at: number | null;
}

export interface KeeperState {
  running: boolean;
  login: boolean;
  login_supported: boolean;
  tokens_today: number;
  daily_tokens: number;
  enabled: boolean;
}
