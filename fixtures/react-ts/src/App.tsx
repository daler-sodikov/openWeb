import { Counter } from "./components/Counter";
import { formatPrice } from "./utils/format";
import items from "./data/items.json";

export interface AppProps {
  title: string;
}

export function App({ title }: AppProps) {
  const total = items.reduce((sum, item) => sum + item.price, 0);
  return (
    <main className="app">
      <h1>{title}</h1>
      <Counter start={items.length} />
      <p>Total: {formatPrice(total)}</p>
    </main>
  );
}
