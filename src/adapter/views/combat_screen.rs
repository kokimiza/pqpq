use crate::application::dto::response::{CombatFrameResponse, StartCombatResponse};
use crate::application::ports::output::CombatOutputPort;
use crate::domain::domain_services::MatchState;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
};
use std::sync::{Arc, Mutex};

/// レトロスタイルの戦闘画面
///
/// デザインコンセプト:
/// - 80年代アーケードゲーム風
/// - シンプルなASCIIアート
/// - ネオンカラー（シアン、マゼンタ、イエロー）
/// - ドット絵風の表現
#[derive(Clone)]
pub struct CombatScreen {
    /// 現在のフレームデータ
    current_frame: Arc<Mutex<Option<CombatFrameResponse>>>,
}

impl CombatScreen {
    pub fn new() -> Self {
        Self {
            current_frame: Arc::new(Mutex::new(None)),
        }
    }

    /// フレームデータを更新
    pub fn update_frame(&self, response: CombatFrameResponse) {
        *self.current_frame.lock().unwrap() = Some(response);
    }

    /// 現在のフレームデータを取得
    pub fn get_current_frame(&self) -> Option<CombatFrameResponse> {
        self.current_frame.lock().unwrap().clone()
    }

    /// 戦闘画面を描画
    pub fn render(&self, frame: &mut Frame) {
        let current = self.current_frame.lock().unwrap();

        if let Some(ref response) = *current {
            self.render_with_data(frame, response);
        } else {
            // データがない場合はダミーデータで描画
            let dummy_response = CombatFrameResponse {
                player1_hp: 200,
                player2_hp: 180,
                player1_x: 20,
                player1_y: 15,
                player2_x: 60,
                player2_y: 15,
                player1_state: "Idle".to_string(),
                player2_state: "Idle".to_string(),
                remaining_frames: 3600,
                match_state: MatchState::InProgress,
                winner: 0,
            };
            self.render_with_data(frame, &dummy_response);
        }
    }

