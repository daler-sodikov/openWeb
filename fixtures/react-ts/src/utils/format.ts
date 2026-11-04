// Formatting helpers.
export function formatPrice(value: number, currency = "EUR"): string {
  return new Intl.NumberFormat("en", { style: "currency", currency }).format(value);
}

export function pluralize(word: string, count: number): string {
  return count === 1 ? word : `${word}s`;
}
