# 移行チェックリスト

## Phase 1: 基盤構築

- [x] shared-lib リポジトリ作成
  - [x] auth クレート作成
  - [x] db クレート作成
  - [x] error クレート作成
- [x] gateway リポジトリ作成（gRPC受付のみ）
- [x] GitLab CI パイプライン構築

## Phase 2: パイロット移行

- [x] timecard-service を Rust で実装
- [x] gateway から InProcess 呼び出し
- [x] 既存 PHP と並行稼働でテスト

## Phase 3: 横展開

- [ ] expense-service 移行
- [ ] tachograph-service 移行
- [ ] 他サービス順次移行

## Phase 4: PHP 退役

- [ ] 全 API が Rust で稼働確認
- [ ] PHP アプリケーション停止
- [ ] Apache/PHP-FPM 削除

---

# ETC Scraper Router 実装プラン

**参照元:** https://github.com/yhonda-ohishi-pub-dev/scrape-vm

## 概要

scrape-vm (Go) を Rust に移行。Router機能とScraper機能を分離。
- **Router**: このリポジトリで実装（gRPC Gateway + ジョブ管理）
- **Scraper**: 別リポジトリで実装（scraper-spec.md 参照）

## アーキテクチャ

```
[External Clients] → gRPC → [router-service] → InProcess → [scraper-service]
                              (このリポジトリ)              (別リポジトリ)
```

## ディレクトリ構成（router-service）

```
router-service/
├── Cargo.toml
├── build.rs              # tonic-build
├── proto/
│   └── scraper.proto
└── src/
    ├── main.rs
    ├── lib.rs
    ├── config.rs
    ├── grpc/
    │   ├── mod.rs
    │   └── handlers.rs
    └── job/
        ├── mod.rs
        ├── queue.rs
        └── state.rs
```

## 実装チェックリスト

### Phase R1: 基盤

- [x] router-service ディレクトリ作成
- [x] Proto定義（scraper.proto）
- [x] tonic-build設定（build.rs）
- [x] 設定構造体（config.rs）
- [x] ジョブステータス型（job/state.rs）

### Phase R2: gRPCサーバー

- [x] Health RPC
- [x] Scrape RPC
- [x] ScrapeMultiple RPC（非同期ジョブ）
- [x] GetDownloadedFiles RPC
- [x] StreamDownload RPC

### Phase R3: ジョブ管理

- [x] ジョブキュー（tokio::sync::mpsc）
- [x] ステータス管理（Arc<RwLock<JobState>>）
- [x] 複数アカウント順次処理

### Phase R4: ScraperService連携

- [x] ScraperService trait定義
- [x] scraper-service クレート依存追加
- [x] InProcess呼び出し実装

### Phase R5: 運用機能（オプション）

- [x] Windowsサービス対応
- [x] 自動更新機能
- [x] P2P通信（webrtc-rs）

### Phase R6: P2P認証（Google OAuth）

**参照元:** scrape-vm の `p2p/setup.go`, `p2p/signaling.go`

- [x] OAuth セットアップフロー実装
  - [x] cf-wbrtc-auth サーバーとの連携
  - [x] ポーリング方式でトークン取得
  - [x] ブラウザでの Google 認証 URL 表示
- [x] クレデンシャル管理
  - [x] APIキー保存・読み込み（`p2p_credentials.env`）
  - [x] リフレッシュトークン対応
- [x] シグナリング認証
  - [x] WebSocket 接続時の APIキー認証
  - [x] 認証済みアプリ登録（AppRegister）
- [x] コマンドライン対応
  - [x] `--p2p-setup` フラグ（手動セットアップ）
  - [x] `--p2p-apikey`, `--p2p-creds` オプション
  - [x] `--p2p-auth-url` オプション

#### 必要な依存クレート

```toml
# P2P OAuth
reqwest = { version = "0.12", features = ["json"] }
tokio-tungstenite = "0.24"
open = "5"  # ブラウザ起動用
```

#### 実装ファイル

```
gateway/src/p2p/
├── mod.rs
├── signaling.rs    # 認証付き WebSocket 通信
├── peer.rs
├── channel.rs
├── auth.rs         # NEW: OAuth セットアップ
└── credentials.rs  # NEW: クレデンシャル管理
```

## gRPC API（Go版互換）

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
