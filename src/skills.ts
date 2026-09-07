import { readdir } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";

export type Skill = {
  name: string;
  description: string;
  path: string;
  content: string;
};

// ── P1 skills cache ─────────────────────────────────────────────────────────
let _cached: Skill[] | null = null;
let _cachedAt = 0;
const CACHE_TTL_MS = 60_000;
// ─────────────────────────────────────────────────────────────────────────────

function parseFrontmatter(raw: string): { name?: string; description?: string; body: string } {
  if (!raw.startsWith("---")) return { body: raw };
  const end = raw.indexOf("\n---", 3);
  if (end === -1) return { body: raw };
  const fm = raw.slice(3, end).trim();
  const body = raw.slice(end + 4).trimStart(); // skip \n---
  let name: string | undefined;
  let description: string | undefined;
  for (const line of fm.split("\n")) {
    const m = line.match(/^\s*(name|description)\s*:\s*(.*)\s*$/);
    if (!m) continue;
    const key = m[1]!.trim();
    let val = m[2]!.trim();
    // strip quotes
    if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
      val = val.slice(1, -1);
    }
    if (key === "name") name = val;
    if (key === "description") description = val;
  }
  return { name, description, body };
}

async function scanDir(base: string): Promise<Skill[]> {
  const skills: Skill[] = [];
  try {
    const entries = await readdir(base, { withFileTypes: true });
    for (const ent of entries) {
      if (!ent.isDirectory()) continue;
      const skillPath = join(base, ent.name, "SKILL.md");
      const file = Bun.file(skillPath);
      if (!(await file.exists())) continue;
      const raw = await file.text();
      const { name, description, body } = parseFrontmatter(raw);
      const skillName = name || ent.name;
      const desc = description || body.split("\n").find((l) => l.trim())?.slice(0, 140) || "";
      skills.push({
        name: skillName,
        description: desc,
        path: skillPath,
        content: raw,
      });
    }
  } catch {
    // dir doesn't exist or unreadable — ignore
  }
  return skills;
}

export async function discoverSkills(): Promise<Skill[]> {
  // Return cached result if still fresh
  if (_cached && Date.now() - _cachedAt < CACHE_TTL_MS) return _cached;

  const home = homedir();
  const bases = [
    join(process.cwd(), "skills"),
    join(process.cwd(), ".lean", "skills"),
    join(home, ".agents", "skills"),
  ];

  const all: Map<string, Skill> = new Map();
  for (const base of bases) {
    const found = await scanDir(base);
    for (const s of found) {
      // local project dirs win over general ~/.agents/skills
      const isLocal = base.startsWith(process.cwd());
      if (!all.has(s.name) || isLocal) {
        all.set(s.name, s);
      }
    }
  }
  const result = [...all.values()].sort((a, b) => a.name.localeCompare(b.name));

  // Store in cache
  _cached = result;
  _cachedAt = Date.now();
  return result;
}

export async function loadSkill(name: string): Promise<string> {
  const skills = await discoverSkills();
  const found = skills.find((s) => s.name === name);
  if (!found) {
    const avail = skills.map((s) => s.name).join(", ") || "(none)";
    throw new Error(`Skill not found: ${name}. Available: ${avail}`);
  }
  return found.content;
}

export async function getSkillCatalog(): Promise<string> {
  const skills = await discoverSkills();
  // Filter pi-internal meta-skills that should not auto-trigger for every prompt
  const filtered = skills.filter((s) => s.name !== "using-superpowers");
  if (filtered.length === 0) return "No skills installed. Use `npx skills add <skill>` to install to ~/.agents/skills.";
  return filtered
    .map((s) => `- ${s.name}: ${s.description} (path: ${s.path})`)
    .join("\n");
}
