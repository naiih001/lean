import { client, TOOLS, DEFAULT_MODEL } from "./llm.ts";
import { executeTool } from "./tools.ts";
import { getSkillCatalog } from "./skills.ts";
import type OpenAI from "openai";

export const SYSTEM_PROMPT =
  "You are a lean coding assistant. Use tools to accomplish the task. Be concise. Fully autonomous until task done. Prefer read_file before edit_file. Use bash for inspection. Be helpful and precise.";

export async function buildSystemPrompt(): Promise<string> {
  const catalog = await getSkillCatalog();
  return `${SYSTEM_PROMPT}

Available skills (pi-style .md from ~/.agents/skills + project ./skills). Load one or multiple with read_skill when relevant — read_skill returns full SKILL.md instructions to follow:
${catalog}

When a task matches a skill, call read_skill. You may call multiple read_skill in one step if needed.`;
}

const MAX_TOOL_OUTPUT_FOR_LLM = 2000;

function truncateForLLM(s: string, limit = MAX_TOOL_OUTPUT_FOR_LLM): string {
  if (s.length <= limit) return s;
  return s.slice(0, limit) + `\n… [truncated ${s.length - limit} chars for LLM, full shown in TUI]`;
}

export type AgentEvent =
  | { type: "text"; delta: string }
  | { type: "reasoning"; delta: string }
  | { type: "text_done"; text: string }
  | { type: "tool_start"; name: string; args: any; id: string }
  | { type: "tool_result"; name: string; result: string; id: string }
  | { type: "step"; n: number }
  | { type: "done"; text: string };

export async function* runAgent(
  userPrompt: string,
  opts?: { model?: string; maxSteps?: number }
): AsyncGenerator<AgentEvent, string, unknown> {
  const model = opts?.model || DEFAULT_MODEL;
  const maxSteps = opts?.maxSteps ?? 100;

  const systemPrompt = await buildSystemPrompt();
  let messages: OpenAI.Chat.Completions.ChatCompletionMessageParam[] = [
    { role: "system", content: systemPrompt },
    { role: "user", content: userPrompt },
  ];

  let finalText = "";

  for (let step = 0; step < maxSteps; step++) {
    yield { type: "step", n: step + 1 };

    // Use streaming for live TUI
    const stream = await client.chat.completions.create({
      model,
      messages,
      tools: TOOLS,
      tool_choice: "auto",
      stream: true,
    });

    let content = "";
    // Accumulate tool calls by index
    const toolCalls: Map<number, { id: string; name: string; args: string }> = new Map();

    for await (const chunk of stream) {
      const delta = chunk.choices[0]?.delta as any;
      if (!delta) continue;

      if (delta.content) {
        content += delta.content;
        yield { type: "text", delta: delta.content };
      }
      if (delta.reasoning) {
        yield { type: "reasoning", delta: delta.reasoning };
      }

      if (delta.tool_calls) {
        for (const tc of delta.tool_calls as any[]) {
          const idx = tc.index ?? 0;
          const existing = toolCalls.get(idx) || { id: "", name: "", args: "" };
          if (tc.id) existing.id = tc.id;
          if (tc.function?.name) existing.name = tc.function.name;
          if (tc.function?.arguments) existing.args += tc.function.arguments;
          toolCalls.set(idx, existing);
        }
      }
    }

    if (content) {
      yield { type: "text_done", text: content };
      finalText = content;
    }

    // Non-streaming fallback: if we somehow got no content and no tools but need to check
    const calls = [...toolCalls.values()].filter((c) => c.name);

    if (calls.length === 0) {
      // No tools → done. Push assistant message for completeness
      if (content) {
        messages.push({ role: "assistant", content } as any);
      }
      yield { type: "done", text: content || finalText };
      return content || finalText;
    }

    // We have tool calls — need to push assistant message with tool_calls
    const assistantMsg: any = {
      role: "assistant",
      content: content || null,
      tool_calls: calls.map((c) => ({
        id: c.id || `call_${Math.random().toString(36).slice(2, 8)}`,
        type: "function" as const,
        function: { name: c.name, arguments: c.args || "{}" },
      })),
    };
    messages.push(assistantMsg);

    // P0: Parallel tool execution — yield all tool_starts first, then await all, then yield results in order.
    // Fall back to sequential if any tool is in SEQUENTIAL_TOOLS (reserved for future use).
    const SEQUENTIAL_TOOLS: string[] = []; // e.g. ['write_file'] if needed
    const isSequential = calls.some((c) => SEQUENTIAL_TOOLS.includes(c.name));

    // Prepare all calls with parsed args + ids
    const prepared = calls.map((c) => {
      let args: any = {};
      try {
        args = c.args ? JSON.parse(c.args) : {};
      } catch {
        args = {};
      }
      const id = c.id || `call_${Math.random().toString(36).slice(2, 8)}`;
      return { ...c, args, id };
    });

    if (isSequential) {
      // Sequential path — preserve original behavior
      for (const c of prepared) {
        yield { type: "tool_start", name: c.name, args: c.args, id: c.id };
        let result: string;
        try {
          result = await executeTool(c.name, c.args);
        } catch (e: any) {
          result = `Error: ${e?.message ?? String(e)}`;
        }
        yield { type: "tool_result", name: c.name, result, id: c.id };
        messages.push({
          role: "tool",
          tool_call_id: c.id,
          content: truncateForLLM(result),
        } as any);
      }
    } else {
      // Parallel path — yield all starts, await all, yield all results in order
      for (const c of prepared) {
        yield { type: "tool_start", name: c.name, args: c.args, id: c.id };
      }

      const results = await Promise.all(
        prepared.map(async (c) => {
          try {
            return await executeTool(c.name, c.args);
          } catch (e: any) {
            return `Error: ${e?.message ?? String(e)}`;
          }
        })
      );

      for (let i = 0; i < prepared.length; i++) {
        const c = prepared[i];
        const result = results[i];
        yield { type: "tool_result", name: c.name, result, id: c.id };
        messages.push({
          role: "tool",
          tool_call_id: c.id,
          content: truncateForLLM(result),
        } as any);
      }
    }

    // If we had content + tools, keep content as potential finalText but continue loop
    if (content) finalText = content;
  }

  yield { type: "done", text: finalText || "Max steps (20) reached." };
  return finalText || "Max steps (20) reached.";
}

// Convenience non-streaming wrapper
export async function runAgentOnce(prompt: string, opts?: { model?: string; maxSteps?: number }): Promise<string> {
  let out = "";
  for await (const ev of runAgent(prompt, opts)) {
    if (ev.type === "text") out += ev.delta;
    if (ev.type === "done") return ev.text;
  }
  return out;
}
