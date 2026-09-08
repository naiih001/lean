use crate::agent::{self, AgentEvent};
use crate::theme::{ASHEN, THEME};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
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

async fn app_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<Stdout>>,
    model: String,
) -> anyhow::Result<()> {
    let mut messages: Vec<Msg> = Vec::new();
    let mut scroll: usize = 0;
    let mut status = String::from("Ready");
    let mut cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut ctx_info = String::from("ctx: 0 msgs");
    let mut running: Option<tokio::task::JoinHandle<()>> = None;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    // history for Up/Down
    let mut history: Vec<String> = Vec::new();
    let mut hist_idx: Option<usize> = None;
    let mut input_text = String::new();

    loop {
        // update ctx_info
        ctx_info = format!("ctx: {} msgs, {} chars", messages.len(), messages.iter().map(|m| m.content.len()).sum::<usize>());

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(5),
                    Constraint::Length(5),
                    Constraint::Length(1),
                ])
                .split(f.area());

            // List
            let items: Vec<ListItem> = messages
                .iter()
                .map(|m| {
                    let color = match m.role.as_str() {
                        "user" => ASHEN.ember,
                        "tool" => Color::Rgb(0x28, 0x2e, 0x28),
                        _ => Color::White,
                    };
                    let style = Style::default().fg(color);
                    // truncate display to 2000 chars
                    let content = if m.content.len() > 2000 {
                        format!("{}… [truncated {} chars]", &m.content[..2000], m.content.len() - 2000)
                    } else {
                        m.content.clone()
                    };
                    let line = Line::from(vec![
                        Span::styled(format!("[{}] ", m.role), style),
                        Span::raw(content),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let mut list_state = ratatui::widgets::ListState::default();
            // simple scroll: show from scroll offset
            let visible: Vec<ListItem> = if scroll < items.len() {
                items.into_iter().skip(scroll).collect()
            } else {
                vec![]
            };
            let list = List::new(visible)
                .block(Block::default().borders(Borders::ALL).title(" lean "));
            f.render_stateful_widget(list, chunks[0], &mut list_state);

            // Input
            let para = Paragraph::new(input_text.as_str())
                .block(Block::default().borders(Borders::ALL).title(" Input "));
            f.render_widget(para, chunks[1]);

            // Footer
            let footer = Paragraph::new(Line::from(vec![
                Span::styled(format!(" model: {} ", model), Style::default().fg(ASHEN.ember)),
                Span::raw(format!("| dir: {} ", cwd)),
                Span::raw(format!("| {} ", ctx_info)),
                Span::styled(status.clone(), Style::default().fg(ASHEN.frost)),
            ]))
            .style(Style::default().bg(THEME.page_bg));
            f.render_widget(footer, chunks[2]);
        })?;

        // handle events with timeout to also poll rx
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
                                }
                                "/help" => {
                                    messages.push(Msg { role: "system".into(), content: "Commands: /help /model <name> /clear /exit. Enter send, Shift+Enter newline, Up/Down history, Esc quit, PgUp/PgDn scroll".into() });
                                }
                                _ if prompt.starts_with("/model ") => {
                                    let m = prompt.strip_prefix("/model ").unwrap().trim();
                                    messages.push(Msg { role: "system".into(), content: format!("Model switch requested: {} (restart to apply)", m) });
                                }
                                _ => {
                                    messages.push(Msg { role: "system".into(), content: format!("Unknown command: {}", prompt) });
                                }
                            }
                            input_text.clear();
                            hist_idx = None;
                            continue;
                        }
                        // push user message
                        history.push(prompt.clone());
                        hist_idx = None;
                        messages.push(Msg { role: "user".into(), content: prompt.clone() });
                        status = "Thinking…".into();
                        input_text.clear();

                        // spawn agent task
                        let tx_clone = tx.clone();
                        let model_clone = model.clone();
                        let prompt_clone = prompt.clone();
                        tokio::spawn(async move {
                            let mut stream = agent::run_agent(prompt_clone, model_clone, 100);
                            // need to box stream: run_agent returns impl Stream, we can poll
                            use futures::StreamExt;
                            let mut s = Box::pin(stream);
                            while let Some(ev) = s.next().await {
                                let _ = tx_clone.send(ev);
                            }
                        });
                    }
                    KeyCode::Up => {
                        if history.is_empty() { continue; }
                        let idx = hist_idx.map(|i| if i == 0 { 0 } else { i - 1 }).unwrap_or(history.len() - 1);
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
                        scroll = scroll.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        scroll = (scroll + 10).min(messages.len().saturating_sub(1));
                    }
                    KeyCode::Home => {
                        scroll = 0;
                    }
                    KeyCode::End => {
                        scroll = messages.len().saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }

        // poll agent events without blocking
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AgentEvent::Text { delta } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "assistant" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg { role: "assistant".into(), content: delta });
                        }
                    } else {
                        messages.push(Msg { role: "assistant".into(), content: delta });
                    }
                    status = "Streaming…".into();
                }
                AgentEvent::Reasoning { delta } => {
                    if let Some(last) = messages.last_mut() {
                        if last.role == "thinking" {
                            last.content.push_str(&delta);
                        } else {
                            messages.push(Msg { role: "thinking".into(), content: delta });
                        }
                    } else {
                        messages.push(Msg { role: "thinking".into(), content: delta });
                    }
                }
                AgentEvent::TextDone { text } => {
                    // already streamed, no-op
                    let _ = text;
                }
                AgentEvent::ToolStart { name, args, id: _ } => {
                    messages.push(Msg { role: "tool".into(), content: format!("{} {}", name, serde_json::to_string(&args).unwrap_or_default()) });
                }
                AgentEvent::ToolResult { name, result, id: _ } => {
                    let display = if result.len() > 2000 {
                        format!("{}… [truncated {} chars]", &result[..2000], result.len() - 2000)
                    } else {
                        result
                    };
                    messages.push(Msg { role: "tool".into(), content: format!("{} → {}", name, display) });
                    // update cwd after bash
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
            // auto-scroll to bottom when new messages arrive
            scroll = messages.len().saturating_sub(1);
        }
    }

    Ok(())
}
