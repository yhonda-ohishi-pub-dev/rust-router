# 移行チェックリスト

## Phase 3-4: 横展開・PHP 退役（将来タスク - 現在対象外）

PHP からの移行タスクは現在このリポジトリの対象外。
expense-service, tachograph-service 等は別途計画する。

---

# gRPC API（Go版互換）

```protobuf
service ETCScraper {
  rpc Health(HealthRequest) returns (HealthResponse);
  rpc Scrape(ScrapeRequest) returns (ScrapeResponse);
  rpc ScrapeMultiple(ScrapeMultipleRequest) returns (ScrapeMultipleResponse);
  rpc GetDownloadedFiles(GetDownloadedFilesRequest) returns (GetDownloadedFilesResponse);
  rpc StreamDownload(StreamDownloadRequest) returns (stream StreamDownloadChunk);
}
```

## 依存クレート

```toml
[dependencies]
tonic = "0.12"
prost = "0.13"
tokio = { version = "1", features = ["full"] }
tower = "0.4"
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
thiserror = "1"
# scraper-service = { git = "...", tag = "v0.1.0" }

[build-dependencies]
tonic-build = "0.12"
```

## ScraperService インターフェース

```rust
#[async_trait]
pub trait ScraperService: Send + Sync {
    async fn scrape(&self, config: ScrapeConfig) -> Result<ScrapeResult, ScraperError>;
}
```

---

# Phase R8: インストーラー & 自動更新

## 概要

Windowsサービスとして配布するためのMSIインストーラー作成と、GitHub Releasesを使った自動更新機能の統合。

## アーキテクチャ

```
[GitHub Releases]
    ├── gateway-v1.0.0-windows-x86_64.msi   # 初回インストール用
    └── gateway-v1.0.0-windows-x86_64.exe   # 自動更新用

[初回インストール]
    MSI → サービス登録 → gateway.exe 配置

[自動更新]
    gateway.exe → GitHub API で最新確認 → ダウンロード → バッチで置換 → サービス再起動
```

## タスク

### インストーラー (cargo-wix)

- [x] cargo-wix インストール: `cargo install cargo-wix`
- [x] 【手動】WiX Toolset インストール（Windows）
  - https://wixtoolset.org/releases/ から WiX v3.x をダウンロード・インストール
  - インストール後、bin フォルダを PATH に追加するか、WIX 環境変数を設定
- [x] `cargo wix init` で設定ファイル生成
- [x] `wix/main.wxs` カスタマイズ
  - [x] サービス登録設定
  - [x] インストール先選択
  - [x] スタートメニュー追加
- [x] 【手動】`cargo wix` でMSIビルド確認（WiX Toolset インストール後に実行可能）
  - 出力: `gateway\target\wix\gateway-0.1.0-x86_64.msi`
  - ビルドコマンド: `cargo wix -b "C:\Program Files (x86)\WiX Toolset v3.14\bin"`
- [ ] 【手動】署名設定（オプション）
  > **オプション**: コード署名証明書の購入・設定が必要。社内配布のみの場合はスキップ可能。

### 自動更新 (既存updaterモジュール拡張)

- [x] `updater/version.rs` 修正: GitHub Releases API対応
  - [x] `GET https://api.github.com/repos/{owner}/{repo}/releases/latest`
  - [x] アセット選択（OS/アーキテクチャ判定）
- [x] `UpdateConfig` に GitHub 設定追加
  ```rust
  pub github_owner: String,  // "yhonda-ohishi-pub-dev"
  pub github_repo: String,   // "rust-router"
  ```
- [x] CLI オプション追加
  - [x] `--check-update`: 更新確認のみ
  - [x] `--update`: 更新実行
  - [x] `--update-channel stable|beta`: 更新チャネル（オプション）
- [x] 定期チェック設定（オプション） - スキップ（基本機能で十分）

### リリース自動化

- [x] GitHub Actions ワークフロー作成
  - [x] タグプッシュ時に自動ビルド
  - [x] Windows: MSI + EXE 作成
  - [x] Linux: バイナリ作成
  - [x] SHA256 チェックサム生成
  - [x] GitHub Releases に自動アップロード

## ファイル命名規則

