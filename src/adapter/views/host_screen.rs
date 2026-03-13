use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::time::Duration;

pub struct HostScreen {
    token: String,
    ring_id: String,
    status: String,
}

impl HostScreen {
    pub fn new(token: String, ring_id: String, status: String) -> Self {
        Self {
            token,
            ring_id,
            status,
        }
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
        let title = Paragraph::new("🥊 pqpq - Host Ring")
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
            Line::from(vec![
                Span::styled("Ring ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&self.ring_id, Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "Invitation Token: ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &self.token,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from("Share this token with your opponent:"),
            Line::from(vec![
                Span::styled("  pqpq join ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    &self.token,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled(
                &self.status,
                Style::default().fg(Color::Yellow),
            )),
        ];

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("Ring Info"));
        frame.render_widget(paragraph, chunks[1]);

        // フッター
        let footer = Paragraph::new("Press 'Esc' to go back | 'q' to quit")
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[2]);
    }

    pub fn check_exit_key() -> Result<bool> {
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.code == KeyCode::Esc
        {
            return Ok(true);
        }
        Ok(false)
    }
}
