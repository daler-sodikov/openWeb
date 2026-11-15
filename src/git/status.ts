// The latest Git status from Rust, indexed for the tree: which class a
// row gets.
import type { GitFile, GitFileKind, GitStatus } from "../ipc/bindings";

export type TreeGitClass = GitFileKind | "changed";

/**
 * Whether `next` is newer than the status generation already shown. Rust
 * numbers every status across projects, so an answer to `gitStatus` that
 * arrives after a newer event (or a late status of a closed project) loses.
 */
export function isNewer(shown: number, next: GitStatus): boolean {
  return next.generation > shown;
}

/** The status bar text: the branch, else the detached commit, else nothing. */
export function branchLabel(status: GitStatus): string {
  if (status.unavailable !== null) return "";
  if (status.branch) return status.branch;
  return status.head ? `detached ${status.head}` : "";
}

export class GitStatusModel {
  status: GitStatus | null = null;
  private files = new Map<string, GitFile>();
  private changedDirs = new Set<string>();
  private ignoredDirs: string[] = [];

  update(status: GitStatus): void {
    this.status = status;
    this.files = new Map(status.files.map((f) => [f.path, f]));
    this.changedDirs = new Set(status.changedDirs);
    this.ignoredDirs = status.ignoredDirs;
  }

  file(path: string): GitFile | undefined {
    return this.files.get(path);
  }

  /** Tree class of a file or folder: its own status, else ignored or "contains changes". */
  classFor(path: string, directory: boolean): TreeGitClass | null {
    const own = this.files.get(path)?.kind;
    if (own) return own;
    if (this.ignoredDirs.some((dir) => path === dir || path.startsWith(`${dir}/`))) {
      return "ignored";
    }
    if (directory && this.changedDirs.has(path)) return "changed";
    return null;
  }
}