```
gateway-v{version}-{os}-{arch}.{ext}
gateway-v{version}-{os}-{arch}.{ext}.sha256

例:
gateway-v1.0.0-windows-x86_64.msi
gateway-v1.0.0-windows-x86_64.exe
gateway-v1.0.0-windows-x86_64.exe.sha256
gateway-v1.0.0-linux-x86_64
gateway-v1.0.0-linux-x86_64.sha256
```

## 依存クレート（追加不要）

既存の依存で対応可能:
- `reqwest`: GitHub API 呼び出し
- 自前 SHA256 実装: チェックサム検証

## 参考

- cargo-wix: https://github.com/volks73/cargo-wix
- GitHub Releases API: https://docs.github.com/en/rest/releases

---

# Phase R9: P2P 再接続機能

## 問題

現在の実装では、シグナリングサーバーやWebRTC接続が切断された場合、再接続されずにそのまま終了する。

### 現状の動作

1. **シグナリング切断時** (`signaling.rs:286-292`)
   - `on_disconnected()` イベント発火
   - 状態を `is_connected = false` に設定
   - **その後何もしない** → 再接続なし

2. **WebRTC切断時** (`peer.rs:235-239`)
   - `Disconnected/Failed/Closed` イベント発火
   - **その後何もしない** → ピア再作成なし

## タスク

### シグナリング再接続

- [x] `signaling.rs` に再接続ロジック追加
  - [x] `ReconnectConfig` 構造体追加（最大試行回数、バックオフ設定）
  - [x] `on_disconnected` 時に自動再接続開始
  - [x] Exponential backoff 実装（1s, 2s, 4s, 8s, max 30s）
  - [x] 最大試行回数超過時はエラーイベント発火
  - [x] 再接続成功時は自動で `register_app()` 呼び出し

### WebRTC ピア再接続

- [x] `peer.rs` にピア再作成機能追加
  - [x] `Failed` 状態検出時のクリーンアップ
  - [x] 新しいピア接続の自動作成（必要に応じて）
- [x] `main.rs` の P2P ハンドラ修正
  - [x] ピア切断イベント処理
  - [x] ピアマップからの削除とリソース解放
  - [x] 複数ピア同時接続のサポート（HashMap管理）
  - [x] シグナリング切断時の全ピアクリーンアップ
  - [x] 接続状態のログ出力強化

### 接続監視（保留）

> **保留理由**: 現在のWebSocket実装（tokio-tungstenite）では、接続が切れた場合にread/writeエラーで検出可能。
> シグナリングサーバー側でPing-Pong実装が必要になった場合に対応予定。

- [ ] ハートビート / Ping-Pong 実装（保留）
  - [ ] シグナリングサーバーへの定期Ping（保留）
  - [ ] タイムアウト検出で早期切断判定（保留）

## 実装案

```rust
// signaling.rs
pub struct ReconnectConfig {
    pub max_attempts: u32,          // default: 10
    pub initial_delay: Duration,    // default: 1s
    pub max_delay: Duration,        // default: 30s
    pub backoff_multiplier: f32,    // default: 2.0
}

impl AuthenticatedSignalingClient {
    pub async fn connect_with_reconnect(&mut self) -> Result<(), P2PError> {
        loop {
            match self.connect().await {
                Ok(_) => {
                    self.wait_for_disconnect().await;
                    // 切断検出、再接続開始
                }
                Err(e) => {
                    // バックオフして再試行
                }
            }
        }
    }
}
```

## 参考

- Go版: `scrape-vm/p2p/signaling.go` の再接続実装

---

# Phase R10: gRPC Reflection でシステム情報公開

## 概要

gRPC Reflection 経由でクライアントがシステム情報を取得できるようにする。
OS判定、ログインセッション状態、Chrome利用可否などを公開。

## 公開情報

| フィールド | 型 | 説明 |
|------------|-----|------|
| `os` | string | "windows" / "linux" / "macos" |
| `arch` | string | "x86_64" / "aarch64" |
| `is_windows` | bool | Windows環境かどうか |
| `user_logged_in` | bool | ユーザーセッションがアクティブか（Windows） |
| `chrome_available` | bool | Chrome/Chromiumが利用可能か |
| `scraping_ready` | bool | スクレイピング実行可能か |

