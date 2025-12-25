# CLAUDE.md

## プロジェクト概要

PHP 200 API を Rust 15-20 マイクロサービスに移行するプロジェクト。

**目標:**
- メモリ使用量: 500MB-1GB → 30MB以下
- コンパイラによる型安全性担保
- 起動時間短縮によるオンデマンド運用

## 技術スタック

- **言語:** Rust (edition 2021)
- **通信:** gRPC (tonic) - gateway のみ
- **内部通信:** tower::ServiceExt による InProcess 呼び出し
- **Web:** axum
- **DB:** sqlx (MySQL)
- **シリアライズ:** serde
- **非同期:** tokio

## アーキテクチャ

```
[外部クライアント] → gRPC → [gateway] → InProcess → [各サービス]
```

- **ポリレポ構成**: 各サービスは独立したリポジトリ
- **git依存**: Cargo の git 依存で共通ライブラリを参照（submodule 不使用）
- **gateway方式**: gRPC は gateway のみで受付

## リポジトリ構成

```
gitlab.com/honda/
├── shared-lib/           # 共通ライブラリ (auth, db, error)
├── gateway/              # APIゲートウェイ
├── timecard-service/     # タイムカード
├── expense-service/      # 経費管理
└── tachograph-service/   # デジタコ
```

## 開発コマンド

```bash
# ビルド
cargo build
cargo build --release

# テスト
cargo test

# 実行
cargo run

# 依存更新
cargo update
```

## 重要なファイル

- `Cargo.toml` - 依存関係定義、git依存でタグ指定
- `src/lib.rs` - サービスのライブラリエントリーポイント
- `.gitlab-ci.yml` - CI/CD パイプライン

## Gateway モジュール構成

```
gateway/src/
├── main.rs           # エントリーポイント、CLI処理
├── lib.rs            # ライブラリエクスポート
├── config.rs         # 設定
├── grpc/             # gRPCサービス実装
├── job/              # ジョブキュー管理
├── p2p/              # P2P通信モジュール
│   ├── auth.rs       # OAuth認証フロー
│   ├── credentials.rs # クレデンシャル管理
│   ├── signaling.rs  # WebSocketシグナリング
│   ├── peer.rs       # WebRTCピア接続
│   └── channel.rs    # データチャネル
└── updater/          # 自動更新機能
```

## P2P認証

cf-wbrtc-auth サーバーとの OAuth 認証を使用。

```bash
# OAuth セットアップ
gateway --p2p-setup --p2p-auth-url https://cf-wbrtc-auth.m-tama-ramu.workers.dev

# APIキー直接指定
gateway --p2p-apikey <key> --p2p-creds ./creds.env
```

クレデンシャルはデフォルトで `~/.config/gateway/p2p_credentials.env` に保存。

### クレデンシャルファイル形式

```env
API_KEY=xxx
APP_ID=xxx
REFRESH_TOKEN=rt_xxx
```

`P2P_API_KEY`, `P2P_APP_ID`, `P2P_REFRESH_TOKEN` 形式も対応。

### トークンリフレッシュ

```
POST /api/app/refresh
Content-Type: application/json

{"refreshToken": "rt_xxx"}
```

レスポンス: `{"apiKey": "...", "appId": "...", "refreshToken": "..."}`

### テスト

```bash
# ユニットテスト
cargo test p2p --lib
cargo test auth --lib

# 実サーバー統合テスト（要クレデンシャルファイル）
cargo test test_real_refresh --lib -- --ignored --nocapture
```

## P2P接続（シグナリング）

```bash
# シグナリングサーバーに接続
gateway --p2p-run

# カスタムシグナリングURL指定
gateway --p2p-run --p2p-signaling-url wss://example.com/ws/app
```

### 現状（2025-12-26 更新）

- **完了**: OAuth認証、シグナリング接続、アプリ登録、WebRTC実装、gRPC-Web over DataChannel
- **WebRTC実装完了**: webrtc-rs v0.12 使用

### 実装済み機能

1. **peer.rs** - WebRTC PeerConnection実装
   - `P2PPeer::new()` - RTCPeerConnection作成
   - `create_answer()` - offerからanswer SDP生成
   - `setup_handlers()` - ICE candidate収集ハンドラ
   - `setup_data_channel_handler()` - DataChannel受信ハンドラ
   - `add_ice_candidate()` - リモートICE candidate追加
   - `send()` - DataChannel経由でデータ送信

2. **grpc_handler.rs** - gRPC-Web over DataChannel
   - `parse_request()` - リクエストパース（path, headers, protobuf message）
   - `encode_response()` - レスポンスエンコード（headers, data frames, trailer frame）
   - `GrpcRouter` - メソッドハンドラのルーティング
   - `process_request()` - リクエスト処理（x-request-id自動コピー）

3. **main.rs** - `on_offer`ハンドラ
   - ブラウザからのoffer受信 → answer生成 → シグナリングで送信
   - ICE candidate交換
   - gRPCリクエスト処理（Health check実装済み）

### gRPC-Web プロトコル形式

**リクエスト:**
```
[path_len(4)][path(N)][headers_len(4)][headers_json(M)][grpc_frames]
```

**レスポンス:**
```
[headers_len(4)][headers_json(N)][data_frames...][trailer_frame]
```

**gRPC-Web Frame:**
```
[flags(1)][length(4)][data(N)]
flags: 0x00 = data, 0x01 = trailer
```

### ビルド環境

**重要**: Windows環境ではMSVCツールチェインが必要（ringクレートのビルドに必要）

```bash
# MSVCツールチェインに切り替え
rustup default stable-x86_64-pc-windows-msvc

# ビルド
cargo build
```

### 次のステップ

1. **他のgRPCメソッド実装**: Scrape, ScrapeMultiple, GetDownloadedFiles等
2. **複数peer対応**: 同時に複数ブラウザからの接続を管理
3. **protobufライブラリ統合**: prost等で自動エンコード/デコード

### 関連リソース

- **フロントエンド**: `C:\js\cf-wbrtc-auth\` (Cloudflare Workers)
- **管理UI**: https://cf-wbrtc-auth.m-tama-ramu.workers.dev/
- **gRPCテストページ**: https://front-js-p2p-grpc.m-tama-ramu.workers.dev/grpc-test

## 注意事項

- 共通ライブラリはタグでバージョン固定
- 各サービスは `lib.rs` でライブラリとして公開（gateway から InProcess 呼び出し可能に）
- sqlx のコンパイル時チェックを活用
- P2P認証情報は `.env` ファイルに保存（gitignore済み: `**/p2p_credentials.env`）
