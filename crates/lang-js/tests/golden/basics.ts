// Greeting helpers
import { format } from "./format";

export interface User {
  readonly id: number;
  name?: string;
}

type Mode = "short" | "long";

export function greet(user: User, mode: Mode = "short"): string {
  const prefix = mode === "long" ? `Hello, ${user.name}!` : "Hi";
  return format(prefix, user.id * 2, /\d+/g, null, true);
}

class Counter<T> extends Base {
  private count = 0;
  increment(): void {
    this.count += 1;
    console.log(this.count);
  }
}
