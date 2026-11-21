// A small program to debug: a loop, a function and a blank line that has no code.
interface Item {
  name: string;
  price: number;
}

function total(items: Item[]): number {
  let sum = 0;
  for (const item of items) {
    sum += item.price;
  }

  return sum;
}

const cart: Item[] = [
  { name: "pen", price: 2 },
  { name: "book", price: 12 },
];

console.log(`total ${total(cart)}`);