    /// 戦闘画面を描画（データ付き）
    pub fn render_with_data(&self, frame: &mut Frame, response: &CombatFrameResponse) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // ヘッダー（HP、タイマー）
                Constraint::Min(18),   // 戦闘エリア
                Constraint::Length(3), // フッター（操作説明）
            ])
            .split(frame.area());

        self.render_header(frame, chunks[0], response);
        self.render_arena(frame, chunks[1], response);
        self.render_footer(frame, chunks[2]);
    }

    /// ヘッダー（HP、タイマー）を描画
    fn render_header(&self, frame: &mut Frame, area: Rect, response: &CombatFrameResponse) {
        let header_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(40), // P1 HP
                Constraint::Percentage(20), // タイマー
                Constraint::Percentage(40), // P2 HP
            ])
            .split(area);

        // Player 1 HP
        let p1_hp_percent = (response.player1_hp as f64 / 255.0) * 100.0;
        let p1_gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("P1"))
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
            .percent(p1_hp_percent as u16)
            .label(format!("{}/255", response.player1_hp));
        frame.render_widget(p1_gauge, header_chunks[0]);

        // タイマー
        let remaining_seconds = response.remaining_frames / 60;
        let timer = Paragraph::new(format!("{:02}", remaining_seconds))
            .block(Block::default().borders(Borders::ALL).title("TIME"))
            .style(Style::default().fg(Color::Yellow))
            .alignment(Alignment::Center);
        frame.render_widget(timer, header_chunks[1]);

        // Player 2 HP
        let p2_hp_percent = (response.player2_hp as f64 / 255.0) * 100.0;
        let p2_gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).title("P2"))
            .gauge_style(Style::default().fg(Color::Magenta).bg(Color::Black))
            .percent(p2_hp_percent as u16)
            .label(format!("{}/255", response.player2_hp));
        frame.render_widget(p2_gauge, header_chunks[2]);
    }

    /// 戦闘エリアを描画
    fn render_arena(&self, frame: &mut Frame, area: Rect, response: &CombatFrameResponse) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("═══ ARENA ═══")
            .style(Style::default().fg(Color::White));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // 地面ライン
        let ground_y = 20;
        let ground_line = "═".repeat(inner.width as usize);
        let ground = Paragraph::new(ground_line).style(Style::default().fg(Color::DarkGray));
        let ground_area = Rect {
            x: inner.x,
            y: inner.y + ground_y as u16,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(ground, ground_area);

        // Player 1 キャラクター（左側、シアン）
        self.render_fighter(
            frame,
            inner,
            response.player1_x,
            response.player1_y,
            Color::Cyan,
            &response.player1_state,
        );

        // Player 2 キャラクター（右側、マゼンタ）
        self.render_fighter(
            frame,
            inner,
            response.player2_x,
            response.player2_y,
            Color::Magenta,
            &response.player2_state,
        );

        // 勝敗表示
        if response.winner != 0 {
            self.render_result(frame, inner, response);
        } else if response.match_state == MatchState::Preparing {
            self.render_countdown(frame, inner, response);
        }
    }

    /// ファイターを描画（レトロなドット絵風）
    fn render_fighter(
        &self,
        frame: &mut Frame,
        area: Rect,
        x: u8,
        y: u8,
        color: Color,
        state: &str,
    ) {
        // 簡易的なASCIIアートキャラクター
        let fighter_art = if state.contains("Attacking") {
            // 攻撃モーション
            vec![" O ", "/|\\", "/ \\"]
        } else if state.contains("Stunned") {
            // 硬直モーション
            vec![" @ ", "\\|/", "/ \\"]
        } else {
            // 待機モーション
            vec![" O ", "-|-", "/ \\"]
        };

        for (i, line) in fighter_art.iter().enumerate() {
            let fighter_line = Paragraph::new(*line)
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD));

            let fighter_area = Rect {
                x: area.x + x as u16,
                y: area.y + y as u16 + i as u16,
                width: 3,
                height: 1,
            };

            if fighter_area.y < area.y + area.height {
                frame.render_widget(fighter_line, fighter_area);
            }
        }
    }

    /// カウントダウンを描画
    fn render_countdown(&self, frame: &mut Frame, area: Rect, response: &CombatFrameResponse) {
        let countdown_num = (response.remaining_frames / 60) + 1;
        let text = if countdown_num > 3 {
            "READY".to_string()
        } else if countdown_num > 0 {
            countdown_num.to_string()
        } else {
            "FIGHT!".to_string()
        };

        let countdown = Paragraph::new(text)
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center);

        let countdown_area = Rect {
            x: area.x,
            y: area.y + area.height / 2,
            width: area.width,
            height: 1,
        };

        frame.render_widget(countdown, countdown_area);
    }

    /// 勝敗結果を描画
    fn render_result(&self, frame: &mut Frame, area: Rect, response: &CombatFrameResponse) {
        let result_text = match response.winner {
            1 => vec![
                Line::from(vec![Span::styled(
                    "PLAYER 1",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![Span::styled(
                    "WINS!",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]),
            ],
            2 => vec![
                Line::from(vec![Span::styled(
                    "PLAYER 2",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![Span::styled(
                    "WINS!",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]),
            ],
            _ => vec![Line::from(vec![Span::styled(
                "DRAW",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )])],
        };

        let result = Paragraph::new(result_text).alignment(Alignment::Center);

        let result_area = Rect {
            x: area.x,
            y: area.y + area.height / 2 - 1,
            width: area.width,
            height: 3,
        };

        frame.render_widget(result, result_area);
    }

    /// フッター（操作説明）を描画
    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let controls = vec![Line::from(vec![
            Span::styled("P1: ", Style::default().fg(Color::Cyan)),
            Span::raw("WASD=Move F=Punch G=Kick  "),
            Span::styled("P2: ", Style::default().fg(Color::Magenta)),
            Span::raw("Arrows=Move K=Punch L=Kick"),
        ])];

        let footer = Paragraph::new(controls)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center);

        frame.render_widget(footer, area);
    }

    /// 終了キーをチェック
    pub fn check_exit_key() -> anyhow::Result<bool> {
        use crossterm::event::{self, Event, KeyCode};
        use std::time::Duration;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.code == KeyCode::Esc
        {
            return Ok(true);
        }
        Ok(false)
    }
}

impl CombatOutputPort for CombatScreen {
    fn present_start(&self, _response: StartCombatResponse) {
        // TUI初期化は別で行うため、ここでは何もしない
    }

    fn present_frame(&self, response: CombatFrameResponse) {
        // フレームデータを更新
        self.update_frame(response);
    }
}

impl Default for CombatScreen {
    fn default() -> Self {
        Self::new()
    }
}
