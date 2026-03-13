use anyhow::Result;
use ratatui::DefaultTerminal;
use std::env;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

use super::combat_screen::CombatScreen;
use super::error_popup::ErrorPopup;
use super::host_screen::HostScreen;
use super::join_screen::{JoinAction, JoinScreen};
use super::loading_screen::LoadingScreen;
use super::menu_screen::{MenuAction, MenuOption, MenuScreen};

use crate::adapter::controllers::{
    CombatController, HostRingController, JoinRingController, ListRingsController,
};
use crate::adapter::presenters::CliPresenter;
use crate::application::usecases::{
    CombatInteractor, HostRingInteractor, JoinRingInteractor, ListRingsInteractor,
};
use crate::infrastructure::netcode::RollbackManager;
use crate::infrastructure::repositories::SupabaseRingRepository;
use sqlx::postgres::PgPoolOptions;

pub enum AppState {
    Menu,
    Loading {
        messages: Vec<String>,
    },
    Host {
        token: String,
        ring_id: String,
        status: String,
    },
    JoinInput,
    Combat,
}

pub enum NavigationCommand {
    Go(AppState),
    Back,
}

pub struct TuiView {
    terminal: DefaultTerminal,
    state_stack: Vec<AppState>,
    menu_screen: MenuScreen,
    join_screen: JoinScreen,
    combat_screen: CombatScreen,
}

impl TuiView {
    pub fn new() -> Result<Self> {
        let terminal = ratatui::init();
        Ok(Self {
            terminal,
            state_stack: vec![AppState::Menu],
            menu_screen: MenuScreen::new(),
            join_screen: JoinScreen::new(),
            combat_screen: CombatScreen::new(),
        })
    }

    pub fn navigate(&mut self, command: NavigationCommand) {
        match command {
            NavigationCommand::Go(state) => {
                self.state_stack.push(state);
            }
            NavigationCommand::Back => {
                if self.state_stack.len() > 1 {
                    self.state_stack.pop();
                }
            }
        }
    }

    pub fn current_state(&self) -> &AppState {
        self.state_stack.last().unwrap()
    }

    pub fn set_state(&mut self, state: AppState) {
        if let Some(last) = self.state_stack.last_mut() {
            *last = state;
        }
    }

    pub fn render(&mut self) -> Result<()> {
        let state = self.current_state();
        match state {
            AppState::Menu => {
                self.terminal.draw(|frame| {
                    self.menu_screen.render(frame);
                })?;
            }
            AppState::Loading { messages } => {
                let screen = LoadingScreen::new("pqpq");
                let messages = messages.clone();
                self.terminal.draw(|frame| {
                    screen.render(frame, &messages);
                })?;
            }
            AppState::Host {
                token,
                ring_id,
                status,
            } => {
                let screen = HostScreen::new(token.clone(), ring_id.clone(), status.clone());
                self.terminal.draw(|frame| {
                    screen.render(frame);
                })?;
            }
            AppState::JoinInput => {
                self.terminal.draw(|frame| {
                    self.join_screen.render(frame);
                })?;
            }
            AppState::Combat => {
                self.terminal.draw(|frame| {
                    self.combat_screen.render(frame);
                })?;
            }
        }
        Ok(())
    }

    pub fn handle_menu_input(&mut self) -> Result<Option<MenuOption>> {
        match self.menu_screen.handle_input()? {
            MenuAction::Select(option) => Ok(Some(option)),
            MenuAction::Exit => {
                std::process::exit(0);
            }
            MenuAction::None => Ok(None),
        }
    }

    pub fn handle_join_input(&mut self) -> Result<Option<String>> {
        match self.join_screen.handle_input()? {
            JoinAction::Submit(token) => {
                self.join_screen.reset();
                Ok(Some(token))
            }
            JoinAction::Back => {
                self.join_screen.reset();
                self.navigate(NavigationCommand::Back);
                Ok(None)
            }
            JoinAction::None => Ok(None),
        }
    }

