import React, { useState, useEffect, useRef, memo } from "react";
import { render, Box, Text, useInput, useApp, useStdout } from "ink";
import { runAgent } from "./agent.ts";
import { DEFAULT_MODEL } from "./llm.ts";
import { theme, ashen } from "./theme.ts";

type Msg =
  | { id: string; role: "user"; content: string }
  | { id: string; role: "assistant"; content: string; thinking?: string }
  | { id: string; role: "tool"; name: string; args: any; result: string; collapsed: boolean };

function Spinner() {
  const [frame, setFrame] = useState(0);
  const frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
  useEffect(() => {
    const id = setInterval(() => setFrame((f) => (f + 1) % frames.length), 80);
    return () => clearInterval(id);
  }, []);
  return <Text color={theme.accent}>{frames[frame]}</Text>;
}

const ToolPanel = memo(function ToolPanel({ msg, onToggle }: { msg: Extract<Msg, { role: "tool" }>; onToggle: () => void }) {
  const bytes = Buffer.byteLength(msg.result || "", "utf-8");
  const lines = (msg.result || "").split("\n").length;
  return (
    <Box flexDirection="column" marginY={1} backgroundColor={theme.toolBg}>
      <Box>
        <Text color={theme.toolTitleText} backgroundColor={theme.toolTitleBg} bold>
          {msg.name}
        </Text>
        <Text color={theme.toolText} backgroundColor={theme.toolBg} dimColor>
          #{msg.id.slice(0, 6)} {msg.collapsed ? "▶" : "▼"} {bytes}B {lines}L
        </Text>
      </Box>
      {!msg.collapsed && (
        <>
          <Box>
            <Text backgroundColor={theme.toolBg} color={theme.toolText} dimColor>
              {JSON.stringify(msg.args, null, 2)}
            </Text>
          </Box>
          <Box flexDirection="column">
            {msg.result.split("\n").slice(0, 200).map((line, i) => (
              <Box key={i}>
                <Text backgroundColor={theme.toolBg} color={theme.toolText}>
                  {line}
                </Text>
              </Box>
            ))}
            {lines > 200 && <Text color={theme.dim}>… {lines - 200} more lines</Text>}
          </Box>
        </>
      )}
      {msg.collapsed && (
        <Box>
          <Text backgroundColor={theme.toolBg} color={theme.toolText} dimColor>
            {JSON.stringify(msg.args).slice(0, 80)}
            {JSON.stringify(msg.args).length > 80 ? "…" : ""} → {msg.result.slice(0, 60).replace(/\n/g, " ")}
            {msg.result.length > 60 ? "…" : ""}
          </Text>
        </Box>
      )}
    </Box>
  );
});

