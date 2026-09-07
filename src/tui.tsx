import React, { useState, useEffect, useRef, useMemo, useCallback, memo } from "react";
import { render, Box, Text, useInput, useApp, useStdout } from "ink";
import { runAgent } from "./agent.ts";
import { DEFAULT_MODEL } from "./llm.ts";
import { theme, ashen } from "./theme.ts";

type Msg =
  | { id: string; role: "user"; content: string }
  | { id: string; role: "assistant"; content: string }
  | { id: string; role: "thinking"; content: string }
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

const ToolPanel = memo(function ToolPanel({ msg, onToggle }: { msg: Extract<Msg, { role: "tool" }>; onToggle: (id: string) => void }) {
  const lines = useMemo(() => (msg.result || "").split("\n"), [msg.result]);
  const bytes = useMemo(() => Buffer.byteLength(msg.result || "", "utf-8"), [msg.result]);
  const argsJson = useMemo(() => JSON.stringify(msg.args, null, 2), [msg.args]);
  const argsJsonFlat = useMemo(() => JSON.stringify(msg.args), [msg.args]);
  return (
    <Box flexDirection="column" marginY={1} backgroundColor={theme.toolBg}>
      <Box>
        <Text color={theme.toolTitleText} backgroundColor={theme.toolTitleBg} bold>
          {msg.name}
        </Text>
        <Text color={theme.toolText} backgroundColor={theme.toolBg} dimColor>
          #{msg.id.slice(0, 6)} {msg.collapsed ? "▶" : "▼"} {bytes}B {lines.length}L
        </Text>
      </Box>
      {!msg.collapsed && (
        <>
          <Box>
            <Text backgroundColor={theme.toolBg} color={theme.toolText} dimColor>
              {argsJson}
            </Text>
          </Box>
          <Box flexDirection="column">
            {lines.slice(0, 200).map((line, i) => (
              <Box key={i}>
                <Text backgroundColor={theme.toolBg} color={theme.toolText}>
                  {line}
                </Text>
              </Box>
            ))}
            {lines.length > 200 && <Text color={theme.dim}>… {lines.length - 200} more lines</Text>}
          </Box>
        </>
      )}
      {msg.collapsed && (
        <Box>
          <Text backgroundColor={theme.toolBg} color={theme.toolText} dimColor>
            {argsJsonFlat.slice(0, 80)}
            {argsJsonFlat.length > 80 ? "…" : ""} → {msg.result.slice(0, 60).replace(/\n/g, " ")}
            {msg.result.length > 60 ? "…" : ""}
          </Text>
        </Box>
      )}
    </Box>
  );
});

