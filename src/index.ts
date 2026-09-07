import * as readline from "node:readline";
import { runAgent } from "./agent.ts";
import { DEFAULT_MODEL } from "./llm.ts";

// Simple readline TUI — lean, pi-like but minimal

const CYAN = "\x1b[36m";
const DIM = "\x1b[2m";
const RESET = "\x1b[0m";
const GREEN = "\x1b[32m";
const YELLOW = "\x1b[33m";

function banner() {
  console.log(`${CYAN}lean${RESET} ${DIM}— light coding assistant  •  ${DEFAULT_MODEL}  •  max 20 steps  •  unrestricted${RESET}`);
  console.log(`${DIM}Type your task. /exit to quit.  e.g. "fix the bug in src/tools.ts"${RESET}\n`);
}

function formatArgs(args: any): string {
  const s = JSON.stringify(args);
  if (s.length > 120) return s.slice(0, 117) + "...";
  return s;
}

function truncateResult(s: string, limit = 3000): string {
  if (s.length <= limit) return s;
  return s.slice(0, limit) + `\n… [truncated ${s.length - limit} chars]`;
}

async function handlePrompt(prompt: string, model?: string) {
  console.log(`${DIM}─${RESET}`);
  let inText = false;

  for await (const ev of runAgent(prompt, { model })) {
    if (ev.type === "text") {
      if (!inText) inText = true;
      process.stdout.write(ev.delta);
    } else if (ev.type === "reasoning") {
      // show reasoning dim (optional, helps see thinking for mimo etc)
      process.stdout.write(`${DIM}${ev.delta}${RESET}`);
    } else if (ev.type === "text_done") {
      if (inText) process.stdout.write("\n");
      inText = false;
    } else if (ev.type === "tool_start") {
      if (inText) {
        process.stdout.write("\n");
        inText = false;
      }
      console.log(`${YELLOW}⎿ ${ev.name}${RESET} ${DIM}${formatArgs(ev.args)}${RESET}`);
    } else if (ev.type === "tool_result") {
      const preview = truncateResult(ev.result);
      // indent result
      const lines = preview.split("\n").slice(0, 20); // cap lines for TUI
      const extra = preview.split("\n").length > 20 ? `\n  ${DIM}… ${preview.split("\n").length - 20} more lines${RESET}` : "";
      console.log(`${DIM}  →${RESET} ${lines.join(`\n  `)}${extra}`);
    } else if (ev.type === "done") {
      if (inText) process.stdout.write("\n");
      console.log(`${GREEN}\nDone.${RESET}\n`);
      break;
    }
  }
}

async function interactive(model?: string) {
  banner();

  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    prompt: `${CYAN}> ${RESET}`,
  });

  rl.prompt();

  for await (const line of rl) {
    const prompt = line.trim();
    if (!prompt) {
      rl.prompt();
      continue;
    }
    if (prompt === "/exit" || prompt === "/quit" || prompt === "exit" || prompt === "quit") {
      rl.close();
      break;
    }
    if (prompt === "/help") {
      console.log(`${DIM}Commands: /exit, /help, /model <name>${RESET}`);
      rl.prompt();
      continue;
    }
    if (prompt.startsWith("/model")) {
      const m = prompt.split(/\s+/)[1];
      if (m) {
        model = m;
        console.log(`${DIM}Model → ${m}${RESET}`);
      } else {
        console.log(`${DIM}Current model: ${model || DEFAULT_MODEL}${RESET}`);
      }
      rl.prompt();
      continue;
    }

    try {
      await handlePrompt(prompt, model);
    } catch (e: any) {
      console.error(`${YELLOW}Error: ${e?.message ?? String(e)}${RESET}`);
    }
    rl.prompt();
  }
}

// CLI: bun src/index.ts "your prompt"  or  bun src/index.ts  (interactive)
const args = process.argv.slice(2);
const promptFromArgs = args.filter((a) => !a.startsWith("--") && !a.startsWith("/")).join(" ").trim();
const modelFlag = args.find((a) => a.startsWith("--model="))?.split("=")[1];

if (promptFromArgs) {
  // one-shot (still stateless, then exit)
  await handlePrompt(promptFromArgs, modelFlag).catch((e) => {
    console.error(e);
    process.exit(1);
  });
  process.exit(0);
} else {
  await interactive(modelFlag);
}
