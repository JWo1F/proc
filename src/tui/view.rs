use super::app::{
  ALL_ACTION_ITEMS, AddField, App, Item, MAIN_ITEMS, MODE_ITEMS, PROCESS_ACTION_ITEMS,
  PROCESS_MODE_ITEMS, PROCESS_MODES, Screen,
};
use crate::core::manager::{OnExit, ProcessSnapshot, describe_mode, format_uptime};
use crate::core::resources::{ResourceHistory, ResourceMap};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
  Block, BorderType, Borders, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row,
  Sparkline, Table, Wrap,
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const WARN: Color = Color::Yellow;
const RUNNING: Color = Color::Green;
const MEM_COLOR: Color = Color::Magenta;
const CPU_COLOR: Color = Color::Blue;

const BOX_WIDTH: u16 = 64;
const WIDE_BOX_WIDTH: u16 = 78;
const MIN_HEIGHT: u16 = 11;
// borders(2) + padding(2) + breadcrumb(1) + separator×2(2) + footer(1)
const CHROME_HEIGHT: u16 = 8;

pub fn render(frame: &mut Frame, app: &App) {
  let outer = frame.area();
  let screen = app.stack.last().expect("stack is never empty");
  let width = match screen {
    Screen::Info { .. } | Screen::Dashboard => WIDE_BOX_WIDTH,
    _ => BOX_WIDTH,
  };
  let area = modal_rect(width, content_height(app), outer);
  frame.render_widget(Clear, area);

  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(Style::default().fg(ACCENT))
    .padding(Padding::new(2, 2, 1, 1));
  let inner = block.inner(area);
  frame.render_widget(block, area);

  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Length(1), // breadcrumb
      Constraint::Length(1), // separator
      Constraint::Min(1),    // body
      Constraint::Length(1), // separator
      Constraint::Length(1), // footer
    ])
    .split(inner);

  frame.render_widget(Paragraph::new(breadcrumb(&app.stack)), rows[0]);
  frame.render_widget(rule(rows[1].width), rows[1]);
  render_body(frame, rows[2], app);
  frame.render_widget(rule(rows[3].width), rows[3]);
  render_footer(frame, rows[4], app);
}

/// Roughly how tall the box needs to be for the current screen's content,
/// so a two-item menu doesn't sit in a box sized for a ten-item one.
fn content_height(app: &App) -> u16 {
  let rows = match app.stack.last().expect("stack is never empty") {
    Screen::Main { .. } => MAIN_ITEMS.len(),
    Screen::Processes { .. } => app.snapshot.processes.len().max(1),
    Screen::ProcessActions { .. } => PROCESS_ACTION_ITEMS.len(),
    Screen::ProcessMode { .. } => PROCESS_MODE_ITEMS.len(),
    Screen::AllActions { .. } => ALL_ACTION_ITEMS.len(),
    Screen::ModeMenu { .. } => MODE_ITEMS.len(),
    Screen::AddProcess { .. } => 6,
    // 9 text fields at most + a blank line + two 4-row graph panels.
    Screen::Info { .. } => 9 + 1 + 4 + 4,
    Screen::Ps => app.snapshot.processes.len().max(1) + 1,
    // Summary + two 4-row graph panels + blank spacer + table header + one row per process.
    Screen::Dashboard => 1 + 4 + 4 + 1 + 1 + app.snapshot.processes.len().max(1),
    Screen::ConfirmQuit => 1,
  };
  (rows as u16 + CHROME_HEIGHT).max(MIN_HEIGHT)
}

fn rule(width: u16) -> Paragraph<'static> {
  Paragraph::new("─".repeat(width as usize)).style(Style::default().fg(MUTED))
}

fn breadcrumb(stack: &[Screen]) -> Line<'static> {
  let last = stack.len() - 1;
  let mut spans = Vec::new();
  for (i, screen) in stack.iter().enumerate() {
    if i > 0 {
      spans.push(Span::styled(" › ", Style::default().fg(MUTED)));
    }
    let style = if i == last {
      Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
      Style::default().fg(MUTED)
    };
    spans.push(Span::styled(label(screen), style));
  }
  Line::from(spans)
}