## タスク

### Proto定義

- [x] `scraper.proto` に `GetSystemInfo` RPC追加
  ```protobuf
  message SystemInfoRequest {}
  message SystemInfoResponse {
    string os = 1;
    string arch = 2;
    bool is_windows = 3;
    bool user_logged_in = 4;
    bool chrome_available = 5;
    bool scraping_ready = 6;
    string version = 7;
  }

  service ETCScraper {
    rpc GetSystemInfo(SystemInfoRequest) returns (SystemInfoResponse);
  }
  ```

### 実装

- [x] `scraper_service.rs` に `get_system_info` 実装
- [x] Windows セッション判定関数
  ```rust
  #[cfg(windows)]
  fn is_user_logged_in() -> bool {
      // WTSEnumerateSessions または query session
  }

  #[cfg(not(windows))]
  fn is_user_logged_in() -> bool {
      true  // Linux/macOS は常にtrue
  }
  ```
- [x] Chrome 存在チェック
  ```rust
  fn is_chrome_available() -> bool {
      which::which("chrome").is_ok()
          || which::which("chromium").is_ok()
          || cfg!(windows) && Path::new(r"C:\Program Files\Google\Chrome\Application\chrome.exe").exists()
  }
  ```

### クライアント側

- [ ] 【手動】フロントエンドで `GetSystemInfo` 呼び出し
- [ ] 【手動】`scraping_ready = false` の場合、UIでエラー表示
- [ ] 【手動】ログイン待ちの場合、ポーリングでログイン検出

## 依存クレート

```toml
# Windows セッション判定（オプション）
[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = ["Win32_System_RemoteDesktop"] }

# Chrome パス検索
which = "6"
```

## 使用例

```rust
// クライアント側
let info = client.get_system_info(Request::new(SystemInfoRequest {})).await?;

if !info.scraping_ready {
    if info.is_windows && !info.user_logged_in {
        println!("Windows PCにログインしてください");
    } else if !info.chrome_available {
        println!("Chromeをインストールしてください");
    }
}
```

---

# Phase R11: WiX Burn インストーラー（GitHub Release連携）

## 概要

WiX Burn（Bootstrapper）を使用して、GitHub Release から Feature 別にダウンロード・インストールするインストーラーを作成。
インストーラー自体が起動時に自己更新するため、ハッシュ固定の問題を回避。

## アーキテクチャ

```
[GitHub Releases]
├── installer.exe              (500KB) ← ユーザーがダウンロード
├── gateway-full.exe           (15MB)
├── gateway-minimal.exe        (8MB)
├── gateway-p2p.exe            (12MB)
└── feed.xml                          ← インストーラー更新フィード

[インストールフロー]
1. installer.exe 起動
2. feed.xml で自己更新チェック
3. 新しいinstaller.exeがあればダウンロード・再起動
4. Feature選択UI表示（Full / Minimal / P2P のみ）
5. 選択されたバイナリをGitHub Releaseからダウンロード
6. インストール完了
```

## Feature構成

| Feature | 含まれる機能 | サイズ目安 |
|---------|------------|-----------|
| Core | 基本機能（gRPC, 設定） | 5MB |
| P2P | WebRTC, シグナリング | +7MB |
| Updater | 自動更新機能 | +3MB |
| Full | 全機能（Core + P2P + Updater） | 15MB |
| Minimal | Core のみ | 5MB |

## タスク

### Cargo Features 設定

- [x] `gateway/Cargo.toml` に feature 追加
  ```toml
  [features]
  default = ["full"]
  full = ["p2p", "updater"]
  minimal = []
  p2p = ["webrtc"]
  updater = []
  ```

- [ ] 条件付きコンパイル追加（将来タスク）
  > **将来タスク理由**: 現在は全機能を含む単一バイナリで問題ない。
  > Feature別ビルドが必要になった際に実装予定。
  ```rust
  #[cfg(feature = "p2p")]
  mod p2p;

  #[cfg(feature = "updater")]
  mod updater;
  ```

### WiX Burn Bundle

