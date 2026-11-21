// Decides whether a failed app start is worth one more try.

/**
 * Chromium's name for Windows WSAENOBUFS: the OS had no buffer space for a
 * socket. CI runners hit it at random while a page loads its modules; it says
 * nothing about the app.
 */
export const TRANSIENT_NETWORK_ERROR = "net::ERR_NO_BUFFER_SPACE";

/** True when a request failed with the transient error (and nothing else is implied). */
export function isTransientStartupFailure(failedRequestErrors: readonly string[]): boolean {
  return failedRequestErrors.includes(TRANSIENT_NETWORK_ERROR);
}
