import * as readline from "node:readline";
import { runAgent } from "./agent.ts";
import { DEFAULT_MODEL } from "./llm.ts";

const args = process.argv.slice(2);
const promptFromArgs = args.filter((a) => !a.startsWith("--") && !a.startsWith("/")).join(" ").trim();
const modelFlag = args.find((a) => a.startsWith("--model="))?.split("=")[1];
const useSimple = args.includes("--simple") || args.includes("--readline");

// Simple readline TUI — lean, pi-like but minimal

const CYAN = "\x1b[36m";
const DIM = "\x1b[2m";
const RESET = "\x1b[0m";
const GREEN = "\x1b[32m";
const YELLOW = "\x1b[33m";
const BOLD = "\x1b[1m";
const MAGENTA = "\x1b[35m";

function banner() {
  console.log(`${CYAN}lean${RESET} ${DIM}— light coding assistant  •  ${DEFAULT_MODEL}  •  max 20 steps  •  unrestricted${RESET}`);
  console.log(`${DIM}Type your task. /exit to quit.  e.g. "fix the bug in src/tools.ts"${RESET}\n`);
}

function formatArgsPretty(args: any): string {
  try {
    return JSON.stringify(args, null, 2);
  } catch {
    return String(args);
  }
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
      const pretty = formatArgsPretty(ev.args);
      const lines = pretty.split("\n");
      console.log(`${YELLOW}${BOLD}┌─ ${ev.name}${RESET} ${DIM}#${ev.id.slice(0, 8)}${RESET}`);
      for (const l of lines) {
        console.log(`${YELLOW}│${RESET} ${DIM}${l}${RESET}`);
      }
    } else if (ev.type === "tool_result") {
      const result = ev.result || "(empty)";
      const bytes = Buffer.byteLength(result, "utf-8");
      const lineCount = result.split("\n").length;
      console.log(`${YELLOW}├─ result${RESET} ${DIM}(${bytes}B, ${lineCount} lines)${RESET}`);
      // verbatim, no truncation — full stdout/stderr
      for (const line of result.split("\n")) {
        console.log(`${YELLOW}│${RESET} ${line}`);
      }
      console.log(`${YELLOW}└─${RESET}`);
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

// CLI mode
if (promptFromArgs) {
  // one-shot (still stateless, then exit) — uses boxed readline-style output
  await handlePrompt(promptFromArgs, modelFlag).catch((e) => {
    console.error(e);
    process.exit(1);
  });
  process.exit(0);
} else {
  // interactive: prefer pi-like Ink TUI if TTY and not --simple, fallback to readline
  const isTTY = process.stdin.isTTY && process.stdout.isTTY;
  if (!useSimple && isTTY) {
    try {
      const { default: App } = await import("./tui.tsx");
      const { render } = await import("ink");
      const React = await import("react");
      render(React.createElement(App));
    } catch (e: any) {
      const msg = e?.message || String(e);
      if (msg.includes("Raw mode") || msg.includes("isRawModeSupported")) {
        console.log(`${DIM}Ink TUI needs a TTY (raw mode). Falling back to readline. Use --simple to force readline.${RESET}\n`);
        await interactive(modelFlag);
      } else {
        console.error(`Failed to launch Ink TUI: ${msg}`);
        console.log(`${DIM}Falling back to readline...${RESET}\n`);
        await interactive(modelFlag);
      }
    }
  } else {
    await interactive(modelFlag);
  }
}
