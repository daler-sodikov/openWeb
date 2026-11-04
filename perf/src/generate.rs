//! Deterministic generated project for benchmarks: TS, TSX, CSS and JSON
//! files with original, varied content.

use std::fs;
use std::io;
use std::path::Path;

fn service(f: usize, i: usize) -> String {
    let mut out = format!(
        "// Service {i} of feature {f}.\nimport {{ formatValue }} from \"../shared/format\";\n\n\
         export interface Record{i} {{\n  id: number;\n  name: string;\n  tags: string[];\n  score: number;\n}}\n\n\
         export class Store{i} {{\n  private items = new Map<number, Record{i}>();\n\n"
    );
    for m in 0..(4 + (f + i) % 6) {
        out.push_str(&format!(
            "  method{m}(input: Record{i}, factor = {factor}): number {{\n    const scaled = input.score * factor;\n    \
             if (scaled > {limit}) {{\n      return Math.round(scaled / 2);\n    }}\n    \
             this.items.set(input.id, {{ ...input, tags: [...input.tags, \"m{m}\"] }});\n    \
             return scaled + formatValue(input.name).length;\n  }}\n\n",
            factor = m + 1,
            limit = 100 + 7 * m
        ));
    }
    out.push_str("  size(): number {\n    return this.items.size;\n  }\n}\n");
    out
}

fn component(f: usize, i: usize) -> String {
    format!(
        "import {{ useState }} from \"react\";\nimport {{ Store{i} }} from \"./service_{i:02}\";\n\n\
         export function Widget{i}({{ title }}: {{ title: string }}) {{\n  const [count, setCount] = useState({f});\n  \
         const store = new Store{i}();\n  return (\n    <section className=\"widget-{f}-{i}\">\n      <h2>{{title}}</h2>\n      \
         <button onClick={{() => setCount(count + 1)}}>Clicked {{count}}</button>\n      <p>{{store.size()}} items</p>\n    \
         </section>\n  );\n}}\n"
    )
}

fn styles(f: usize, i: usize) -> String {
    format!(
        ".widget-{f}-{i} {{\n  --gap: {}px;\n  display: grid;\n  gap: var(--gap);\n  padding: calc(var(--gap) * 2);\n}}\n\n\
         .widget-{f}-{i} > h2 {{\n  font-size: 1.{}rem;\n  color: #{:06x};\n}}\n",
        4 + i % 5,
        i % 9,
        (f * 7919 + i * 104729) % 0xFFFFFF
    )
}

fn data(f: usize, i: usize) -> String {
    let items: Vec<String> = (0..8)
        .map(|k| {
            format!(
                "    {{ \"id\": {k}, \"name\": \"item-{f}-{i}-{k}\", \"enabled\": {} }}",
                k % 2 == 0
            )
        })
        .collect();
    format!(
        "{{\n  \"feature\": {f},\n  \"items\": [\n{}\n  ]\n}}\n",
        items.join(",\n")
    )
}

/// Writes about `files` files under `root` and returns how many were written.
pub fn generate(root: &Path, files: usize) -> io::Result<usize> {
    fs::create_dir_all(root.join("src/shared"))?;
    fs::write(
        root.join("package.json"),
        "{\n  \"name\": \"bench-project\",\n  \"private\": true\n}\n",
    )?;
    fs::write(
        root.join("main.ts"),
        "// Typing target for ferrule-perf.\nexport const answer = 42;\n",
    )?;
    fs::write(
        root.join("src/shared/format.ts"),
        "export function formatValue(value: string): string {\n  return value.trim().toLowerCase();\n}\n",
    )?;
    let mut written = 3;
    let mut f = 0;
    while written < files {
        let dir = root.join(format!("src/feature_{f:03}"));
        fs::create_dir_all(&dir)?;
        for i in 0..5 {
            for (name, text) in [
                (format!("service_{i:02}.ts"), service(f, i)),
                (format!("widget_{i:02}.tsx"), component(f, i)),
                (format!("styles_{i:02}.css"), styles(f, i)),
                (format!("data_{i:02}.json"), data(f, i)),
            ] {
                if written >= files {
                    return Ok(written);
                }
                fs::write(dir.join(name), text)?;
                written += 1;
            }
        }
        f += 1;
    }
    Ok(written)
}