fn label(screen: &Screen) -> String {
  match screen {
    Screen::Main { .. } => "Menu".to_string(),
    Screen::Processes { .. } => "Processes".to_string(),
    Screen::ProcessActions { name, .. } => name.clone(),
    Screen::ProcessMode { name, .. } => format!("{} — run mode", name),
    Screen::AllActions { .. } => "All processes".to_string(),
    Screen::ModeMenu { .. } => "Mode".to_string(),
    Screen::AddProcess { .. } => "Add process".to_string(),
    Screen::Info { name } => format!("{} — info", name),
    Screen::Ps => "Ps".to_string(),
    Screen::Dashboard => "Dashboard".to_string(),
    Screen::ConfirmQuit => "Quit".to_string(),
  }
}

/// "[P] Processes" — the bracketed letter is the direct hotkey for this item.
fn item_label((hotkey, text): Item) -> String {
  format!("[{}] {}", hotkey.to_ascii_uppercase(), text)
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
  match app.stack.last().expect("stack is never empty") {
    Screen::Main { selected } => render_list(frame, area, MAIN_ITEMS, *selected),

    Screen::Processes { selected } => {
      if app.snapshot.processes.is_empty() {
        render_empty(
          frame,
          area,
          "No processes yet — add one from the main menu.",
        );
        return;
      }
      let items: Vec<ListItem> = app
        .snapshot
        .processes
        .iter()
        .enumerate()
        .map(|(i, p)| {
          let prefix = match i {
            0..=8 => format!("[{}]", i + 1),
            _ => "   ".to_string(),
          };
          ListItem::new(format!("{} {:<16}{}", prefix, p.name, p.status))
            .style(status_style(p.status))
        })
        .collect();
      render_items(frame, area, items, *selected);
    }

    Screen::ProcessActions { selected, .. } => {
      render_list(frame, area, PROCESS_ACTION_ITEMS, *selected)
    }

    Screen::ProcessMode { name, selected } => {
      render_process_mode(frame, area, name, *selected, app)
    }

    Screen::AllActions { selected } => render_list(frame, area, ALL_ACTION_ITEMS, *selected),

    Screen::ModeMenu { selected } => {
      let items: Vec<ListItem> = MODE_ITEMS
        .iter()
        .enumerate()
        .map(|(i, item)| {
          let is_current = matches!(
            (i, app.snapshot.mode),
            (0, OnExit::Restart) | (1, OnExit::Stop) | (2, OnExit::Ignore)
          );
          let mut text = item_label(*item);
          if is_current {
            text.push_str("  (current)");
          }
          ListItem::new(text)
        })
        .collect();
      render_items(frame, area, items, *selected);
    }

    Screen::AddProcess {
      name,
      command,
      field,
    } => render_add_process(frame, area, name, command, *field),

    Screen::Info { name } => render_info(frame, area, name, app),

    Screen::Ps => render_ps(frame, area, app),

    Screen::Dashboard => render_dashboard(frame, area, app),

    Screen::ConfirmQuit => {
      let p = Paragraph::new("Stop all processes and quit?")
        .style(Style::default().fg(WARN))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
      frame.render_widget(p, area);
    }
  }
}

/// The per-process mode list, with the process's current choice marked.
fn render_process_mode(frame: &mut Frame, area: Rect, name: &str, selected: usize, app: &App) {
  let current = app
    .snapshot
    .processes
    .iter()
    .find(|p| p.name == name)
    .map(|p| p.flags.on_exit);

  let items: Vec<ListItem> = PROCESS_MODE_ITEMS
    .iter()
    .enumerate()
    .map(|(i, item)| {
      let mut text = item_label(*item);
      if current.is_some() && PROCESS_MODES.get(i).copied() == current {
        text.push_str("  (current)");
      }
      ListItem::new(text)
    })
    .collect();
  render_items(frame, area, items, selected);
}

fn status_style(status: &str) -> Style {
  match status {
    "running" => Style::default().fg(RUNNING),
    "restarting" => Style::default().fg(WARN),
    _ => Style::default().fg(MUTED),
  }
}

fn render_list(frame: &mut Frame, area: Rect, items: &[Item], selected: usize) {
  let items: Vec<ListItem> = items
    .iter()
    .map(|item| ListItem::new(item_label(*item)))
    .collect();
  render_items(frame, area, items, selected);
}

