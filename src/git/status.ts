// The latest Git status from Rust, indexed for the tree: which class a
// row gets.
import type { GitFile, GitFileKind, GitStatus } from "../ipc/bindings";

export type TreeGitClass = GitFileKind | "changed";

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