const StreamingTail = memo(function StreamingTail({
  streamingText,
  streamingReasoning,
  status,
}: {
  streamingText: string;
  streamingReasoning: string;
  status: string;
}) {
  return (
    <Box flexDirection="column" marginY={1}>
      {streamingReasoning && (
        <Box>
          <Text color={theme.thinking} italic>
            {streamingReasoning.slice(-200)}
          </Text>
        </Box>
      )}
      {streamingText ? (
        <Box flexDirection="column">
          <Text color={theme.assistantText}>{streamingText}</Text>
        </Box>
      ) : (
        <Box>
          <Spinner />
          <Text color={theme.dim}> {status || "thinking…"}</Text>
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
  const [scrollOffset, setScrollOffset] = useState(0); // 0 = auto-follow bottom, positive = lines scrolled up
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
          // flush any pending streams
          if (flushTimerRef.current) {
            clearTimeout(flushTimerRef.current);
            flushTimerRef.current = null;
            flushStreaming();
          }
          // persist thinking between tool calls
          if (reasoningBuf) {
            addMsg({ id: Math.random().toString(36).slice(2), role: "thinking", content: reasoningBuf });
            reasoningBuf = "";
            reasoningBufRef.current = "";
            setStreamingReasoning("");
          }
          if (buffer) {
            addMsg({ id: Math.random().toString(36).slice(2), role: "assistant", content: buffer });
            buffer = "";
            textBufRef.current = "";
            setStreamingText("");
          }
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
          // persist any remaining thinking before final answer
          if (reasoningBuf) {
            addMsg({ id: Math.random().toString(36).slice(2), role: "thinking", content: reasoningBuf });
            reasoningBuf = "";
            reasoningBufRef.current = "";
          }
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

    // scroll controls
    if (key.pageUp) {
      setScrollOffset((prev) => prev + 10);
      return;
    }
    if (key.pageDown) {
      setScrollOffset((prev) => Math.max(0, prev - 10));
      return;
    }
    if (key.home) {
      setScrollOffset(messages.length * 2); // approximate max scroll
      return;
    }
    if (key.end) {
      setScrollOffset(0); // auto-follow bottom
      return;
    }

    if (key.return && !key.ctrl) {
      // Enter submits
      const toSubmit = input;
      setInput("");
      setCursor(0);
      setScrollOffset(0); // snap to bottom on submit
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

  const handleToggle = useCallback((id: string) => {
    setMessages((prev) => prev.map((x) => (x.id === id ? { ...x, collapsed: !x.collapsed } : x)));
  }, []);

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

  // Compute viewport height for scroll windowing
  const termRows = stdout.rows || 24;
  // Reserve: 2 header lines + 3 input box lines + 1 margin = 6 lines for chrome
  const contentRows = Math.max(5, termRows - 6);

  // Calculate approximate lines per message for scroll positioning
  const messageLines = useMemo(() => {
    return messages.map((m) => {
      if (m.role === "user" || m.role === "assistant") {
        const text = m.content || "";
        return Math.max(1, text.split("\n").length) + 2; // +2 for marginY=1 (2 blank lines)
      }
      if (m.role === "thinking") {
        const text = m.content || "";
        return Math.max(1, text.split("\n").length) + 2;
      }
      if (m.role === "tool") {
        if (m.collapsed) return 3; // title + summary + margin
        const resultLines = (m.result || "").split("\n").length;
        const argsLines = JSON.stringify(m.args, null, 2).split("\n").length;
        return 3 + argsLines + Math.min(200, resultLines) + 2; // header + args + result + margin
      }
      return 1;
    });
  }, [messages]);

  const totalLines = useMemo(() => messageLines.reduce((a, b) => a + b, 0), [messageLines]);

  // Streaming tail is ~1-3 lines
  const streamingLines = streamingText || streamingReasoning ? 2 : 0;
  const maxScroll = Math.max(0, totalLines + streamingLines - contentRows);

  // Clamp scrollOffset
  const clampedScroll = Math.min(scrollOffset, maxScroll);

  // If not scrolled up (auto-follow), render from end
  // Otherwise, calculate which messages to show
  const visibleMessages = useMemo(() => {
    if (clampedScroll === 0) {
      // Auto-follow: show as many messages as fit, streaming at bottom
      // Walk backwards and collect messages until we fill the viewport
      let linesNeeded = contentRows - streamingLines;
      const result: { msg: Msg; visible: boolean }[] = [];
      for (let i = messages.length - 1; i >= 0; i--) {
        const ml = messageLines[i]!;
        if (linesNeeded <= 0) break;
        result.unshift({ msg: messages[i]!, visible: true });
        linesNeeded -= ml;
      }
      return result;
    }
    // Scrolled up: skip lines from the bottom, then show viewport
    let skipLines = clampedScroll;
    let startIdx = messages.length;
    for (let i = messages.length - 1; i >= 0; i--) {
      const ml = messageLines[i]!;
      if (skipLines <= 0) {
        startIdx = i;
        break;
      }
      skipLines -= ml;
      if (skipLines <= 0) {
        startIdx = i;
        break;
      }
    }
    // Now collect messages from startIdx to fill contentRows
    let linesNeeded = contentRows;
    const result: { msg: Msg; visible: boolean }[] = [];
    for (let i = startIdx; i < messages.length && linesNeeded > 0; i++) {
      result.push({ msg: messages[i]!, visible: true });
      linesNeeded -= messageLines[i]!;
    }
    return result;
  }, [messages, messageLines, clampedScroll, contentRows, streamingLines]);



  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <Text color={theme.accent} bold>
          lean
        </Text>
        <Text color={theme.dim}> — light coding assistant • {model} • max 100 steps • {"~/.agents/skills"}</Text>
        {clampedScroll > 0 && (
          <Text color={theme.accent}> [SCROLLED ↑{clampedScroll}lines]</Text>
        )}
      </Box>

      <Box flexDirection="column" flexGrow={1}>
        {visibleMessages.map(({ msg: m }) => {
          if (m.role === "user") {
            return (
              <Box key={m.id} marginY={1}>
                <Text color={theme.userText}>{m.content}</Text>
              </Box>
            );
          }
          if (m.role === "assistant") {
            return (
              <Box key={m.id} flexDirection="column" marginY={1}>
                <Text color={theme.assistantText}>{m.content}</Text>
              </Box>
            );
          }
          if (m.role === "thinking") {
            return (
              <Box key={m.id} marginY={1}>
                <Text color={theme.thinking} italic>
                  {m.content}
                </Text>
              </Box>
            );
          }
          if (m.role === "tool") {
            return <ToolPanel key={m.id} msg={m} onToggle={handleToggle} />;
          }
          return null;
        })}
        {isStreaming && scrollOffset === 0 && (
          <StreamingTail streamingText={streamingText} streamingReasoning={streamingReasoning} status={status} />
        )}
      </Box>

      <Box borderStyle="round" borderColor={isStreaming ? theme.dim : theme.accent} flexDirection="column">
        {renderInput()}
        <Box>
          <Text color={theme.dim}>Enter submit • Ctrl+Enter newline • ↑/↓ history • PgUp/PgDn scroll • /help • c collapse • Ctrl+C quit</Text>
        </Box>
      </Box>
    </Box>
  );
}

if (import.meta.main) {
  render(<App />);
}

export default App;
