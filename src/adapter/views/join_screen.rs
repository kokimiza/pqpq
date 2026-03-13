use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::time::Duration;

pub struct JoinScreen {
    token_input: String,
    cursor_position: usize,
}

impl JoinScreen {
    pub fn new() -> Self {
        Self {
            token_input: String::new(),
            cursor_position: 0,
        }
    }

    pub fn reset(&mut self) {
        self.token_input.clear();
        self.cursor_position = 0;
    }

    pub fn render(&self, frame: &mut Frame) {
        let size = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(size);

        // タイトル
        let title = Paragraph::new("🥊 pqpq - Join Ring")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(title, chunks[0]);

        // メインコンテンツ
        let content = vec![
            Line::from(""),
            Line::from(Span::styled(
                "招待トークンを入力してください",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("Token: ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    &self.token_input,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("_", Style::default().fg(Color::Gray)),
            ]),
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled(
                "Enter を押して参加",
                Style::default().fg(Color::Gray),
            )),
        ];

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Join Ring"));
        frame.render_widget(paragraph, chunks[1]);

        // フッター
        let footer = Paragraph::new("Enter: 参加 | Esc: 戻る")
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[2]);
    }

    pub fn handle_input(&mut self) -> Result<JoinAction> {
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char(c) => {
                    self.token_input.insert(self.cursor_position, c);
                    self.cursor_position += 1;
                }
                KeyCode::Backspace => {
                    if self.cursor_position > 0 {
                        self.cursor_position -= 1;
                        self.token_input.remove(self.cursor_position);
                    }
                }
                KeyCode::Left => {
                    if self.cursor_position > 0 {
                        self.cursor_position -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor_position < self.token_input.len() {
                        self.cursor_position += 1;
                    }
                }
                KeyCode::Enter => {
                    if !self.token_input.is_empty() {
                        return Ok(JoinAction::Submit(self.token_input.clone()));
                    }
                }
                KeyCode::Esc => {
                    return Ok(JoinAction::Back);
                }
                _ => {}
            }
        }
        Ok(JoinAction::None)
    }
}

pub enum JoinAction {
    None,
    Submit(String),
    Back,
}
