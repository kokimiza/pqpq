/// Windowsファイアウォール診断ユーティリティ
///
/// WebRTC P2P通信に必要なファイアウォール許可が設定されているかチェックし、
/// 未設定の場合はユーザーにガイダンスを提供する。
///
/// ## 背景
/// Windows環境でWebRTCを使用するアプリを初回起動すると、
/// 「このアプリにネットワークへのアクセスを許可しますか？」ダイアログが表示される。
/// ユーザーが誤ってキャンセルすると受信接続がブロックされ、P2P通信が失敗する。
use std::path::PathBuf;

/// ファイアウォール診断結果
#[derive(Debug, Clone)]
pub enum FirewallStatus {
    /// 許可ルールが存在する
    Allowed,
    /// ブロックルールが存在する（キャンセルした可能性が高い）
    Blocked,
    /// ルールが見つからない（初回起動前、またはチェック不可）
    Unknown,
    /// Windows以外のOS
    NotApplicable,
}

impl FirewallStatus {
    /// ファイアウォールが原因で通信に問題が起きる可能性があるか
    pub fn may_cause_issues(&self) -> bool {
        matches!(self, FirewallStatus::Blocked)
    }
}

/// 現在の実行ファイルのパスを取得
fn current_exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Windowsファイアウォールのルールをチェック
///
/// `netsh advfirewall firewall show rule` を使用して、
/// 現在の実行ファイルに対するルールを確認する。
pub fn check_firewall_status() -> FirewallStatus {
    if !cfg!(target_os = "windows") {
        return FirewallStatus::NotApplicable;
    }

    let Some(exe_path) = current_exe_path() else {
        return FirewallStatus::Unknown;
    };

    let exe_name = exe_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("pqpq.exe");

    // netshでファイアウォールルールを検索
    let output = std::process::Command::new("netsh")
        .args(["advfirewall", "firewall", "show", "rule", "name=all"])
        .output();

    let Ok(output) = output else {
        return FirewallStatus::Unknown;
    };

    let stdout = String::from_utf8_lossy(&output.stdout);

    // 実行ファイル名に関連するルールを探す
    // netshの出力はブロック単位で区切られている
    let blocks: Vec<&str> = stdout.split("\r\n\r\n").collect();

    for block in &blocks {
        let block_lower = block.to_lowercase();
        let exe_lower = exe_name.to_lowercase();

        if !block_lower.contains(&exe_lower) {
            continue;
        }

        // "Action: Block" が含まれていればブロック
        if block_lower.contains("block") && block_lower.contains("action") {
            return FirewallStatus::Blocked;
        }

        // "Action: Allow" が含まれていれば許可
        if block_lower.contains("allow") && block_lower.contains("action") {
            return FirewallStatus::Allowed;
        }
    }

    FirewallStatus::Unknown
}

/// ファイアウォール問題の修正手順を返す
pub fn firewall_fix_instructions() -> String {
    let exe_path = current_exe_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "pqpq.exe".to_string());

    format!(
        r#"=== Firewall Configuration Required ===

pqpq needs network access for P2P battles.
The Windows Firewall may be blocking connections.

Fix Option 1: Reset via Windows Settings
  1. Open "Windows Security" > "Firewall & network protection"
  2. Click "Allow an app through firewall"
  3. Find "pqpq" and check both Private/Public
  4. If not listed, click "Allow another app" and browse to:
     {}

Fix Option 2: PowerShell (Run as Administrator)
  netsh advfirewall firewall delete rule name="pqpq"
  netsh advfirewall firewall add rule name="pqpq" dir=in action=allow program="{}" enable=yes

After fixing, restart pqpq."#,
        exe_path, exe_path
    )
}

/// WebRTC接続失敗時のエラーメッセージを生成
///
/// ファイアウォールの状態を考慮して、適切なヒントを付加する。
pub fn connection_error_with_hint(original_error: &str) -> String {
    let status = check_firewall_status();

    match status {
        FirewallStatus::Blocked => {
            format!(
                "{}\n\n[!] Windows Firewall is blocking pqpq.\n\
                 The firewall dialog may have been cancelled.\n\
                 See instructions above or run pqpq again to get the dialog.",
                original_error
            )
        }
        FirewallStatus::Unknown => {
            format!(
                "{}\n\n[?] If this is your first run, Windows Firewall\n\
                 may have shown a dialog. If you cancelled it,\n\
                 network access is blocked. Re-run pqpq or\n\
                 manually allow it in Windows Firewall settings.",
                original_error
            )
        }
        _ => original_error.to_string(),
    }
}
