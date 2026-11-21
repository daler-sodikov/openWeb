"use strict";
function total(items) {
    let sum = 0;
    for (const item of items) {
        sum += item.price;
    }
    return sum;
}
const cart = [
    { name: "pen", price: 2 },
    { name: "book", price: 12 },
];
console.log(`total ${total(cart)}`);
//# sourceMappingURL=app.js.map