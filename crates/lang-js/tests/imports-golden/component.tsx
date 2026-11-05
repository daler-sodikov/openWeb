import { useState, useMemo } from "react";
import type { Props } from "./types";
import { formatPrice } from "./utils/format";
import "./styles.css";
import { z } from "zod";
import * as path from "node:path";
import Default, { named } from "../shared/lib";
import { unused } from "./unused";

export function Card({ title }: Props) {
  const [count] = useState(0);
  const schema = z.string();
  return <Default label={formatPrice(count)} schema={schema}>{title}</Default>;
}
