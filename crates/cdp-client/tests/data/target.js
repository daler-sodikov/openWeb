function add(a, b) {
  const sum = a + b;
  return sum;
}
const obj = { name: "ferrule", list: [1, 2, 3] };
console.log("hello", obj);
const total = add(1, 2);
setTimeout(() => { throw new Error("boom"); }, 50);
