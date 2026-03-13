use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

#[derive(Clone, Copy, PartialEq)]
pub enum MenuOption {
    HostRing,
    JoinRing,
    ListRings,
}

impl MenuOption {
    fn label(&self) -> &str {
        match self {
            MenuOption::HostRing => "リングをつくる",
            MenuOption::JoinRing => "リングをさがす",
            MenuOption::ListRings => "リングをみる",
        }
    }

    fn all() -> Vec<MenuOption> {
        vec![
            MenuOption::HostRing,
            MenuOption::JoinRing,
            MenuOption::ListRings,
        ]
    }
}

pub struct MenuScreen {
    selected: usize,
}

impl MenuScreen {
    pub fn new() -> Self {
        Self { selected: 0 }
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(area);

        // タイトル
        let title = Paragraph::new("🥊 pqpq - Terminal P2P Fighting Game")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(title, chunks[0]);

        // メニューオプション
        let options = MenuOption::all();
        let items: Vec<ListItem> = options
            .iter()
            .enumerate()
            .map(|(i, option)| {
                let style = if i == self.selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let prefix = if i == self.selected { "▶ " } else { "  " };
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(option.label(), style),
                ]))
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("メニュー"));
        frame.render_widget(list, chunks[1]);

        // フッター
        let footer = Paragraph::new("↑↓: 選択 | Enter: 決定 | q: 終了")
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[2]);
    }

    pub fn handle_input(&mut self) -> Result<MenuAction> {
        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Up => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    let options = MenuOption::all();
                    if self.selected < options.len() - 1 {
                        self.selected += 1;
                    }
                }
                KeyCode::Enter => {
                    let options = MenuOption::all();
                    return Ok(MenuAction::Select(options[self.selected]));
                }
                KeyCode::Char('q') => {
                    return Ok(MenuAction::Exit);
                }
                _ => {}
            }
        }
        Ok(MenuAction::None)
    }
}

pub enum MenuAction {
    None,
    Select(MenuOption),
    Exit,
}
