import OpenAI from "openai";

export const DEFAULT_MODEL = "mimo-v2.5-free";

export const client = new OpenAI({
  apiKey: process.env.OPENCODE_API_KEY || process.env.OPENAI_API_KEY || "sk-test",
  baseURL: process.env.OPENCODE_BASE_URL || "http://127.0.0.1:8080/zen/v1",
});

export const TOOLS: OpenAI.Chat.Completions.ChatCompletionTool[] = [
  {
    type: "function",
    function: {
      name: "read_file",
      description: "Read a file from disk. Returns the file content as text.",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "File path to read" },
        },
        required: ["path"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "write_file",
      description: "Write content to a file. Creates parent directories if needed. Overwrites if exists.",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "File path to write" },
          content: { type: "string", description: "Content to write" },
        },
        required: ["path", "content"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "edit_file",
      description: "Exact text replacement in a file. oldText must match exactly once. Use for surgical edits.",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "File path to edit" },
          oldText: { type: "string", description: "Exact text to replace (must appear exactly once)" },
          newText: { type: "string", description: "Replacement text" },
        },
        required: ["path", "oldText", "newText"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "bash",
      description: "Run a shell command via bash -c. Unrestricted. Returns stdout+stderr verbatim.",
      parameters: {
        type: "object",
        properties: {
          command: { type: "string", description: "Shell command to execute" },
        },
        required: ["command"],
        additionalProperties: false,
      },
    },
  },
  {
    type: "function",
    function: {
      name: "web_search",
      description: "Search the web for information. Returns top 5 results with title, url, snippet.",
      parameters: {
        type: "object",
        properties: {
          query: { type: "string", description: "Search query" },
        },
        required: ["query"],
        additionalProperties: false,
      },
    },
  },
];
