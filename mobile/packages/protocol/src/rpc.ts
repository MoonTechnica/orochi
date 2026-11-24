import type {
  AgentRow,
  Browse,
  Folder,
  ProjectAdded,
  LiveSeat,
  LiveTask,
  Member,
  Mention,
  ProjectRow,
  Prompt,
  RouteStats,
  Said,
  SearchHit,
  Talk,
  Team,
  Thread,
  FileText,
  TreeEntry,
  TreeHit,
  GitOverview,
  BoardPart,
  FileRow,
  DiffScope,
  KeeperState,
  NoticeRow,
  ResidentRow,
  RoutineRow,
} from './types';
import type { RemoteClient } from './client';

/** Typed wrappers over the whitelisted `/rpc/<command>` surface the app uses. */
export const api = (c: RemoteClient) => ({
  sidebar: (archived = false, limit = 200) => c.rpc<ProjectRow[]>('sidebar', { limit, archived }),
  thread: (id: string) => c.rpc<Thread | null>('thread', { id }),
  patch: (id: number) => c.rpc<string | null>('patch', { id }),
  room: (thread: string) => c.rpc<Said[]>('room', { thread }),
  chatter: (since: number) => c.rpc<Talk[]>('chatter', { since }),
  members: (thread: string) => c.rpc<Member[]>('members', { thread }),
  renameMember: (thread: string, member: string, name: string) =>
    c.rpc<null>('rename_member', { thread, member, name }),
  mentions: () => c.rpc<Mention[]>('mentions'),
  team: (thread: string) => c.rpc<Team>('team', { thread }),
  openPrompts: () => c.rpc<Prompt[]>('open_prompts'),
  liveSeats: () => c.rpc<LiveSeat[]>('live_seats'),
  liveTasks: () => c.rpc<LiveTask[]>('live_tasks'),
  search: (query: string) => c.rpc<SearchHit[]>('search', { query }),
  agents: () => c.rpc<AgentRow[]>('agents'),
  insights: () => c.rpc<RouteStats[]>('insights'),
  residents: () => c.rpc<ResidentRow[]>('residents'),
  routines: () => c.rpc<RoutineRow[]>('routines'),
  notices: (unread = false) => c.rpc<NoticeRow[]>('notices', { unread }),
  keeper: () => c.rpc<KeeperState>('keeper'),
  /**
   * A phone can run, pause or resume a routine, and allow or decline one an agent proposed;
   * deleting one stays at the desktop.
   */
  routineAction: (id: string, action: 'run' | 'pause' | 'resume' | 'approve' | 'decline') =>
    c.rpc<boolean>('routine_action', { id, action }),
  /** Makes a resident: a pinned conversation with a folder of its own, to talk to. */
  residentAdd: (name: string, instruction: string) =>
    c.rpc<{ thread: string; home: string }>('resident_add', { name, instruction }),
  /** Pauses or resumes a resident: the one switch for everything it has scheduled. */
  residentAction: (name: string, action: 'pause' | 'resume') =>
    c.rpc<boolean>('resident_action', { name, action }),
  readNotices: (id?: number) => c.rpc<null>('read_notices', id === undefined ? {} : { id }),
  /** Provider -> small PNG data URL of the vendor's icon, where its app is installed on the desktop. */
  agentIcons: () => c.rpc<Record<string, string>>('agent_icons'),
  folders: () => c.rpc<Folder[]>('folders'),
  treeRead: (thread: string, path: string) => c.rpc<FileText>('tree_read', { thread, path }),
  treePatch: (thread: string, scope: string, path: string) =>
    c.rpc<string>('tree_patch', { thread, scope, path }),
  treeList: (thread: string, dir: string) => c.rpc<TreeEntry[]>('tree_list', { thread, dir }),
  treeFind: (thread: string, query: string, by: 'name' | 'content', limit = 50) =>
    c.rpc<TreeHit[] | null>('tree_find', { thread, query, by, limit }),
  treeFiles: (thread: string, scope: DiffScope) =>
    c.rpc<FileRow[]>('tree_files', { thread, scope }),
  gitOverview: (thread: string) => c.rpc<GitOverview>('git_overview', { thread }),
  gitShow: (thread: string, hash: string) => c.rpc<string>('git_show', { thread, hash }),
  board: (thread: string) => c.rpc<BoardPart[]>('board', { thread }),
  comment: (thread: string, comments: [string, number, string][]) =>
    c.rpc<string>('comment', { thread, comments }),
  setRoute: (thread: string, route: string | null) => c.rpc<null>('set_route', { thread, route }),
  send: (thread: string, text: string) => c.rpc<string>('send', { thread, text }),
  say: (thread: string, text: string, to: string | null = null, member: string | null = null) =>
    c.rpc<null>('say', { thread, to, text, member }),
  answer: (prompt: string, option: string | null) => c.rpc<boolean>('answer', { prompt, option }),
  interrupt: (thread: string) => c.rpc<null>('interrupt', { thread }),
  stopSeat: (thread: string, seat: string | null) => c.rpc<null>('stop_seat', { thread, seat }),
  seen: (thread: string) => c.rpc<null>('seen', { thread }),
  seeMentions: (thread: string) => c.rpc<null>('see_mentions', { thread }),
  newThread: (root: string, worktree = false) =>
    c.rpc<string>('new_thread', { root, worktree, sandbox: null }),
  browse: (path: string | null = null) => c.rpc<Browse>('browse', { path }),
  addProject: (root: string) => c.rpc<ProjectAdded>('add_project', { root }),
  /** A clone can take minutes: the desktop stops it after 15, so the wait is a little longer. */
  cloneProject: (url: string, name: string | null = null) =>
    c.rpc<ProjectAdded>('clone_project', { url, name }, 16 * 60 * 1000),
  ensureHost: (thread: string) => c.rpc<boolean>('ensure_host', { thread }),
  renameThread: (thread: string, title: string) => c.rpc<null>('rename_thread', { thread, title }),
  pinThread: (thread: string, pinned: boolean) => c.rpc<null>('pin_thread', { thread, pinned }),
  archiveThread: (thread: string, archived: boolean) =>
    c.rpc<null>('archive_thread', { thread, archived }),
});

export type Api = ReturnType<typeof api>;