    pub fn show_error_popup(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<()> {
        let popup = ErrorPopup::new(title, message);
        let current_state = self.current_state();

        let state_for_render = match current_state {
            AppState::Menu => AppState::Menu,
            AppState::Loading { messages } => AppState::Loading {
                messages: messages.clone(),
            },
            AppState::Host {
                token,
                ring_id,
                status,
            } => AppState::Host {
                token: token.clone(),
                ring_id: ring_id.clone(),
                status: status.clone(),
            },
            AppState::JoinInput => AppState::JoinInput,
            AppState::Combat => AppState::Combat,
        };

        self.terminal.draw(|frame| {
            match &state_for_render {
                AppState::Menu => {
                    self.menu_screen.render(frame);
                }
                AppState::Loading { messages } => {
                    let screen = LoadingScreen::new("pqpq");
                    screen.render(frame, messages);
                }
                AppState::Host {
                    token,
                    ring_id,
                    status,
                } => {
                    let screen = HostScreen::new(token.clone(), ring_id.clone(), status.clone());
                    screen.render(frame);
                }
                AppState::JoinInput => {
                    self.join_screen.render(frame);
                }
                AppState::Combat => {
                    self.combat_screen.render(frame);
                }
            }

            popup.render(frame);
        })?;

        ErrorPopup::wait_for_key()?;

        Ok(())
    }

    pub fn check_exit(&self) -> Result<bool> {
        let state = self.current_state();
        match state {
            AppState::Menu => Ok(false),
            AppState::Loading { messages } => {
                if LoadingScreen::has_error(messages) {
                    LoadingScreen::check_exit_key()
                } else {
                    Ok(false)
                }
            }
            AppState::Host { .. } => HostScreen::check_exit_key(),
            AppState::JoinInput => Ok(false),
            AppState::Combat => CombatScreen::check_exit_key(),
        }
    }

    // メニュー画面のメインループ
    pub async fn run_menu(&mut self) -> Result<()> {
        loop {
            self.render()?;

            if let Some(option) = self.handle_menu_input()? {
                match option {
                    MenuOption::HostRing => {
                        if let Err(e) = self.run_host_flow().await {
                            // エラーポップアップを表示して閉じるまで待つ
                            self.show_error_popup("Host Error", format!("{}", e))?;
                            // エラーが発生したのでメニューに戻る（デフォルト動作）
                            self.navigate(NavigationCommand::Back);
                        }
                    }
                    MenuOption::JoinRing => {
                        self.navigate(NavigationCommand::Go(AppState::JoinInput));
                        if let Err(e) = self.run_join_input_flow().await {
                            // エラーポップアップを表示して閉じるまで待つ
                            self.show_error_popup("Join Error", format!("{}", e))?;
                            // 既にrun_join_input_flowで適切に処理されているので何もしない
                        }
                    }
                    MenuOption::ListRings => {
                        if let Err(e) = self.run_list_flow().await {
                            // エラーポップアップを表示して閉じるまで待つ
                            self.show_error_popup("List Error", format!("{}", e))?;
                            // リスト表示エラーは常にメニューに戻る
                            self.navigate(NavigationCommand::Back);
                        }
                    }
                }
            }

            sleep(Duration::from_millis(100)).await;
        }
    }

    // Join入力画面のループ
    async fn run_join_input_flow(&mut self) -> Result<()> {
        loop {
            self.render()?;

            match self.handle_join_input()? {
                Some(token) => {
                    match self.run_join_flow(token).await {
                        Ok(_) => {
                            // マッチング成功したので入力画面から抜ける
                            break;
                        }
                        Err(e) => {
                            // エラーポップアップを表示して閉じるまで待つ
                            self.show_error_popup("Join Error", format!("{}", e))?;
                            // プレゼンターから戻るべきかを取得
                            // （この時点でプレゼンターは既にエラーレイヤーに基づいて判断済み）
                            // インフラエラーならメニューに戻る、アプリケーションエラーなら入力画面に留まる
                            // ここでは簡易的にエラーメッセージで判断
                            if e.to_string().contains("Infrastructure")
                                || e.to_string().contains("Failed to connect")
                                || e.to_string().contains("DATABASE_URL")
                            {
                                // インフラエラーはメニューに戻る
                                return Err(e);
                            }
                            // アプリケーションエラー（トークン間違いなど）は入力画面に留まる
                            // ループを続けて再入力を促す
                        }
                    }
                }
                None => {
                    // Backボタンが押された場合
                    if !matches!(self.current_state(), AppState::JoinInput) {
                        break;
                    }
                }
            }

            sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    // ホストフロー実行
    async fn run_host_flow(&mut self) -> Result<()> {
        self.navigate(NavigationCommand::Go(AppState::Loading {
            messages: vec![],
        }));

        // コントローラをビルド
        let (host_controller, host_interactor, presenter) = Self::build_host_controller().await?;
        let host_controller = Arc::new(host_controller);
        let presenter_clone = presenter.clone();

        // プレゼンターをリセット
        presenter.reset();

        // リング作成
        let controller = Arc::clone(&host_controller);
        let handle = tokio::spawn(async move { controller.create_ring().await });

        // Loading画面でリング作成を待つ
        loop {
            let messages = presenter_clone.get_progress_messages();
            self.set_state(AppState::Loading {
                messages: messages.clone(),
            });
            self.render()?;

            if messages.iter().any(|m| m.starts_with("ERROR[")) {
                let error_msg = messages
                    .iter()
                    .find(|m| m.starts_with("ERROR["))
                    .unwrap()
                    .to_string();
                // エラーが発生したのでLoading画面を削除してメニューに戻る
                self.navigate(NavigationCommand::Back);
                return Err(anyhow::anyhow!("{}", error_msg));
            }

            if messages.iter().any(|m| m.contains("TOKEN:")) {
                break;
            }

            if handle.is_finished() {
                match handle.await? {
                    Ok(_) => break,
                    Err(e) => {
                        // エラーが発生したのでLoading画面を削除してメニューに戻る
                        self.navigate(NavigationCommand::Back);
                        return Err(e);
                    }
                }
            }

            sleep(Duration::from_millis(100)).await;
        }

        // トークンとring_idを取得
        let messages = presenter_clone.get_progress_messages();
        let token = messages
            .iter()
            .find(|m| m.starts_with("TOKEN:"))
            .and_then(|m| m.strip_prefix("TOKEN:"))
            .ok_or_else(|| anyhow::anyhow!("Missing token"))?;
        let ring_id = messages
            .iter()
            .find(|m| m.starts_with("RING_ID:"))
            .and_then(|m| m.strip_prefix("RING_ID:"))
            .ok_or_else(|| anyhow::anyhow!("Missing ring_id"))?;

        // Host画面に遷移
        self.set_state(AppState::Host {
            token: token.to_string(),
            ring_id: ring_id.to_string(),
            status: "Waiting for opponent...".to_string(),
        });

        // マッチング待機
        let ring_uuid = uuid::Uuid::parse_str(ring_id)?;
        let controller = Arc::clone(&host_controller);
        let match_handle = tokio::spawn(async move { controller.wait_for_match(ring_uuid).await });

        // Host画面でマッチング待機
        loop {
            // ポーリング状態を取得してHost画面の待機メッセージを更新
            let messages = presenter_clone.get_progress_messages();
            let latest_poll = messages
                .iter()
                .rev()
                .find(|m| m.starts_with("Polling #") || m.starts_with("Still waiting"))
                .cloned()
                .unwrap_or_else(|| "Waiting for opponent...".to_string());
            self.set_state(AppState::Host {
                token: token.to_string(),
                ring_id: ring_id.to_string(),
                status: latest_poll,
            });
            self.render()?;

            // タスクが完了したらawaitする
            if match_handle.is_finished() {
                match match_handle.await {
                    Ok(Ok(_)) => {
                        // InteractorからP2P接続を取り出す
                        let p2p = host_interactor.take_p2p_connection().await;
                        if let Some(p2p) = p2p {
                            self.navigate(NavigationCommand::Go(AppState::Combat));
                            if let Err(e) = self.run_combat_loop(p2p, true).await {
                                // 戦闘中のエラー: Combat画面を除去してからエラーを返す
                                self.navigate(NavigationCommand::Back);
                                self.navigate(NavigationCommand::Back);
                                return Err(e);
                            }
                            // 戦闘終了後はメニューに戻る（Combat + Host + Loading）
                            self.navigate(NavigationCommand::Back);
                        }
                        // Host画面とLoading画面をスキップ
                        self.navigate(NavigationCommand::Back);
                        self.navigate(NavigationCommand::Back);
                        break;
                    }
                    Ok(Err(e)) => {
                        // マッチングエラーが発生したのでHost画面を削除してメニューに戻る
                        self.navigate(NavigationCommand::Back);
                        self.navigate(NavigationCommand::Back);
                        return Err(e);
                    }
                    Err(e) => {
                        // タスクエラーが発生したのでHost画面を削除してメニューに戻る
                        self.navigate(NavigationCommand::Back);
                        self.navigate(NavigationCommand::Back);
                        return Err(anyhow::anyhow!("Match task failed: {}", e));
                    }
                }
            }

            if self.check_exit()? {
                // ユーザーがキャンセルしたのでHost画面を削除してメニューに戻る
                self.navigate(NavigationCommand::Back);
                self.navigate(NavigationCommand::Back);
                break;
            }

            sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }

    // 参加フロー実行
    async fn run_join_flow(&mut self, token: String) -> Result<()> {
        self.navigate(NavigationCommand::Go(AppState::Loading {
            messages: vec![],
        }));

        // コントローラをビルド
        let (join_controller, join_interactor, presenter) = Self::build_join_controller().await?;
        let join_controller = Arc::new(join_controller);
        let presenter_clone = presenter.clone();

        // プレゼンターをリセット
        presenter.reset();

        // リング参加
        let controller = Arc::clone(&join_controller);
        let token_clone = token.clone();
        let handle = tokio::spawn(async move { controller.join_ring(token_clone).await });

        // Loading画面でリング参加を待つ
        loop {
            let messages = presenter_clone.get_progress_messages();
            self.set_state(AppState::Loading {
                messages: messages.clone(),
            });
            self.render()?;

            if messages.iter().any(|m| m.starts_with("ERROR[")) {
                let error_msg = messages
                    .iter()
                    .find(|m| m.starts_with("ERROR["))
                    .unwrap()
                    .to_string();
                // エラーが発生したのでLoading画面を削除して入力画面に戻る
                self.navigate(NavigationCommand::Back);
                return Err(anyhow::anyhow!("{}", error_msg));
            }

            if messages.iter().any(|m| m.contains("RING_ID:")) {
                break;
            }

            if handle.is_finished() {
                match handle.await? {
                    Ok(_) => break,
                    Err(e) => {
                        // エラーが発生したのでLoading画面を削除して入力画面に戻る
                        self.navigate(NavigationCommand::Back);
                        return Err(e);
                    }
                }
            }

            sleep(Duration::from_millis(100)).await;
        }

        // ring_idを取得
        let messages = presenter_clone.get_progress_messages();
        let ring_id = messages
            .iter()
            .find(|m| m.starts_with("RING_ID:"))
            .and_then(|m| m.strip_prefix("RING_ID:"))
            .ok_or_else(|| anyhow::anyhow!("Missing ring_id"))?;

        let ring_uuid = uuid::Uuid::parse_str(ring_id)?;

        // マッチング待機
        let controller = Arc::clone(&join_controller);
        let match_handle = tokio::spawn(async move { controller.wait_for_match(ring_uuid).await });

        // Loading画面でマッチング待機
        loop {
            let messages = presenter_clone.get_progress_messages();
            self.set_state(AppState::Loading {
                messages: messages.clone(),
            });
            self.render()?;

            // タスクが完了したらawaitする
            if match_handle.is_finished() {
                match match_handle.await {
                    Ok(Ok(_)) => {
                        // InteractorからP2P接続を取り出す
                        let p2p = join_interactor.take_p2p_connection().await;
                        if let Some(p2p) = p2p {
                            self.set_state(AppState::Loading {
                                messages: vec![
                                    "P2P connection acquired, starting combat...".to_string(),
                                ],
                            });
                            self.render()?;
                            self.navigate(NavigationCommand::Go(AppState::Combat));
                            if let Err(e) = self.run_combat_loop(p2p, false).await {
                                // 戦闘中のエラー: Combat画面を除去してからエラーを返す
                                self.navigate(NavigationCommand::Back);
                                self.navigate(NavigationCommand::Back);
                                return Err(e);
                            }
                            // 戦闘終了後はメニューに戻る（Combat + Loading + JoinInput）
                            self.navigate(NavigationCommand::Back);
                        } else {
                            // P2P接続が取得できなかった
                            self.navigate(NavigationCommand::Back);
                            return Err(anyhow::anyhow!("Failed to acquire P2P connection"));
                        }
                        // Loading画面とJoinInput画面をスキップ
                        self.navigate(NavigationCommand::Back);
                        self.navigate(NavigationCommand::Back);
                        break;
                    }
                    Ok(Err(e)) => {
                        // マッチングエラーが発生したのでLoading画面を削除して入力画面に戻る
                        self.navigate(NavigationCommand::Back);
                        return Err(e);
                    }
                    Err(e) => {
                        // タスクエラーが発生したのでLoading画面を削除して入力画面に戻る
                        self.navigate(NavigationCommand::Back);
                        return Err(anyhow::anyhow!("Match task failed: {}", e));
                    }
                }
            }

            if self.check_exit()? {
                // ユーザーがキャンセルしたのでLoading画面を削除して入力画面に戻る
                self.navigate(NavigationCommand::Back);
                break;
            }

            sleep(Duration::from_millis(100)).await;
        }

        Ok(())
    }

    // リスト表示フロー実行
    async fn run_list_flow(&mut self) -> Result<()> {
        self.navigate(NavigationCommand::Go(AppState::Loading {
            messages: vec![],
        }));

        let result = async {
            let (list_controller, _presenter) = Self::build_list_controller().await?;
            list_controller.execute().await
        }
        .await;

        // リスト表示は常にメニューに戻る
        self.navigate(NavigationCommand::Back);
        result
    }

    // 戦闘画面ループ
    async fn run_combat_loop(
        &mut self,
        p2p: Arc<dyn crate::domain::services::P2PService>,
        is_host: bool,
    ) -> Result<()> {
        use crate::infrastructure::netcode::InputSerializerImpl;

        self.set_state(AppState::Loading {
            messages: vec!["Initializing combat...".to_string()],
        });
        self.render()?;

        // RollbackManagerを作成
        let rollback_manager = RollbackManager::default_config();

        // InputSerializerを作成
        let serializer = Arc::new(InputSerializerImpl::new());

        self.set_state(AppState::Loading {
            messages: vec!["Creating combat interactor...".to_string()],
        });
        self.render()?;

        // CombatInteractorをビルド
        let combat_interactor = CombatInteractor::new(rollback_manager, p2p, serializer, is_host);
        let combat_screen_arc = Arc::new(self.combat_screen.clone());
        let combat_controller = CombatController::new(combat_interactor, combat_screen_arc);

        self.set_state(AppState::Loading {
            messages: vec!["Starting combat...".to_string()],
        });
        self.render()?;

        // 戦闘を開始
        combat_controller
            .run_combat("player1".to_string(), "player2".to_string(), 60)
            .await?;

        self.set_state(AppState::Loading {
            messages: vec!["Combat finished!".to_string()],
        });
        self.render()?;

        // 戦闘終了後、結果を表示してキー入力を待つ
        loop {
            self.render()?;

            // 現在のフレームデータを取得して終了判定
            if let Some(frame) = self.combat_screen.get_current_frame()
                && frame.winner != 0
            {
                // 勝者が決まったら少し待ってから終了
                sleep(Duration::from_secs(2)).await;
                break;
            }

            if self.check_exit()? {
                break;
            }

            sleep(Duration::from_millis(16)).await; // 60 FPS
        }

        Ok(())
    }

    // DI: HostRingControllerのビルド
    async fn build_host_controller() -> Result<(
        HostRingController<HostRingInteractor<SupabaseRingRepository, CliPresenter>>,
        Arc<HostRingInteractor<SupabaseRingRepository, CliPresenter>>,
        CliPresenter,
    )> {
        use crate::infrastructure::errors::InfrastructureError;

        let database_url = env::var("DATABASE_URL").map_err(InfrastructureError::EnvVar)?;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .map_err(|e| {
                InfrastructureError::DatabaseConnection(format!("Failed to connect: {}", e))
            })?;

        let ring_repository = SupabaseRingRepository::new(pool);
        let cli_presenter = CliPresenter::new();
        let host_ring_interactor = Arc::new(HostRingInteractor::new(
            ring_repository,
            cli_presenter.clone(),
        ));
        let controller = HostRingController::new(Arc::clone(&host_ring_interactor));

        Ok((controller, host_ring_interactor, cli_presenter))
    }

    // DI: JoinRingControllerのビルド
    async fn build_join_controller() -> Result<(
        JoinRingController<JoinRingInteractor<SupabaseRingRepository, CliPresenter>>,
        Arc<JoinRingInteractor<SupabaseRingRepository, CliPresenter>>,
        CliPresenter,
    )> {
        use crate::infrastructure::errors::InfrastructureError;

        let database_url = env::var("DATABASE_URL").map_err(InfrastructureError::EnvVar)?;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .map_err(|e| {
                InfrastructureError::DatabaseConnection(format!("Failed to connect: {}", e))
            })?;

        let ring_repository = SupabaseRingRepository::new(pool);
        let cli_presenter = CliPresenter::new();
        let join_ring_interactor = Arc::new(JoinRingInteractor::new(
            ring_repository,
            cli_presenter.clone(),
        ));
        let controller = JoinRingController::new(Arc::clone(&join_ring_interactor));

        Ok((controller, join_ring_interactor, cli_presenter))
    }

    // DI: ListRingsControllerのビルド
    async fn build_list_controller() -> Result<(
        ListRingsController<ListRingsInteractor<SupabaseRingRepository, CliPresenter>>,
        CliPresenter,
    )> {
        use crate::infrastructure::errors::InfrastructureError;

        let database_url = env::var("DATABASE_URL").map_err(InfrastructureError::EnvVar)?;

        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .map_err(|e| {
                InfrastructureError::DatabaseConnection(format!("Failed to connect: {}", e))
            })?;

        let ring_repository = SupabaseRingRepository::new(pool);
        let cli_presenter = CliPresenter::new();
        let list_rings_interactor =
            ListRingsInteractor::new(ring_repository, cli_presenter.clone());
        let controller = ListRingsController::new(list_rings_interactor);

        Ok((controller, cli_presenter))
    }
}

impl Drop for TuiView {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