fn render_items(frame: &mut Frame, area: Rect, items: Vec<ListItem>, selected: usize) {
  let list = List::new(items)
    .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
    .highlight_symbol("▍ ");
  let mut state = ListState::default();
  state.select(Some(selected));
  frame.render_stateful_widget(list, area, &mut state);
}

fn render_empty(frame: &mut Frame, area: Rect, message: &str) {
  let p = Paragraph::new(message)
    .style(Style::default().fg(MUTED))
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true });
  frame.render_widget(p, area);
}

fn render_add_process(frame: &mut Frame, area: Rect, name: &str, command: &str, field: AddField) {
  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Length(3),
      Constraint::Length(3),
      Constraint::Min(0),
    ])
    .split(area);

  render_field(frame, rows[0], "Name", name, field == AddField::Name);
  render_field(
    frame,
    rows[1],
    "Command",
    command,
    field == AddField::Command,
  );
}

fn render_field(frame: &mut Frame, area: Rect, label: &str, value: &str, focused: bool) {
  let style = if focused {
    Style::default().fg(ACCENT)
  } else {
    Style::default().fg(MUTED)
  };
  let text = if focused {
    format!("{}▏", value)
  } else {
    value.to_string()
  };
  let block = Block::default()
    .title(Span::styled(format!(" {} ", label), style))
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(style)
    .padding(Padding::horizontal(1));
  frame.render_widget(Paragraph::new(text).block(block), area);
}

fn render_info(frame: &mut Frame, area: Rect, name: &str, app: &App) {
  let Some(proc) = app.snapshot.processes.iter().find(|p| p.name == name) else {
    render_empty(frame, area, "Process no longer exists.");
    return;
  };
  let resources = app.resources.get(name);

  let status = match proc.pid {
    Some(pid) => format!("{} (pid {})", proc.status, pid),
    None => proc.status.to_string(),
  };

  let field = |k: &str, v: String| {
    Line::from(vec![
      Span::styled(format!("{:<11}", k), Style::default().fg(MUTED)),
      Span::raw(v),
    ])
  };

  let mut lines = vec![
    field("Command", proc.command.clone()),
    field("Status", status),
  ];
  if let Some(uptime) = proc.uptime {
    lines.push(field("Uptime", format_uptime(uptime)));
  }
  lines.push(field("Restarts", proc.restarts.to_string()));
  lines.push(field(
    "Run mode",
    describe_mode(proc.flags.on_exit, app.snapshot.mode),
  ));
  let flags = proc.flags.labels();
  if !flags.is_empty() {
    lines.push(field("Flags", flags.join(", ")));
  }
  if let Some(ref last_exit) = proc.last_exit {
    lines.push(field("Last exit", last_exit.clone()));
  }
  if let Some(r) = resources {
    lines.push(field("Processes", r.current.process_count.to_string()));
    lines.push(field(
      "Threads",
      format_thread_count(r.current.thread_count),
    ));
  }

  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Length(lines.len() as u16),
      Constraint::Length(1),
      Constraint::Length(4),
      Constraint::Length(4),
      Constraint::Min(0),
    ])
    .split(area);

  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rows[0]);

  match resources {
    Some(r) => {
      let mem_mb: Vec<u64> = r.mem_history.iter().copied().collect();
      let cpu_pct: Vec<u64> = r.cpu_history.iter().copied().collect();
      render_metric_panel(
        frame,
        rows[2],
        "Memory",
        format!("{} MB", r.current.mem_bytes / (1024 * 1024)),
        Some(format!("{} MB", r.peak_mem_bytes / (1024 * 1024))),
        &mem_mb,
        MEM_COLOR,
      );
      render_metric_panel(
        frame,
        rows[3],
        "CPU",
        format!("{:.1}%", r.current.cpu_percent),
        Some(format!("{:.1}%", r.peak_cpu_percent)),
        &cpu_pct,
        CPU_COLOR,
      );
    }
    None => {
      render_metric_panel(
        frame,
        rows[2],
        "Memory",
        "warming up…".to_string(),
        None,
        &[],
        MEM_COLOR,
      );
      render_metric_panel(
        frame,
        rows[3],
        "CPU",
        "warming up…".to_string(),
        None,
        &[],
        CPU_COLOR,
      );
    }
  }
}

