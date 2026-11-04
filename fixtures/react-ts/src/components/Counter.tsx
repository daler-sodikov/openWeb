import { useState } from "react";

export function Counter({ start }: { start: number }) {
  const [count, setCount] = useState(start);
  const reset = () => setCount(start);
  return (
    <div>
      <button onClick={() => setCount(count + 1)}>count is {count}</button>
      <button onClick={reset}>reset</button>
    </div>
  );
}
