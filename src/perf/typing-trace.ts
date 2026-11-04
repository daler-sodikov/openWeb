// Typing-path timing (docs/ARCHITECTURE.md, 4.8), on only when the backend
// runs with FERRULE_TYPING_TRACE (ferrule-perf sets it). Per keystroke:
// key event → local transaction dispatched → frame with the local echo →
// Rust confirmed the edit → frame with fresh highlights. Each stage is a
// User Timing measure and a sample sent to Rust, which writes the file.
import { commands, type TraceSample } from "../ipc/bindings";

/** One keystroke being traced. */
export interface KeyTrace {
  readonly start: number;
}

const FLUSH_MS = 1000;

let enabled = false;
let pendingKey: number | null = null;
const samples: TraceSample[] = [];

export async function initTypingTrace(): Promise<void> {
  enabled = await commands.typingTraceEnabled().then(
    (r) => r.status === "ok" && r.data,
    () => false,
  );
  if (!enabled) return;
  setInterval(() => {
    if (samples.length === 0) return;
    const batch = samples.splice(0);
    void commands.recordTypingTrace(batch);
  }, FLUSH_MS);
}

/** A printable key went down (`event.timeStamp`, performance clock). */
export function keyPressed(timeStamp: number): void {
  if (enabled) pendingKey = timeStamp;
}

/** The trace of the key that produced the transaction being dispatched. */
export function takeKey(): KeyTrace | null {
  if (!enabled || pendingKey === null) return null;
  const trace = { start: pendingKey };
  pendingKey = null;
  return trace;
}

export function mark(trace: KeyTrace, stage: string): void {
  const end = performance.now();
  performance.measure(`typing:${stage}`, { start: trace.start, end });
  samples.push({ stage: `view.${stage}`, ms: end - trace.start });
}

/** Marks `stage` in the next animation frame (just before it is painted). */
export function markNextFrame(trace: KeyTrace, stage: string): void {
  requestAnimationFrame(() => {
    mark(trace, stage);
  });
}
