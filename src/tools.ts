import { $ } from "bun";

export type SearchResult = {
  title: string;
  url: string;
  snippet: string;
};

// --- file tools ---

export async function readFile(path: string): Promise<string> {
  const file = Bun.file(path);
  if (!(await file.exists())) {
    throw new Error(`File not found: ${path}`);
  }
  return await file.text();
}

export async function writeFile(path: string, content: string): Promise<string> {
  const { dirname } = await import("node:path");
  const dir = dirname(path);
  if (dir && dir !== ".") {
    await $`mkdir -p ${dir}`.nothrow().quiet();
  }
  await Bun.write(path, content);
  return `Wrote ${content.length} bytes to ${path}`;
}

export async function editFile(path: string, oldText: string, newText: string): Promise<string> {
  const content = await readFile(path);
  const occurrences = content.split(oldText).length - 1;
  if (occurrences === 0) {
    throw new Error(`edit_file: oldText not found in ${path}`);
  }
  if (occurrences > 1) {
    throw new Error(`edit_file: oldText matches ${occurrences} times in ${path} — must be unique`);
  }
  const next = content.replace(oldText, newText);
  await Bun.write(path, next);
  return `Edited ${path} (replaced 1 occurrence)`;
}

// --- bash ---

export async function runBash(command: string): Promise<string> {
  // Unrestricted, no timeout, verbatim stdout+stderr
  const result = await $`bash -c ${command}`.nothrow().quiet();
  const stdout = result.stdout.toString();
  const stderr = result.stderr.toString();
  const exitCode = result.exitCode ?? 0;

  let out = "";
  if (stdout) out += stdout;
  if (stderr) {
    if (out && !out.endsWith("\n")) out += "\n";
    out += stderr;
  }
  if (!out) out = `(no output, exit ${exitCode})`;
  else out += `\n[exit ${exitCode}]`;
  return out;
}

// --- web search ---

export async function webSearch(query: string): Promise<SearchResult[]> {
  // Prefer Exa if key available (more reliable than DuckDuckGo)
  const exaKey = process.env.EXA_API_KEY;
  if (exaKey) {
    try {
      const res = await fetch("https://api.exa.ai/search", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-api-key": exaKey,
        },
        body: JSON.stringify({ query, numResults: 5, type: "auto", contents: { text: { maxCharacters: 600 } } }),
      });
      if (res.ok) {
        const data = (await res.json()) as any;
        const results: SearchResult[] = (data.results || []).slice(0, 5).map((r: any) => ({
          title: r.title || r.url || query,
          url: r.url || "",
          snippet: (r.text || r.snippet || r.highlights?.[0] || "").slice(0, 600),
        }));
        if (results.length > 0) return results;
      }
    } catch {
      // fall through to DuckDuckGo
    }
  }

  // Fallback: DuckDuckGo (fixed bug from original)
  const response = await fetch(`https://api.duckduckgo.com/?q=${encodeURIComponent(query)}&format=json`);
  if (!response.ok) {
    throw new Error(`Search failed: ${response.status}`);
  }
  const data = (await response.json()) as any;
  const topics: any[] = data.RelatedTopics || [];
  // Flatten nested Topics
  const flat: any[] = [];
  for (const item of topics) {
    if (item.Topics) flat.push(...item.Topics);
    else flat.push(item);
  }
  return flat
    .filter((item: any) => item.FirstURL && item.Text)
    .slice(0, 5)
    .map((item: any) => ({
      title: item.Text.split(" - ")[0]?.slice(0, 120) || item.Text.slice(0, 80),
      url: item.FirstURL as string,
      snippet: item.Text as string,
    }));
}

// --- skills ---

export async function readSkill(name: string): Promise<string> {
  const { loadSkill } = await import("./skills.ts");
  return await loadSkill(name);
}

// --- dispatcher ---

export async function executeTool(name: string, args: any): Promise<string> {
  switch (name) {
    case "read_file":
      return await readFile(args.path);
    case "write_file":
      return await writeFile(args.path, args.content);
    case "edit_file":
      return await editFile(args.path, args.oldText, args.newText);
    case "bash":
      return await runBash(args.command);
    case "web_search":
      return JSON.stringify(await webSearch(args.query), null, 2);
    case "read_skill":
      return await readSkill(args.name);
    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}