/// A bordered panel showing a metric's current value in the title and its
/// recent history as a sparkline underneath.
fn render_metric_panel(
  frame: &mut Frame,
  area: Rect,
  label: &str,
  current: String,
  peak: Option<String>,
  history: &[u64],
  color: Color,
) {
  let title = match peak {
    Some(peak) => format!(" {} · {}  (peak {}) ", label, current, peak),
    None => format!(" {} · {} ", label, current),
  };
  let block = Block::default()
    .title(Span::styled(
      title,
      Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(Style::default().fg(MUTED));
  let inner = block.inner(area);
  frame.render_widget(block, area);

  // One bar per data point: show only the most recent `inner.width` samples
  // so the graph always fills the panel instead of leaving empty columns on
  // the right once there's more history than the panel is wide.
  let visible = &history[history.len().saturating_sub(inner.width as usize)..];
  let sparkline = Sparkline::default()
    .data(visible)
    .style(Style::default().fg(color));
  frame.render_widget(sparkline, inner);
}

fn format_thread_count(count: usize) -> String {
  if count == 0 {
    // sysinfo can only report thread counts on Linux; elsewhere this is
    // "unknown", not "zero threads" (every live process has at least one).
    "—".to_string()
  } else {
    count.to_string()
  }
}

fn render_dashboard(frame: &mut Frame, area: Rect, app: &App) {
  if app.snapshot.processes.is_empty() {
    render_empty(frame, area, "No processes — add one from the main menu.");
    return;
  }

  let names: Vec<String> = app
    .snapshot
    .processes
    .iter()
    .map(|p| p.name.clone())
    .collect();
  let totals = |pick: fn(&ResourceHistory) -> f64| -> f64 {
    names
      .iter()
      .filter_map(|n| app.resources.get(n))
      .map(pick)
      .sum()
  };
  let total_cpu = totals(|r| r.current.cpu_percent as f64) as f32;
  let total_mem_mb = (totals(|r| r.current.mem_bytes as f64) / (1024.0 * 1024.0)) as u64;
  let total_procs = totals(|r| r.current.process_count as f64) as usize;
  // Sum of each process's own all-time peak — not the historical peak of the
  // combined total, but a reasonable "worst case across the board" reading.
  let peak_cpu = totals(|r| r.peak_cpu_percent as f64) as f32;
  let peak_mem_mb = (totals(|r| r.peak_mem_bytes as f64) / (1024.0 * 1024.0)) as u64;

  let summary = Line::from(vec![
    Span::styled("CPU ", Style::default().fg(MUTED)),
    Span::styled(
      format!("{:.1}%", total_cpu),
      Style::default().fg(CPU_COLOR).add_modifier(Modifier::BOLD),
    ),
    Span::raw("    "),
    Span::styled("Memory ", Style::default().fg(MUTED)),
    Span::styled(
      format!("{} MB", total_mem_mb),
      Style::default().fg(MEM_COLOR).add_modifier(Modifier::BOLD),
    ),
    Span::raw("    "),
    Span::styled("Processes ", Style::default().fg(MUTED)),
    Span::styled(
      format!("{} across {} groups", total_procs, names.len()),
      Style::default().add_modifier(Modifier::BOLD),
    ),
  ]);

  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
      Constraint::Length(1),
      Constraint::Length(4),
      Constraint::Length(4),
      Constraint::Length(1),
      Constraint::Min(1),
    ])
    .split(area);

  frame.render_widget(Paragraph::new(summary), rows[0]);

  let mem_hist = aggregate_history(&app.resources, &names, |r| &r.mem_history);
  let cpu_hist = aggregate_history(&app.resources, &names, |r| &r.cpu_history);
  render_metric_panel(
    frame,
    rows[1],
    "Memory (total)",
    format!("{} MB", total_mem_mb),
    Some(format!("{} MB", peak_mem_mb)),
    &mem_hist,
    MEM_COLOR,
  );
  render_metric_panel(
    frame,
    rows[2],
    "CPU (total)",
    format!("{:.1}%", total_cpu),
    Some(format!("{:.1}%", peak_cpu)),
    &cpu_hist,
    CPU_COLOR,
  );

  render_process_table(frame, rows[4], app);
}

