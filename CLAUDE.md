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
gateway --p2p-setup --p2p-auth-url https://auth.example.com

# APIキー直接指定
gateway --p2p-apikey <key> --p2p-creds ./creds.env
```

クレデンシャルはデフォルトで `~/.config/gateway/p2p_credentials.env` に保存。

## 注意事項

- 共通ライブラリはタグでバージョン固定
- 各サービスは `lib.rs` でライブラリとして公開（gateway から InProcess 呼び出し可能に）
- sqlx のコンパイル時チェックを活用
- P2P認証情報は `.env` ファイルに保存（gitignore 推奨）
