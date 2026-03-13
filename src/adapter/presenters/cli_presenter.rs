use crate::application::dto::response::{HostRingResponse, JoinRingResponse, ListRingsResponse};
use crate::application::ports::output::{
    ErrorLayer, HostRingOutputPort, JoinRingOutputPort, ListRingsOutputPort,
};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct CliPresenter {
    progress_messages: Arc<Mutex<Vec<String>>>,
    /// エラー時に前の画面に戻るべきか（デフォルト: true）
    should_go_back_on_error: Arc<Mutex<bool>>,
}

impl CliPresenter {
    pub fn new() -> Self {
        Self {
            progress_messages: Arc::new(Mutex::new(Vec::new())),
            should_go_back_on_error: Arc::new(Mutex::new(true)),
        }
    }

    pub fn get_progress_messages(&self) -> Vec<String> {
        self.progress_messages.lock().unwrap().clone()
    }

    /// エラー時に前の画面に戻るべきかを取得
    #[allow(dead_code)]
    pub fn should_go_back_on_error(&self) -> bool {
        *self.should_go_back_on_error.lock().unwrap()
    }

    /// エラー時に前の画面に戻るべきかを設定
    fn set_should_go_back_on_error(&self, should_go_back: bool) {
        *self.should_go_back_on_error.lock().unwrap() = should_go_back;
    }

    /// プログレスメッセージをクリアして、戻るフラグをリセット
    pub fn reset(&self) {
        self.progress_messages.lock().unwrap().clear();
        *self.should_go_back_on_error.lock().unwrap() = true;
    }

    fn format_error(&self, layer: ErrorLayer, message: &str) -> String {
        let layer_name = match layer {
            ErrorLayer::Domain => "Domain",
            ErrorLayer::Application => "Application",
            ErrorLayer::Infrastructure => "Infrastructure",
        };
        format!("ERROR[{}]: {}", layer_name, message)
    }

    /// エラーレイヤーに基づいて、前の画面に戻るべきかを判断
    fn should_go_back_for_layer(&self, layer: ErrorLayer) -> bool {
        match layer {
            // インフラエラー（DB接続、通信エラー）は戻る
            ErrorLayer::Infrastructure => true,
            // アプリケーションエラー（トークン間違い、リングが見つからない）は戻らない
            ErrorLayer::Application => false,
            // ドメインエラー（バリデーションエラー）は戻らない
            ErrorLayer::Domain => false,
        }
    }
}

#[async_trait]
impl HostRingOutputPort for CliPresenter {
    async fn notify_progress(&self, message: &str) -> Result<()> {
        self.progress_messages
            .lock()
            .unwrap()
            .push(message.to_string());
        Ok(())
    }

    async fn notify_error(&self, layer: ErrorLayer, message: &str) -> Result<()> {
        // エラーレイヤーに基づいて戻るべきかを設定
        self.set_should_go_back_on_error(self.should_go_back_for_layer(layer.clone()));

        self.progress_messages
            .lock()
            .unwrap()
            .push(self.format_error(layer, message));
        Ok(())
    }

    async fn present(&self, response: HostRingResponse) -> Result<()> {
        self.progress_messages
            .lock()
            .unwrap()
            .push("Ring created successfully!".to_string());

        // TUIで表示するためにメッセージを保持
        self.progress_messages
            .lock()
            .unwrap()
            .push(format!("TOKEN:{}", response.token));
        self.progress_messages
            .lock()
            .unwrap()
            .push(format!("RING_ID:{}", response.ring_id));

        Ok(())
    }
}

#[async_trait]
impl JoinRingOutputPort for CliPresenter {
    async fn notify_progress(&self, message: &str) -> Result<()> {
        self.progress_messages
            .lock()
            .unwrap()
            .push(message.to_string());
        Ok(())
    }

    async fn notify_error(&self, layer: ErrorLayer, message: &str) -> Result<()> {
        // エラーレイヤーに基づいて戻るべきかを設定
        self.set_should_go_back_on_error(self.should_go_back_for_layer(layer.clone()));

        self.progress_messages
            .lock()
            .unwrap()
            .push(self.format_error(layer, message));
        Ok(())
    }

    async fn present(&self, response: JoinRingResponse) -> Result<()> {
        self.progress_messages
            .lock()
            .unwrap()
            .push("Joined ring successfully!".to_string());

        self.progress_messages
            .lock()
            .unwrap()
            .push(format!("RING_ID:{}", response.ring_id));

        Ok(())
    }
}

#[async_trait]
impl ListRingsOutputPort for CliPresenter {
    async fn notify_error(&self, layer: ErrorLayer, message: &str) -> Result<()> {
        // エラーレイヤーに基づいて戻るべきかを設定
        self.set_should_go_back_on_error(self.should_go_back_for_layer(layer.clone()));

        self.progress_messages
            .lock()
            .unwrap()
            .push(self.format_error(layer, message));
        Ok(())
    }

    async fn present(&self, response: ListRingsResponse) -> Result<()> {
        println!("🥊 Open Rings:");
        if response.rings.is_empty() {
            println!("  No open rings available.");
        } else {
            for ring in response.rings {
                println!("  Token: {} (created: {})", ring.token, ring.created_at);
            }
        }
        Ok(())
    }
}
