import type { ProjectRow, ThreadRow } from '@orochi/protocol';

export interface ListSection {
  id: string;
  title: string;
  kind: 'pinned' | 'project';
  project?: ProjectRow;
  threads: ThreadRow[];
}

const byRecent = (a: ThreadRow, b: ThreadRow) => b.updated_at - a.updated_at;

/** Pinned conversations first, then each project (pinned projects first) newest-first. */
export function listSections(projects: ProjectRow[], query = '', archived = false): ListSection[] {
  const q = query.trim().toLowerCase();
  const fits = (th: ThreadRow, project: ProjectRow) =>
    !q || th.title.toLowerCase().includes(q) || project.name.toLowerCase().includes(q);
  const pinned: ThreadRow[] = [];
  const sections: ListSection[] = [];
  const ordered = [...projects].sort((a, b) => Number(b.pinned) - Number(a.pinned));
  for (const project of ordered) {
    if (project.missing) continue;
    const rest: ThreadRow[] = [];
    for (const th of project.threads) {
      if (!!th.archived !== archived || !fits(th, project)) continue;
      (th.pinned ? pinned : rest).push(th);
    }
    if (rest.length || (!q && !archived && project.threads.length === 0)) {
      sections.push({
        id: project.id,
        title: project.name,
        kind: 'project',
        project,
        threads: rest.sort(byRecent),
      });
    }
  }
  if (pinned.length) {
    sections.unshift({
      id: 'pinned',
      title: 'Pinned',
      kind: 'pinned',
      threads: pinned.sort(byRecent),
    });
  }
  return sections;
}

export const countNeeds = (projects: ProjectRow[]) =>
  projects.flatMap((p) => p.threads).filter((th) => th.status === 'needs_you').length;

export const totalUnread = (projects: ProjectRow[]) =>
  projects.flatMap((p) => p.threads).reduce((n, th) => n + (th.unread > 0 ? 1 : 0), 0);
