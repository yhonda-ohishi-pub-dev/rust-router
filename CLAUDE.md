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

## frontend repo
"C:\js\front-js-p2p-grpc"

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

## Proto 管理

詳細は `plan.md` の「Proto 集約計画」を参照。

- **proto 集約**: `shared-lib/proto/` に全 proto を配置
- **feature フラグ**: `gateway`, `scraper`, `timecard`, `all`, `reflection`
- **外部利用**: git 依存で feature 指定して必要な proto だけ取得可能

```toml
# 使用例
proto = { path = "../shared-lib/proto", features = ["all", "reflection"] }
```

## リリース

Git の pre-push hook でタグ push 時に自動リリース。

```bash
# リリース手順
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

**自動実行内容:**
1. `cargo build --release`
2. `cargo wix` で MSI インストーラー生成
3. SHA256 チェックサム生成
4. `gh release create` で GitHub Release にアップロード

**生成物:**
- `gateway-vX.X.X-windows-x86_64.exe` - スタンドアロン実行ファイル
- `gateway-vX.X.X-windows-x86_64.msi` - MSI インストーラー

**必要ツール:**
- WiX Toolset v3.14: `winget install WiXToolset.WiXToolset`
- GitHub CLI: `gh auth login`

## Windows Service

### サービス管理

```powershell
# サービス状態確認
Get-Service GatewayService

# サービス開始/停止（管理者権限）
net start GatewayService
net stop GatewayService
```

### Windows Event Log

サービスモードで起動時、Application ログに `GatewayService` として出力。

```powershell
# ログ確認
Get-WinEvent -FilterHashtable @{LogName='Application'; ProviderName='GatewayService'} -MaxEvents 10

# Event Log ソース手動登録（管理者権限、開発時のみ）
New-EventLog -LogName Application -Source GatewayService

# 登録確認
reg query "HKLM\SYSTEM\CurrentControlSet\Services\EventLog\Application\GatewayService"

# 削除
Remove-EventLog -Source GatewayService
```

**実装:**
- `tracing-layer-win-eventlog` クレート使用（tracing layer として直接動作）
- `main.rs` でサービスモード判定（`shutdown_rx.is_some()`）
- MSI インストール時に `util:EventSource` で自動登録

### 自動更新

```powershell
# 更新確認
gateway --check-update

# MSI でアップデート
gateway --update-msi

# EXE でアップデート
gateway --update
```

スタートメニューにも「Gateway Update」「Gateway Check Update」ショートカットあり。

**実装:** `updater/installer.rs` - バッチスクリプト生成 → MSI 実行

## 注意事項

- 共通ライブラリはタグでバージョン固定
- 各サービスは `lib.rs` でライブラリとして公開（gateway から InProcess 呼び出し可能に）
- sqlx のコンパイル時チェックを活用
- P2P認証情報は `.env` ファイルに保存（gitignore済み: `**/p2p_credentials.env`）
- 実装計画・チェックリストは `plan.md` で管理

## サービスモード

```bash
# モード確認
gateway --get-mode

# P2Pモードに切り替え
gateway --set-mode p2p

# gRPCモードに切り替え
gateway --set-mode grpc
```

**レジストリ設定:**
- `HKLM\SOFTWARE\Gateway\ServiceMode`: "p2p" または "grpc"
- `HKLM\SOFTWARE\Gateway\SignalingUrl`: シグナリングサーバーURL

**動作:**
- P2Pモード: WebRTC経由でブラウザからgRPCリクエストを受信
- gRPCモード: 従来のgRPCサーバーとして動作（直接接続）

## 引き継ぎ（2025-12-29）

### 完了した作業
- **v0.2.30 リリース**: pre-push hook でリリース成功
- **GitHub Actions workflow 削除**: ローカル pre-push hook でリリース処理を実行する方針に統一
- **pre-push hook 更新**: pre-release → テスト → stable 昇格フローを追加
- **`--update-from <tag>` オプション実装**: 特定バージョンの MSI/EXE をインストール
  - `gateway --update-from v0.2.30` - EXE でインストール
  - `gateway --update-from v0.2.30 --msi` - MSI でインストール
- **pre-push hook で MSI インストールテスト追加**: `--update-from $TAG --msi` で実際にインストールテスト

### 未解決の問題
- MSI インストールがサービス実行中にハングする場合がある（installer.rs 修正済みだが未テスト）

### 次のステップ
- [ ] 他のgRPCメソッド実装（Scrape, ScrapeMultiple等）
- [ ] 複数peer対応（同時に複数ブラウザからの接続管理）

### 現在のバージョン
- Cargo.toml: `0.2.30`
- 最新リリース: `v0.2.30`
