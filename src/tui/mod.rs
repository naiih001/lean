use crate::agent::{self, AgentEvent};
use crate::theme::{ASHEN, THEME};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use std::io::Stdout;

pub async fn run(model: String) -> anyhow::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let res = app_loop(&mut terminal, model).await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = res {
        eprintln!("TUI error: {}", e);
    }
    Ok(())
}

struct Msg {
    role: String,
    content: String,
}

impl Msg {
    /// Render this message as styled ratatui Lines.
    fn render_lines(&self) -> Vec<Line<'static>> {
        match self.role.as_str() {
            "user" => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(ASHEN.bone),
                    ))
                })
                .collect(),
            "assistant" => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(ASHEN.bone),
                    ))
                })
                .collect(),
            "thinking" => {
                let mut lines = vec![Line::from(Span::styled(
                    "Thinking...".to_string(),
                    Style::default()
                        .fg(ASHEN.frost)
                        .add_modifier(Modifier::ITALIC),
                ))];
                for l in self.content.lines().take(3) {
                    lines.push(Line::from(Span::styled(
                        l.to_string(),
                        Style::default()
                            .fg(ASHEN.deep_ash)
                            .add_modifier(Modifier::ITALIC),
                    )));
                }
                lines
            }
            "tool" => self.render_tool_lines(),
            _ => self
                .content
                .lines()
                .map(|l| {
                    Line::from(Span::styled(
                        l.to_string(),
                        Style::default().fg(ASHEN.frost),
                    ))
                })
                .collect(),
        }
    }

    /// Render tool messages with styled name, path/command, and result.
    fn render_tool_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(pos) = self.content.find(" → ") {
            // ── Tool result ──
            let name = self.content[..pos].trim();
            let result = &self.content[pos + " → ".len()..];

            lines.push(Line::from(Span::styled(
                format!("{} ✓", name),
                Style::default().fg(ASHEN.ember),
            )));

            let result_lines: Vec<&str> = result.lines().collect();
            let show = result_lines.len().min(5);
            for l in &result_lines[..show] {
                lines.push(Line::from(Span::styled(
                    format!("  {}", l),
                    Style::default().fg(ASHEN.smoke),
                )));
            }
            if result_lines.len() > 5 {
                lines.push(Line::from(Span::styled(
                    format!(
                        "  ... ({} earlier lines, ctrl+o to expand)",
                        result_lines.len()
                    ),
                    Style::default().fg(ASHEN.deep_ash),
                )));
            }
        } else {
            // ── Tool start ──
            let first_space = self.content.find(' ');
            if let Some(pos) = first_space {
                let name = &self.content[..pos];
                let args_str = self.content[pos..].trim();

                lines.push(Line::from(Span::styled(
                    name.to_string(),
                    Style::default().fg(ASHEN.ember),
                )));

                // Try to pretty-print known JSON arg shapes
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(args_str) {
                    if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
                        let home = std::env::var("HOME").unwrap_or_default();
                        let display = if let Some(rest) = path.strip_prefix(&home) {
                            format!("~{}", rest)
                        } else {
                            path.to_string()
                        };
                        lines.push(Line::from(Span::styled(
                            format!("  {}", display),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("  $ {}", cmd),
                            Style::default().fg(ASHEN.light_ash),
                        )));
                    } else if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", url),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else if let Some(q) = v.get("query").and_then(|q| q.as_str()) {
                        lines.push(Line::from(Span::styled(
                            format!("  \"{}\"", q),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", args_str),
                            Style::default().fg(ASHEN.smoke),
                        )));
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", args_str),
                        Style::default().fg(ASHEN.smoke),
                    )));
                }
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines
    }
}

/// Count the rendered lines for all messages (for scroll calculations).
fn total_rendered_lines(messages: &[Msg]) -> usize {
    messages.iter().map(|m| m.render_lines().len() + 1).sum()
}

