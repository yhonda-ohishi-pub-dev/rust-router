# 完了済みタスク

このファイルには plan.md から移動した完了済みのタスクを記録しています。

---

## Phase 1-2: 基盤構築・パイロット移行（完了）

完了済み - 詳細は省略

---

## ETC Scraper Router 実装プラン（Phase R1-R6 完了）

**参照元:** https://github.com/yhonda-ohishi-pub-dev/scrape-vm

### 概要

scrape-vm (Go) を Rust に移行。Router機能とScraper機能を分離。
- **Router**: このリポジトリで実装（gRPC Gateway + ジョブ管理）
- **Scraper**: 別リポジトリで実装（scraper-spec.md 参照）

### アーキテクチャ

```
[External Clients] → gRPC → [router-service] → InProcess → [scraper-service]
                              (このリポジトリ)              (別リポジトリ)
```

### ディレクトリ構成（router-service）

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

### Phase R1: 基盤（完了）

- [x] router-service ディレクトリ作成
- [x] Proto定義（scraper.proto）
- [x] tonic-build設定（build.rs）
- [x] 設定構造体（config.rs）
- [x] ジョブステータス型（job/state.rs）

### Phase R2: gRPCサーバー（完了）

- [x] Health RPC
- [x] Scrape RPC
- [x] ScrapeMultiple RPC（非同期ジョブ）
- [x] GetDownloadedFiles RPC
- [x] StreamDownload RPC

### Phase R3: ジョブ管理（完了）

- [x] ジョブキュー（tokio::sync::mpsc）
- [x] ステータス管理（Arc<RwLock<JobState>>）
- [x] 複数アカウント順次処理

### Phase R4: ScraperService連携（完了）

- [x] ScraperService trait定義
- [x] scraper-service クレート依存追加
- [x] InProcess呼び出し実装

### Phase R5: 運用機能（完了）

- [x] Windowsサービス対応
- [x] 自動更新機能
- [x] P2P通信（webrtc-rs）

### Phase R6: P2P認証（Google OAuth）（完了）

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

---

## Proto 集約計画（完了）

### 概要
shared-lib/proto クレートを作成し、全 proto を集約。feature フラグで選択的利用を可能にする。

### 構成

```
shared-lib/
├── proto/
│   ├── Cargo.toml      # feature: gateway, scraper, timecard, all, reflection
│   ├── build.rs        # tonic-build で一括生成
│   ├── src/lib.rs      # #[cfg(feature = "xxx")] で条件付きエクスポート
│   └── proto/
│       ├── gateway.proto
│       ├── scraper.proto
│       └── timecard.proto
```

### タスク（完了）

- [x] shared-lib/proto クレート作成（feature フラグ付き）
- [x] gateway.proto を shared-lib/proto/proto/ に移動
- [x] gateway の Cargo.toml 更新（proto 依存追加、build.rs 削除）
- [x] gRPC reflection 追加
- [x] ビルド・テスト
- [x] CLAUDE.md に Proto 管理セクション追加

### 使用例

```toml
# gateway（全部使う）
proto = { path = "../shared-lib/proto", features = ["all", "reflection"] }

# 外部プロジェクト（scraper だけ）
proto = { git = "https://github.com/.../rust-router", features = ["scraper"] }
```

### 変更ファイル

- `shared-lib/proto/Cargo.toml` (新規)
- `shared-lib/proto/build.rs` (新規)
- `shared-lib/proto/src/lib.rs` (新規)
- `shared-lib/proto/proto/gateway.proto` (移動)
- `gateway/Cargo.toml` (更新)
- `gateway/build.rs` (削除)
- `gateway/src/grpc/mod.rs` (更新)
- `CLAUDE.md` (Proto 管理セクション追加)

---

## P2P gRPC Bridge 実装（完了）

### 概要

P2P DataChannel 経由の gRPC リクエストを tonic サービスに接続する。
現在は手動で protobuf をエンコードしている箇所を、tonic 生成コードを再利用するように変更。

### 完了タスク

- [x] `grpc_handler.rs` に `TonicServiceBridge` 追加
  - [x] imports 追加 (`bytes`, `http_body_util`, `tower::Service`, `tonic::body::BoxBody`)
  - [x] `TonicServiceBridge<S>` 構造体定義
  - [x] `call()` メソッド（HTTP Request 構築 → tonic サービス呼び出し）
  - [x] `parse_http_response()` メソッド（レスポンス解析）
  - [x] `process_request_with_service()` 関数
- [x] `main.rs` の P2P 部分を `TonicServiceBridge` に移行
  - 現在: 手動 `GrpcRouter` で Health のみ対応
  - 目標: `TonicServiceBridge<EtcScraperServer>` で全メソッド対応
  - 変更点:
    1. `grpc_router` フィールドを `grpc_bridge` に変更
    2. `DataReceived` ハンドラを async 対応（`process_request_with_service` 使用）
