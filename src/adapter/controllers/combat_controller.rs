use crate::application::dto::request::{CombatFrameRequest, StartCombatRequest};
use crate::application::ports::input::CombatInputPort;
use crate::application::ports::output::CombatOutputPort;
use crate::domain::value_objects::CombatInputKey;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use std::sync::Arc;
use std::time::Duration;

/// 戦闘コントローラ
pub struct CombatController<I: CombatInputPort, O: CombatOutputPort> {
    input_port: I,
    output_port: Arc<O>,
}

impl<I: CombatInputPort, O: CombatOutputPort> CombatController<I, O> {
    pub fn new(input_port: I, output_port: Arc<O>) -> Self {
        Self {
            input_port,
            output_port,
        }
    }

    /// 戦闘を実行
    pub async fn run_combat(
        &self,
        player1_id: String,
        player2_id: String,
        round_time: u32,
    ) -> Result<()> {
        // 戦闘開始
        let start_request = StartCombatRequest {
            player1_id,
            player2_id,
            round_time_seconds: round_time,
        };

        let start_response = self.input_port.start_combat(start_request).await?;
        self.output_port.present_start(start_response);

        // メインループ
        let mut frame_number = 0;
        loop {
            // 入力を取得
            let (player1_input, player2_input) = self.read_inputs()?;

            // フレーム更新
            let frame_request = CombatFrameRequest {
                player1_input,
                player2_input,
                frame_number,
            };

            let frame_response = self.input_port.update_frame(frame_request).await?;
            self.output_port.present_frame(frame_response.clone());

            // 終了判定
            if frame_response.winner != 0 {
                break;
            }

            frame_number += 1;

            // 60FPS制御（約16.67ms）
            tokio::time::sleep(Duration::from_millis(16)).await;
        }

        Ok(())
    }

    /// キーボード入力を読み取る（ノンブロッキング）
    fn read_inputs(&self) -> Result<(CombatInputKey, CombatInputKey)> {
        let mut player1_input = CombatInputKey::empty();
        let mut player2_input = CombatInputKey::empty();

        // ノンブロッキングで入力をポーリング
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(key_event) = event::read()? {
                self.map_key_to_input(key_event, &mut player1_input, &mut player2_input);
            }
        }

        Ok((player1_input, player2_input))
    }

    /// キー入力をゲーム入力にマッピング
    fn map_key_to_input(&self, key: KeyEvent, p1: &mut CombatInputKey, p2: &mut CombatInputKey) {
        match key.code {
            // Player 1: WASD + FG
            KeyCode::Char('w') | KeyCode::Char('W') => p1.press(CombatInputKey::UP),
            KeyCode::Char('s') | KeyCode::Char('S') => p1.press(CombatInputKey::DOWN),
            KeyCode::Char('a') | KeyCode::Char('A') => p1.press(CombatInputKey::LEFT),
            KeyCode::Char('d') | KeyCode::Char('D') => p1.press(CombatInputKey::RIGHT),
            KeyCode::Char('f') | KeyCode::Char('F') => p1.press(CombatInputKey::PUNCH),
            KeyCode::Char('g') | KeyCode::Char('G') => p1.press(CombatInputKey::KICK),

            // Player 2: Arrow keys + KL
            KeyCode::Up => p2.press(CombatInputKey::UP),
            KeyCode::Down => p2.press(CombatInputKey::DOWN),
            KeyCode::Left => p2.press(CombatInputKey::LEFT),
            KeyCode::Right => p2.press(CombatInputKey::RIGHT),
            KeyCode::Char('k') | KeyCode::Char('K') => p2.press(CombatInputKey::PUNCH),
            KeyCode::Char('l') | KeyCode::Char('L') => p2.press(CombatInputKey::KICK),

            _ => {}
        }
    }
}