- [x] `installer/Bundle.wxs` 作成
  ```xml
  <Bundle Name="Gateway Installer" Version="1.0.0">
    <!-- 自己更新フィード -->
    <Update Location="https://github.com/.../releases/latest/download/feed.xml"/>

    <BootstrapperApplication>
      <bal:WixStandardBootstrapperApplication Theme="hyperlinkLicense"/>
    </BootstrapperApplication>

    <!-- Feature選択変数 -->
    <Variable Name="InstallFull" Value="1"/>
    <Variable Name="InstallP2P" Value="0"/>
    <Variable Name="InstallMinimal" Value="0"/>

    <Chain>
      <!-- Full版 -->
      <ExePackage Id="GatewayFull"
        DownloadUrl="https://github.com/.../releases/latest/download/gateway-full.exe"
        InstallCondition="InstallFull"
        Compressed="no">
        <RemotePayload Size="15728640" Hash="..."/>
      </ExePackage>

      <!-- Minimal版 -->
      <ExePackage Id="GatewayMinimal"
        DownloadUrl="https://github.com/.../releases/latest/download/gateway-minimal.exe"
        InstallCondition="InstallMinimal"
        Compressed="no">
        <RemotePayload Size="5242880" Hash="..."/>
      </ExePackage>
    </Chain>
  </Bundle>
  ```

### 自己更新フィード

- [x] `feed.xml` テンプレート作成
  ```xml
  <?xml version="1.0" encoding="utf-8"?>
  <Feed>
    <Update Version="1.2.0"
            Location="https://github.com/.../releases/download/v1.2.0/installer.exe"/>
  </Feed>
  ```

### GitHub Actions（自動リリース）

- [x] `.github/workflows/release.yml` 作成
  ```yaml
  name: Release

  on:
    push:
      tags:
        - 'v*'

  jobs:
    build-windows:
      runs-on: windows-latest
      steps:
        - uses: actions/checkout@v4

        - name: Setup Rust
          uses: dtolnay/rust-action@stable

        - name: Build Full
          run: cargo build --release --features full
          working-directory: gateway

        - name: Build Minimal
          run: cargo build --release --features minimal --target-dir target-minimal
          working-directory: gateway

        - name: Build P2P Only
          run: cargo build --release --features p2p --target-dir target-p2p
          working-directory: gateway

        - name: Install WiX
          run: dotnet tool install --global wix

        - name: Build Installer
          run: wix build installer/Bundle.wxs -o installer.exe

        - name: Generate feed.xml
          run: |
            $version = "${{ github.ref_name }}".TrimStart('v')
            @"
            <?xml version="1.0" encoding="utf-8"?>
            <Feed>
              <Update Version="$version"
                      Location="https://github.com/${{ github.repository }}/releases/download/${{ github.ref_name }}/installer.exe"/>
            </Feed>
            "@ | Out-File -FilePath feed.xml -Encoding utf8

        - name: Upload to Release
          uses: softprops/action-gh-release@v1
          with:
            files: |
              gateway/target/release/gateway.exe
              gateway/target-minimal/release/gateway.exe
              gateway/target-p2p/release/gateway.exe
              installer.exe
              feed.xml
            generate_release_notes: true
  ```

### ファイル命名（リリースアセット）

```
gateway-full-v{version}-windows-x86_64.exe
gateway-minimal-v{version}-windows-x86_64.exe
gateway-p2p-v{version}-windows-x86_64.exe
installer-v{version}.exe
feed.xml
```

## ディレクトリ構成

```
gateway/
├── Cargo.toml          # features 追加
├── src/
│   ├── main.rs         # #[cfg(feature = "...")] 追加
│   ├── p2p/            # feature = "p2p"
│   └── updater/        # feature = "updater"
└── installer/
    ├── Bundle.wxs      # Burn Bundle定義
    ├── Theme.xml       # UIカスタマイズ（オプション）
    └── License.rtf     # ライセンス表示
```

## メリット

1. **ハッシュ問題回避**: インストーラー自己更新で常に最新
2. **帯域節約**: 選択したFeatureのみダウンロード
3. **自動化**: tag push で全自動リリース
4. **軽量**: インストーラー本体は500KB程度

## 参考

- WiX v4 Burn: https://wixtoolset.org/docs/tools/burn/
- GitHub Actions for Rust: https://github.com/actions-rust-lang/setup-rust-toolchain
