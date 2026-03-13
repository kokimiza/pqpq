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

pub struct LoadingScreen {
    title: String,
}

impl LoadingScreen {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
        }
    }

    pub fn render(&self, frame: &mut Frame, messages: &[String]) {
        let frames = vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame_idx = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            / 100) as usize
            % frames.len();

        // エラーチェック
        let has_error = messages.iter().any(|m| m.starts_with("ERROR["));

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
        let title_text = if has_error {
            format!("🥊 {} - Error Occurred", self.title)
        } else {
            format!("🥊 {} - Processing", self.title)
        };
        let title = Paragraph::new(title_text)
            .style(
                Style::default()
                    .fg(if has_error { Color::Red } else { Color::Cyan })
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .style(Style::default().fg(if has_error { Color::Red } else { Color::White })),
            );
        frame.render_widget(title, chunks[0]);

        // メインコンテンツ
        let mut content = vec![Line::from("")];

        for msg in messages {
            if msg.starts_with("ERROR[") {
                // エラーメッセージを赤で表示
                content.push(Line::from(vec![
                    Span::styled(
                        "  ✗ ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(msg, Style::default().fg(Color::Red)),
                ]));
            } else if msg.starts_with("TOKEN:") || msg.starts_with("RING_ID:") {
                // 内部データは表示しない
                continue;
            } else {
                content.push(Line::from(vec![
                    Span::styled("  ✓ ", Style::default().fg(Color::Green)),
                    Span::raw(msg),
                ]));
            }
        }

        // アニメーションスピナー（エラーがない場合のみ）
        if !has_error {
            content.push(Line::from(""));
            content.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", frames[frame_idx]),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled("Processing...", Style::default().fg(Color::Yellow)),
            ]));
        } else {
            content.push(Line::from(""));
            content.push(Line::from(""));
            content.push(Line::from(Span::styled(
                "  Press 'Esc' to go back",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::ITALIC),
            )));
        }

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Left)
            .block(Block::default().borders(Borders::ALL).title("Progress"));
        frame.render_widget(paragraph, chunks[1]);

        // フッター
        let footer_text = if has_error {
            "Operation failed".to_string()
        } else {
            "Please wait...".to_string()
        };
        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(if has_error { Color::Red } else { Color::Gray }))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, chunks[2]);
    }

    pub fn has_error(messages: &[String]) -> bool {
        messages.iter().any(|m| m.starts_with("ERROR["))
    }

    pub fn check_exit_key() -> Result<bool> {
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.code == KeyCode::Esc
        {
            return Ok(true);
        }
        Ok(false)
    }
}