/// Sum a per-process history metric across all groups, right-aligned so
/// samples from before a process existed count as 0 rather than misaligning
/// the series.
fn aggregate_history(
  resources: &ResourceMap,
  names: &[String],
  pick: impl Fn(&ResourceHistory) -> &std::collections::VecDeque<u64>,
) -> Vec<u64> {
  let max_len = names
    .iter()
    .filter_map(|n| resources.get(n))
    .map(|r| pick(r).len())
    .max()
    .unwrap_or(0);
  let mut result = vec![0u64; max_len];
  for name in names {
    let Some(r) = resources.get(name) else {
      continue;
    };
    let history = pick(r);
    let offset = max_len - history.len();
    for (i, value) in history.iter().enumerate() {
      result[offset + i] += value;
    }
  }
  result
}

fn render_process_table(frame: &mut Frame, area: Rect, app: &App) {
  let header = Row::new(vec!["NAME", "STATUS", "CPU%", "MEM MB", "PROCS", "THRD"])
    .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));

  let rows = app
    .snapshot
    .processes
    .iter()
    .map(|p| dashboard_row(p, app.resources.get(&p.name)));

  let widths = [
    Constraint::Length(14),
    Constraint::Length(10),
    Constraint::Length(7),
    Constraint::Length(9),
    Constraint::Length(7),
    Constraint::Length(6),
  ];

  let table = Table::new(rows, widths).header(header).column_spacing(1);
  frame.render_widget(table, area);
}

fn dashboard_row<'a>(proc: &'a ProcessSnapshot, resources: Option<&ResourceHistory>) -> Row<'a> {
  let (cpu, mem, procs, threads) = match resources {
    Some(r) => (
      format!("{:.1}", r.current.cpu_percent),
      (r.current.mem_bytes / (1024 * 1024)).to_string(),
      r.current.process_count.to_string(),
      format_thread_count(r.current.thread_count),
    ),
    None => (
      "—".to_string(),
      "—".to_string(),
      "—".to_string(),
      "—".to_string(),
    ),
  };

  Row::new(vec![
    Cell::from(proc.name.clone()),
    Cell::from(proc.status),
    Cell::from(cpu),
    Cell::from(mem),
    Cell::from(procs),
    Cell::from(threads),
  ])
  .style(status_style(proc.status))
}

fn render_ps(frame: &mut Frame, area: Rect, app: &App) {
  if app.snapshot.processes.is_empty() {
    render_empty(frame, area, "No processes — add one from the main menu.");
    return;
  }

  let header = Row::new(vec!["NAME", "STATUS", "PID", "UPTIME", "RESTARTS"])
    .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD));

  let rows = app.snapshot.processes.iter().map(|p| {
    let pid = p.pid.map_or_else(|| "—".to_string(), |pid| pid.to_string());
    let uptime = p.uptime.map_or_else(|| "—".to_string(), format_uptime);
    Row::new(vec![
      Cell::from(p.name.clone()),
      Cell::from(p.status),
      Cell::from(pid),
      Cell::from(uptime),
      Cell::from(p.restarts.to_string()),
    ])
    .style(status_style(p.status))
  });

  let widths = [
    Constraint::Length(16),
    Constraint::Length(11),
    Constraint::Length(8),
    Constraint::Length(9),
    Constraint::Length(9),
  ];

  let table = Table::new(rows, widths).header(header).column_spacing(1);
  frame.render_widget(table, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
  let hint = match app.stack.last().expect("stack is never empty") {
    Screen::AddProcess { .. } => "Tab  switch field    Enter  next / submit    Esc  cancel",
    Screen::ConfirmQuit => "Y  confirm    N / Esc  cancel",
    Screen::Info { .. } | Screen::Ps | Screen::Dashboard => "Esc  back",
    _ => "↑↓  move    Enter  select    letter  jump    Esc  back",
  };
  frame.render_widget(Paragraph::new(hint).style(Style::default().fg(MUTED)), area);
}

/// A box `width` wide and `height` tall, centered within `area` (clamped so
/// it never exceeds the available space).
fn modal_rect(width: u16, height: u16, area: Rect) -> Rect {
  let width = width.min(area.width);
  let height = height.min(area.height);
  let x = area.x + (area.width.saturating_sub(width)) / 2;
  let y = area.y + (area.height.saturating_sub(height)) / 2;
  Rect::new(x, y, width, height)
}
