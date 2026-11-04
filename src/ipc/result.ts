import type { IpcError } from "./bindings";

/** Result shape produced by the generated command wrappers. */
export type IpcResult<T> = { status: "ok"; data: T } | { status: "error"; error: IpcError };

/** Error thrown by {@link unwrap}; carries the backend's typed error. */
export class BackendError extends Error {
  constructor(readonly ipc: IpcError) {
    super(ipc.message);
    this.name = "BackendError";
  }
}

/** Returns the data of an ok result or throws a {@link BackendError}. */
export function unwrap<T>(result: IpcResult<T>): T {
  if (result.status === "error") {
    throw new BackendError(result.error);
  }
  return result.data;
}

/** Human-readable message for any thrown value. */
export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error) {
    return String(error.message);
  }
  return String(error);
}
