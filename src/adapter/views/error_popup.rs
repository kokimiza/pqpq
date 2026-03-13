use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub struct ErrorPopup {
    title: String,
    message: String,
}

impl ErrorPopup {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        // ポップアップのサイズを計算
        let popup_width = area.width.min(80);
        let popup_height = area.height.min(20);

        let popup_area = Rect {
            x: (area.width.saturating_sub(popup_width)) / 2,
            y: (area.height.saturating_sub(popup_height)) / 2,
            width: popup_width,
            height: popup_height,
        };

        // 背景をクリア
        frame.render_widget(Clear, popup_area);

        // ポップアップのレイアウト
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(popup_area);

        // タイトル
        let title_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .style(Style::default().bg(Color::Black));

        let title = Paragraph::new(Span::styled(
            format!("❌ {}", self.title),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center)
        .block(title_block);

        frame.render_widget(title, chunks[0]);

        // メッセージ
        let message_block = Block::default()
            .borders(Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(Color::Red))
            .style(Style::default().bg(Color::Black));

        let message = Paragraph::new(self.message.as_str())
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true })
            .block(message_block);

        frame.render_widget(message, chunks[1]);

        // フッター
        let footer_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .style(Style::default().bg(Color::Black));

        let footer = Paragraph::new("Press any key to close")
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(footer_block);

        frame.render_widget(footer, chunks[2]);
    }

    pub fn wait_for_key() -> Result<()> {
        // 既存のイベントを全てクリア
        while event::poll(std::time::Duration::from_millis(0))? {
            let _ = event::read()?;
        }

        // 新しいキー入力を待つ
        loop {
            if event::poll(std::time::Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        break;
                    }
                    _ => continue,
                }
            }
        }

        // ポップアップを閉じた後、残りのイベントを全てクリア
        std::thread::sleep(std::time::Duration::from_millis(50));
        while event::poll(std::time::Duration::from_millis(0))? {
            let _ = event::read()?;
        }

        Ok(())
    }
}
