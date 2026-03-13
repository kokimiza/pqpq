mod adapter;
mod application;
mod domain;
mod infrastructure;

use adapter::views::TuiView;
use anyhow::Result;
use infrastructure::firewall;

#[tokio::main]
async fn main() -> Result<()> {
    // .envファイルを読み込む
    match dotenvy::from_filename(".env") {
        Ok(_) => println!("Loaded .env file"),
        Err(e) => eprintln!("Warning: Could not load .env file: {}", e),
    }

    // Windowsファイアウォールの状態をチェック
    let fw_status = firewall::check_firewall_status();
    if fw_status.may_cause_issues() {
        eprintln!();
        eprintln!("{}", firewall::firewall_fix_instructions());
        eprintln!();
        eprintln!("Press Enter to continue anyway, or Ctrl+C to exit...");
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
    }

    // TUIビューを起動してメニュー画面を表示
    let mut tui = TuiView::new()?;
    tui.run_menu().await?;

    Ok(())
}
