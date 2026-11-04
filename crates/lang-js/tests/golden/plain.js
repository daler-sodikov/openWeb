/* Plain JavaScript */
const items = [1, 2.5, 0x1f];
for (let i = 0; i < items.length; i++) {
  if (items[i] > 1) break;
}
module.exports = { items, ok: undefined };