- [x] ビルド・テスト実施
- [x] フロントエンドからテスト（Health, ScrapeMultiple）
  - **手動テスト**: ユーザーが実施（自動化対象外）
  - **テスト手順**:
    1. 認証設定（初回のみ）: `gateway --p2p-setup --p2p-auth-url https://cf-wbrtc-auth.m-tama-ramu.workers.dev`
    2. P2P 起動: `gateway --p2p-run`
    3. ブラウザで https://front-js-p2p-grpc.m-tama-ramu.workers.dev/grpc-test にアクセス
    4. Health RPC をテスト
    5. ScrapeMultiple RPC をテスト
  - **ビルド確認済み**: 2025-12-26（Agent #2, Agent #3）
  - **ステータス**: 手動テスト待ち（ユーザーが上記手順でテスト実行可能）

---

## Proto 統一作業（フロントとバックエンドの互換性修正）（完了）

### 問題
フロントエンド（front-js-p2p-grpc）とバックエンド（gateway）で proto 定義が異なり、`ScrapeMultiple` 呼び出しでデシリアライズエラーが発生。

**原因**:
- フロント: `scraper.ScrapeMultipleResponse` → `results`, `success_count`, `total_count` を期待
- バックエンド: `gateway.ScrapeMultipleResponse` → `job_id`, `message` を返していた

**正式proto**: https://github.com/yhonda-ohishi-pub-dev/scrape-vm/blob/main/proto/scraper.proto

### 完了タスク

- [x] `shared-lib/proto/proto/scraper.proto` 作成（フロントの proto に合わせた）
- [x] `shared-lib/proto/build.rs` 修正（`#[cfg(feature = "scraper")]` 追加）
- [x] `shared-lib/proto/src/lib.rs` 修正（`pub mod scraper` 追加）
- [x] `gateway/src/grpc/mod.rs` 修正（`pub mod scraper_server` 追加）
- [x] `gateway/src/grpc/scraper_service.rs` 修正（新しい scraper proto の型を使用）
- [x] ビルド確認: `cargo build`
  - ビルド成功（2025-12-26 確認）
  - warnings のみ（dead_code）
- [x] `main.rs` の修正
  - `EtcScraperServer` のimport元を `scraper_server` に変更済み
  - 現在: `use crate::grpc::scraper_server::etc_scraper_server::EtcScraperServer;`
- [x] テスト（手動）
  - `gateway --p2p-run` で起動
  - https://front-js-p2p-grpc.m-tama-ramu.workers.dev/grpc-test でテスト
  - **完了 (2025-12-26)**: Health, ScrapeMultiple 動作確認済み

---

## Phase R7: ScrapeMultiple 非同期化（完了）

### 問題

現在の `scrape_multiple` は同期処理のため、スクレイピング完了まで WebRTC 接続がタイムアウトする。
Go版と同様に、即座にレスポンスを返してバックグラウンド処理する方式に変更する。

### 完了タスク

- [x] `job/queue.rs` 修正: ジョブ管理機能追加
- [x] `scraper_service.rs` 修正: 非同期処理に変更
- [x] Health API で進捗確認できることを確認（current_job フィールド）
- [x] テスト（2025-12-26 完了）

### 参考

- Go版: `scrape-vm/grpc/server.go` の `ScrapeMultiple` 実装
- フロント: Health API でポーリングして進捗表示

---

## scraper-service 実統合（完了）

### 概要

現在 `scraper_service.rs` はスタブ実装。実際の `scraper-service` クレートを呼び出すように変更する。

### 完了タスク

- [x] `scraper-service` クレートの API 確認
  - git 依存: `https://github.com/yhonda-ohishi-pub-dev/rust-scraper.git`
  - 公開されている trait/struct を確認
  - **完了 (2025-12-26)**: `ScraperService`, `ScrapeRequest`, `ScrapeResult` 確認済み

- [x] `scraper_service.rs` の実装
  - スタブコードを `scraper-service` 呼び出しに置き換え
  - `scrape()`, `scrape_multiple()`, `get_downloaded_files()` の実装
  - **完了 (2025-12-26)**: 全メソッドを scraper-service 経由で実装

- [x] ビルド・テスト
  - **完了 (2025-12-26)**: ビルド成功（警告のみ）、21テスト PASS

### ファイル変更一覧

| ファイル | 状態 | 内容 |
|---------|------|------|
| `shared-lib/proto/proto/scraper.proto` | 新規 | フロント互換のproto定義 |
| `shared-lib/proto/build.rs` | 修正済 | scraper feature追加 |
| `shared-lib/proto/src/lib.rs` | 修正済 | scraper モジュール追加 |
| `gateway/src/grpc/mod.rs` | 修正済 | scraper_server 追加 |
| `gateway/src/grpc/scraper_service.rs` | 修正済 | 新proto型を使用 |
| `gateway/src/main.rs` | 修正済 | EtcScraperServer のimport元変更 |