function App() {
  const { exit } = useApp();
  const { stdout } = useStdout();
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [cursor, setCursor] = useState(0);
  const [history, setHistory] = useState<string[]>([]);
  const [hIdx, setHIdx] = useState(-1);
  const [model, setModel] = useState(DEFAULT_MODEL);
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingText, setStreamingText] = useState("");
  const [streamingReasoning, setStreamingReasoning] = useState("");
  const [status, setStatus] = useState("");
  const textBufRef = useRef("");
  const reasoningBufRef = useRef("");
  const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const flushStreaming = () => {
    setStreamingText(textBufRef.current);
    setStreamingReasoning(reasoningBufRef.current.slice(-500));
  };
  const scheduleFlush = () => {
    if (flushTimerRef.current) return;
    flushTimerRef.current = setTimeout(() => {
      flushStreaming();
      flushTimerRef.current = null;
    }, 40);
  };

  const addMsg = (m: Msg) => setMessages((prev) => [...prev, m]);

  const handleSubmit = async (text: string) => {
    const prompt = text.trim();
    if (!prompt) return;
    // commands
    if (prompt.startsWith("/")) {
      const [cmd, ...rest] = prompt.slice(1).split(/\s+/);
      const arg = rest.join(" ").trim();
      if (cmd === "help") {
        addMsg({
          id: Math.random().toString(36).slice(2),
          role: "assistant",
          content: `Commands:\n/help — this help\n/model <name> — show/switch model (current: ${model})\n/clear — clear history\n/new — new session (clear)\n/exit, /quit, /q — quit\n\nKeys: ↑/↓ history, Ctrl+J newline, Enter submit, c collapse/expand tools, Ctrl+C quit`,
        });
        return;
      }
      if (cmd === "model") {
        if (arg) {
          setModel(arg);
          addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: `Model → ${arg}` });
        } else {
          addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: `Current model: ${model}` });
        }
        return;
      }
      if (cmd === "clear" || cmd === "new") {
        setMessages([]);
        setStreamingText("");
        setStreamingReasoning("");
        setStatus("");
        return;
      }
      if (cmd === "exit" || cmd === "quit" || cmd === "q") {
        exit();
        return;
      }
      // unknown command
      addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: `Unknown command: /${cmd}. Try /help` });
      return;
    }

    // normal prompt
    setHistory((h) => [...h, prompt]);
    setHIdx(-1);
    addMsg({ id: Math.random().toString(36).slice(2), role: "user", content: prompt });
    setIsStreaming(true);
    textBufRef.current = "";
    reasoningBufRef.current = "";
    setStreamingText("");
    setStreamingReasoning("");
    setStatus("thinking…");

    let buffer = "";
    let reasoningBuf = "";
    try {
      for await (const ev of runAgent(prompt, { model })) {
        if (ev.type === "text") {
          buffer += ev.delta;
          textBufRef.current = buffer;
          reasoningBufRef.current = reasoningBuf;
          scheduleFlush();
          if (status) setStatus("");
        } else if (ev.type === "reasoning") {
          reasoningBuf += ev.delta;
          reasoningBufRef.current = reasoningBuf;
          textBufRef.current = buffer;
          scheduleFlush();
        } else if (ev.type === "text_done") {
          buffer = ev.text;
          textBufRef.current = buffer;
          // immediate flush for text_done
          if (flushTimerRef.current) {
            clearTimeout(flushTimerRef.current);
            flushTimerRef.current = null;
          }
          flushStreaming();
        } else if (ev.type === "tool_start") {
          // flush any pending text as assistant msg before tool
          if (flushTimerRef.current) {
            clearTimeout(flushTimerRef.current);
            flushTimerRef.current = null;
            flushStreaming();
          }
          if (buffer) {
            addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: buffer });
            buffer = "";
            textBufRef.current = "";
            setStreamingText("");
          }
          // add placeholder tool msg (streaming)
          setStatus(`running ${ev.name}…`);
        } else if (ev.type === "tool_result") {
          addMsg({
            id: ev.id,
            role: "tool",
            name: ev.name,
            args: ev.args,
            result: ev.result,
            collapsed: false,
          });
          setStatus("");
        } else if (ev.type === "done") {
          if (flushTimerRef.current) {
            clearTimeout(flushTimerRef.current);
            flushTimerRef.current = null;
          }
          flushStreaming();
          if (buffer || ev.text) {
            const final = ev.text || buffer;
            if (final) addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: final });
          }
          buffer = "";
          textBufRef.current = "";
          reasoningBufRef.current = "";
          setStreamingText("");
          setStreamingReasoning("");
          setStatus("");
        }
      }
    } catch (e: any) {
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
      addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: `Error: ${e?.message ?? String(e)}` });
    } finally {
      if (flushTimerRef.current) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
      setIsStreaming(false);
      textBufRef.current = "";
      reasoningBufRef.current = "";
      setStreamingText("");
      setStreamingReasoning("");
      setStatus("");
    }
  };

  useInput((inputChar, key) => {
    // global toggles
    if (key.ctrl && inputChar === "c") {
      // Ink handles exit via useApp, but we also want to allow copy? For now, exit if not streaming
      if (!isStreaming) exit();
      return;
    }
    if (inputChar === "c" && !isStreaming && messages.some((m) => m.role === "tool")) {
      // collapse toggle via 'c' when not typing? But we are typing — ambiguous.
      // We will handle 'c' as collapse only when input is empty
      if (input === "") {
        setMessages((prev) => prev.map((m) => (m.role === "tool" ? { ...m, collapsed: !m.collapsed } : m)));
        return;
      }
    }

    if (key.return && !key.ctrl) {
      // Enter submits
      const toSubmit = input;
      setInput("");
      setCursor(0);
      handleSubmit(toSubmit);
      return;
    }
    if (key.return && key.ctrl) {
      // Ctrl+Enter inserts newline
      const next = input.slice(0, cursor) + "\n" + input.slice(cursor);
      setInput(next);
      setCursor(cursor + 1);
      return;
    }
    // history navigation
    if (key.upArrow) {
      if (history.length === 0) return;
      const nextIdx = hIdx === -1 ? history.length - 1 : Math.max(0, hIdx - 1);
      setHIdx(nextIdx);
      const h = history[nextIdx] || "";
      setInput(h);
      setCursor(h.length);
      return;
    }
    if (key.downArrow) {
      if (hIdx === -1) return;
      const nextIdx = hIdx + 1;
      if (nextIdx >= history.length) {
        setHIdx(-1);
        setInput("");
        setCursor(0);
      } else {
        setHIdx(nextIdx);
        const h = history[nextIdx] || "";
        setInput(h);
        setCursor(h.length);
      }
      return;
    }
    if (key.backspace || key.delete) {
      if (cursor > 0) {
        const next = input.slice(0, cursor - 1) + input.slice(cursor);
        setInput(next);
        setCursor(cursor - 1);
      }
      return;
    }
    if (key.leftArrow) {
      setCursor((c) => Math.max(0, c - 1));
      return;
    }
    if (key.rightArrow) {
      setCursor((c) => Math.min(input.length, c + 1));
      return;
    }
    // regular char
    if (inputChar && !key.ctrl && !key.meta) {
      const next = input.slice(0, cursor) + inputChar + input.slice(cursor);
      setInput(next);
      setCursor(cursor + 1);
    }
  });

  const renderInput = () => {
    const before = input.slice(0, cursor);
    const at = input.slice(cursor, cursor + 1) || " ";
    const after = input.slice(cursor + 1);
    // handle multiline display: split by \n
    const lines = input.split("\n");
    // For cursor, find which line it's on
    let pos = 0;
    let cursorLine = 0;
    let cursorCol = 0;
    for (let i = 0; i < lines.length; i++) {
      const len = lines[i]!.length + 1; // +1 for \n
      if (pos + len > cursor) {
        cursorLine = i;
        cursorCol = cursor - pos;
        break;
      }
      pos += len;
    }
    return (
      <Box flexDirection="column">
        {lines.map((line, idx) => {
          const isCursorLine = idx === cursorLine;
          if (!isCursorLine) {
            return (
              <Text key={idx}>
                {idx === 0 ? "> " : "  "}
                {line}
              </Text>
            );
          }
          const beforeC = line.slice(0, cursorCol);
          const cur = line.slice(cursorCol, cursorCol + 1) || " ";
          const afterC = line.slice(cursorCol + 1);
          return (
            <Text key={idx}>
              {idx === 0 ? "> " : "  "}
              {beforeC}
              <Text inverse>{cur}</Text>
              {afterC}
            </Text>
          );
        })}
      </Box>
    );
  };

  return (
    <Box flexDirection="column" backgroundColor={theme.pageBg}>
      <Box marginBottom={1}>
        <Text color={theme.accent} bold>
          lean
        </Text>
        <Text color={theme.dim}> — light coding assistant • {model} • max 20 steps • {"~/.agents/skills"}</Text>
      </Box>

      <Box flexDirection="column" flexGrow={1}>
        {messages.map((m) => {
          if (m.role === "user") {
            return (
              <Box key={m.id} marginY={1} backgroundColor={theme.userBg}>
                <Text color={theme.userText}>{m.content}</Text>
              </Box>
            );
          }
          if (m.role === "assistant") {
            return (
              <Box key={m.id} flexDirection="column" marginY={1} backgroundColor={theme.assistantBg}>
                <Text color={theme.assistantText}>{m.content}</Text>
              </Box>
            );
          }
          if (m.role === "tool") {
            return <ToolPanel key={m.id} msg={m} onToggle={() => setMessages((prev) => prev.map((x) => (x.id === m.id ? { ...x, collapsed: !x.collapsed } : x)))} />;
          }
          return null;
        })}
        {isStreaming && (
          <Box flexDirection="column" marginY={1}>
            {streamingReasoning && (
              <Box>
                <Text color={theme.thinking} italic>
                  {streamingReasoning.slice(-200)}
                </Text>
              </Box>
            )}
            {streamingText ? (
              <Box flexDirection="column" backgroundColor={theme.assistantBg}>
                <Text color={theme.assistantText}>{streamingText}</Text>
              </Box>
            ) : (
              <Box>
                <Spinner />
                <Text color={theme.dim}> {status || "thinking…"}</Text>
              </Box>
            )}
          </Box>
        )}
      </Box>

      <Box borderStyle="round" borderColor={isStreaming ? theme.dim : theme.accent} flexDirection="column">
        {renderInput()}
        <Box>
          <Text color={theme.dim}>Enter submit • Ctrl+Enter newline • ↑/↓ history • /help • c collapse tools • Ctrl+C quit</Text>
        </Box>
      </Box>
    </Box>
  );
}

if (import.meta.main) {
  render(<App />);
}

export default App;
