# Scraper Service 仕様書

**このファイルは別リポジトリ（scraper-service）で読み込んで実装する**

**参照元:** https://github.com/yhonda-ohishi-pub-dev/scrape-vm

## 概要

ETC利用照会サービス（etc-meisai.jp）から利用明細CSVを自動ダウンロードするスクレイパー。
Router（router-service）からInProcessで呼び出される。

## アーキテクチャ

```
[router-service] → tower::Service → [scraper-service]
                    InProcess呼び出し
```

## ディレクトリ構成

```
scraper-service/
├── Cargo.toml
└── src/
    ├── lib.rs            # ライブラリエントリーポイント
    ├── traits.rs         # Scraper trait定義
    ├── config.rs         # ScraperConfig
    ├── error.rs          # ScraperError
    ├── service.rs        # tower::Service実装
    └── etc/
        ├── mod.rs
        └── scraper.rs    # ETC Scraper実装
```

## 実装チェックリスト

### Phase S1: 基盤

- [ ] scraper-service リポジトリ作成
- [ ] Scraper trait定義
- [ ] ScraperConfig 構造体
- [ ] ScraperError 定義

### Phase S2: ETC Scraper実装

- [ ] ブラウザ初期化（headless_chrome / chromiumoxide）
- [ ] ダウンロード許可設定
- [ ] ログインページ遷移
- [ ] 認証情報入力・ログイン
- [ ] 検索ページ遷移
- [ ] CSVダウンロードリンククリック
- [ ] ダウンロード完了待機
- [ ] ファイルリネーム（アカウント名付与）

### Phase S3: tower::Service実装

- [ ] Service<ScrapeRequest> 実装
- [ ] InProcess呼び出し対応
- [ ] エラーハンドリング

## Scraper Trait

```rust
use async_trait::async_trait;
use std::path::PathBuf;

#[async_trait]
pub trait Scraper: Send + Sync {
    /// ブラウザ初期化
    async fn initialize(&mut self) -> Result<(), ScraperError>;

    /// ログイン実行
    async fn login(&mut self) -> Result<(), ScraperError>;

    /// CSVダウンロード
    async fn download(&mut self) -> Result<PathBuf, ScraperError>;

    /// リソース解放
    async fn close(&mut self) -> Result<(), ScraperError>;
}
```

## 設定構造体

```rust
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ScraperConfig {
    pub user_id: String,
    pub password: String,
    pub download_path: PathBuf,
    pub headless: bool,
    pub timeout: Duration,
}

impl Default for ScraperConfig {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            password: String::new(),
            download_path: PathBuf::from("./downloads"),
            headless: true,
            timeout: Duration::from_secs(60),
        }
    }
}
```

## エラー型

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScraperError {
    #[error("ブラウザ初期化エラー: {0}")]
    BrowserInit(String),

    #[error("ナビゲーションエラー: {0}")]
    Navigation(String),

    #[error("ログインエラー: {0}")]
    Login(String),

    #[error("ダウンロードエラー: {0}")]
    Download(String),

    #[error("タイムアウト: {0}")]
    Timeout(String),

    #[error("ファイル操作エラー: {0}")]
    FileIO(#[from] std::io::Error),
}
```

## tower::Service実装

```rust
use tower::Service;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::future::Future;

pub struct ScrapeRequest {
    pub user_id: String,
    pub password: String,
    pub download_path: PathBuf,
    pub headless: bool,
}

pub struct ScrapeResult {
    pub csv_path: PathBuf,
    pub csv_content: Vec<u8>,
}

pub struct ScraperService {
    // 設定など
}

impl Service<ScrapeRequest> for ScraperService {
    type Response = ScrapeResult;
    type Error = ScraperError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: ScrapeRequest) -> Self::Future {
        Box::pin(async move {
            // スクレイピング実行
            todo!()
        })
    }
}
```

## ETC Scraper 実装詳細

### ログインフロー

1. `https://www.etc-meisai.jp/` にアクセス
2. ログインリンク（`funccode=1013000000`）をクリック
3. ログインフォームに認証情報入力
   - `input[name='risLoginId']` → ユーザーID
   - `input[name='risPassword']` → パスワード
4. ログインボタン（`input[type='button'][value='ログイン']`）をクリック

### ダウンロードフロー

1. 「検索条件の指定」リンクをクリック
2. 「全て」オプション（`input[name='sokoKbn'][value='0']`）を選択
3. 設定保存ボタン（`input[name='focusTarget_Save']`）をクリック
4. 検索ボタン（`input[name='focusTarget']`）をクリック
5. CSVダウンロードリンク（「明細」「CSV」を含むリンク）をクリック
6. ダウンロード完了を待機（最大30秒）
7. ファイル名を `{user_id}_{original_name}.csv` にリネーム

### ダウンロード監視

Go版では以下の方法でダウンロードを検出:
- `browser.SetDownloadBehavior` でダウンロード許可
- `chromedp.ListenBrowser` でダウンロードイベント監視
- ファイルポーリングで完了検出

Rust版でも同様のアプローチ:
- headless_chrome: `Browser::set_download_behavior`
- ダウンロードディレクトリをポーリング監視

## 依存クレート

```toml
[dependencies]
# ブラウザ自動化（どちらか選択）
headless_chrome = "1.0"
# または
# chromiumoxide = "0.5"

# 非同期
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# tower Service
tower = "0.4"

# エラー
thiserror = "1"

# ログ
tracing = "0.1"
```

## 注意事項

- Chromeがインストールされている必要あり
- headless=false でデバッグ可能
- ダウンロードタイムアウト: 30秒
- セッションごとにタイムスタンプ付きフォルダを作成
- アカウント間で2秒の待機時間を設ける