async fn app_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<Stdout>>,
    model: String,
) -> anyhow::Result<()> {
    let mut messages: Vec<Msg> = Vec::new();
    let mut scroll: u16 = 0;
    let mut auto_scroll = true;
    let mut status = String::from("Ready");
    let mut cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut ctx_info = String::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // input history
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut input_text = String::new();

    loop {
        ctx_info = format!("{} msgs", messages.len());

        let term_size = terminal.size()?;
        let viewport_height = term_size.height.saturating_sub(2).max(1) as usize;

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),   // main content – fills remaining space
                    Constraint::Length(1), // input line
                    Constraint::Length(1), // footer
                ])
                .split(f.area());

            // ── Build all message lines ──
            let mut all_lines: Vec<Line<'static>> = Vec::new();
            for m in &messages {
                all_lines.extend(m.render_lines());
                all_lines.push(Line::from("")); // blank separator
            }

            let total_lines = all_lines.len();

            if auto_scroll {
                scroll = total_lines.saturating_sub(viewport_height) as u16;
            }

            // ── Main content: no borders, dark page background ──
            let content_block = Block::default().style(Style::default().bg(THEME.page_bg));
            let para = Paragraph::new(all_lines)
                .block(content_block)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0));
            f.render_widget(para, chunks[0]);

            // ── Input: single line, minimal, no border ──
            let input_line = if input_text.is_empty() {
                Line::from(Span::styled(" ", Style::default().fg(ASHEN.deep_ash)))
            } else {
                Line::from(Span::styled(
                    input_text.as_str(),
                    Style::default().fg(ASHEN.smoke),
                ))
            };
            let input_para = Paragraph::new(input_line)
                .style(Style::default().bg(THEME.page_bg));
            f.render_widget(input_para, chunks[1]);

            // ── Footer: muted, low contrast ──
            let footer_line = Line::from(vec![
                Span::styled(
                    format!(" {} ", model),
                    Style::default().fg(ASHEN.deep_ash),
                ),
                Span::styled(
                    format!(" {} ", cwd),
                    Style::default().fg(ASHEN.deep_ash),
                ),
                Span::styled(
                    format!(" {} ", ctx_info),
                    Style::default().fg(ASHEN.deep_ash),
                ),
                Span::styled(
                    status.clone(),
                    Style::default().fg(ASHEN.deep_ash),
                ),
            ]);
            let footer = Paragraph::new(footer_line)
                .style(Style::default().bg(THEME.page_bg));
            f.render_widget(footer, chunks[2]);
        })?;

        // ── Handle keyboard events ──
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(k) = event::read()? {
                match k.code {
                    KeyCode::Esc => break,
                    KeyCode::Enter if k.modifiers.contains(KeyModifiers::SHIFT) => {
                        input_text.push('\n');
                    }
                    KeyCode::Enter => {
                        let prompt = input_text.trim().to_string();
                        if prompt.is_empty() {
                            continue;
                        }
                        if prompt.starts_with('/') {
                            match prompt.as_str() {
                                "/exit" | "/quit" => break,
                                "/clear" => {
                                    messages.clear();
                                    scroll = 0;
                                    auto_scroll = true;
                                }
                                "/help" => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: "Commands: /help /model <name> /clear /exit. Enter send, Shift+Enter newline, Up/Down history, Esc quit, PgUp/PgDn scroll".into(),
                                    });
                                }
                                _ if prompt.starts_with("/model ") => {
                                    let m = prompt.strip_prefix("/model ").unwrap().trim();
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("Model: {} (restart to apply)", m),
                                    });
                                }
                                _ => {
                                    messages.push(Msg {
                                        role: "system".into(),
                                        content: format!("Unknown: {}", prompt),
                                    });
                                }
                            }
                            input_text.clear();
                            hist_idx = None;
                            continue;
                        }
                        // Send user message
                        history.push(prompt.clone());
                        hist_idx = None;
                        messages.push(Msg {
                            role: "user".into(),
                            content: prompt.clone(),
                        });
                        status = "Thinking…".into();
                        input_text.clear();
                        auto_scroll = true;

                        let tx_clone = tx.clone();
                        let model_clone = model.clone();
                        tokio::spawn(async move {
                            let stream = agent::run_agent(prompt, model_clone, 100);
                            use futures::StreamExt;
                            let mut s = Box::pin(stream);
                            while let Some(ev) = s.next().await {
                                let _ = tx_clone.send(ev);
                            }
                        });
                    }
                    KeyCode::Up => {
                        if history.is_empty() {
                            continue;
                        }
                        let idx = hist_idx
                            .map(|i| if i == 0 { 0 } else { i - 1 })
                            .unwrap_or(history.len() - 1);
                        hist_idx = Some(idx);
                        input_text = history[idx].clone();
                    }
                    KeyCode::Down => {
                        if let Some(idx) = hist_idx {
                            if idx + 1 < history.len() {
                                hist_idx = Some(idx + 1);
                                input_text = history[idx + 1].clone();
                            } else {
                                hist_idx = None;
                                input_text.clear();
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        input_text.push(c);
                    }
                    KeyCode::Backspace => {
                        input_text.pop();
                    }
                    KeyCode::PageUp => {
                        scroll = scroll.saturating_sub(viewport_height as u16);
                        auto_scroll = false;
                    }
                    KeyCode::PageDown => {
                        let total = total_rendered_lines(&messages);
                        let max_scroll = total.saturating_sub(viewport_height) as u16;
                        scroll = (scroll + viewport_height as u16).min(max_scroll);
                        if scroll >= max_scroll {
                            auto_scroll = true;
                        }
                    }
                    KeyCode::Home => {
                        scroll = 0;
                        auto_scroll = false;
                    }
                    KeyCode::End => {
                        auto_scroll = true;
                    }
                    _ => {}
                }
            }
        }

        // ── Poll agent events (non-blocking) ──
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AgentEvent::Text { delta } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "assistant" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg {
                                role: "assistant".into(),
                                content: delta,
                            });
                        }
                    } else {
                        messages.push(Msg {
                            role: "assistant".into(),
                            content: delta,
                        });
                    }
                    status = "Streaming…".into();
                }
                AgentEvent::Reasoning { delta } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "thinking" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg {
                                role: "thinking".into(),
                                content: delta,
                            });
                        }
                    } else {
                        messages.push(Msg {
                            role: "thinking".into(),
                            content: delta,
                        });
                    }
                }
                AgentEvent::TextDone { text } => {
                    let _ = text;
                }
                AgentEvent::ToolStart { name, args, id: _ } => {
                    messages.push(Msg {
                        role: "tool".into(),
                        content: format!(
                            "{} {}",
                            name,
                            serde_json::to_string(&args).unwrap_or_default()
                        ),
                    });
                }
                AgentEvent::ToolResult { name, result, id: _ } => {
                    let truncated: String = result.chars().take(2000).collect();
                    let display = if result.chars().count() > 2000 {
                        format!(
                            "{}… [truncated {} chars]",
                            truncated,
                            result.chars().count() - 2000
                        )
                    } else {
                        result
                    };
                    messages.push(Msg {
                        role: "tool".into(),
                        content: format!("{} → {}", name, display),
                    });
                    if name == "bash" {
                        if let Ok(new_cwd) = std::env::current_dir() {
                            cwd = new_cwd.display().to_string();
                        }
                    }
                }
                AgentEvent::Step { n } => {
                    status = format!("Step {}", n);
                }
                AgentEvent::Done { text } => {
                    status = "Done".into();
                    let _ = text;
                }
            }
            auto_scroll = true;
        }
    }

    Ok(())
}
