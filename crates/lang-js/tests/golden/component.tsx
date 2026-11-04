import { useState } from "react";

export function Button({ label }: { label: string }) {
  const [clicks, setClicks] = useState<number>(0);
  return (
    <button className="btn" onClick={() => setClicks(clicks + 1)}>
      {label}: {clicks}
    </button>
  );
}

const Panel = () => <section />;
Button({ label: "x" });
