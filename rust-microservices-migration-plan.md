# PHP → Rust マイクロサービス移行計画書

## 1. 概要

### 1.1 目的
- PHP 200 API → Rust 15-20サービスに再編
- メモリ使用量削減（500MB-1GB → 30MB以下）
- コンパイラによる品質担保（AI生成コードの検証）
- 起動時間短縮によるオンデマンド運用

### 1.2 アーキテクチャ方針
- **ポリレポ構成**: 各サービスは独立したリポジトリ
- **git依存**: submoduleを使わず、Cargoのgit依存で共通ライブラリを参照
- **ゲートウェイ方式**: gRPCは gateway のみ、各サービスは軽量な内部通信

---

## 2. リポジトリ構成

```
gitlab.com/honda/
├── shared-lib/              # 共通ライブラリ
├── gateway/                 # APIゲートウェイ（gRPC受付）
├── timecard-service/        # タイムカード
├── expense-service/         # 経費管理
├── tachograph-service/      # デジタコ
└── ...                      # 他サービス
```

### 2.1 shared-lib（共通ライブラリ）

```
shared-lib/
├── Cargo.toml              # workspace定義
├── auth/
│   ├── Cargo.toml
│   └── src/lib.rs          # 認証ロジック
├── db/
│   ├── Cargo.toml
│   └── src/lib.rs          # DB接続プール
└── error/
    ├── Cargo.toml
    └── src/lib.rs          # 共通エラー型
```

### 2.2 gateway（APIゲートウェイ）

```
gateway/
├── Cargo.toml
├── proto/
│   └── service.proto
└── src/
    ├── main.rs
    ├── router.rs           # ルーティング
    └── services/           # 各サービスへの振り分け
```

### 2.3 各サービス（例: timecard-service）

```
timecard-service/
├── Cargo.toml
├── src/
│   ├── lib.rs              # ライブラリとして公開
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── punch.rs
│   │   ├── summary.rs
│   │   └── export.rs
│   └── models/
└── .gitlab-ci.yml
```

---

## 3. 依存関係（git依存）

### 3.1 gateway/Cargo.toml

```toml
[package]
name = "gateway"
version = "0.1.0"
edition = "2021"

[dependencies]
# 共通ライブラリ（git依存）
shared-auth = { git = "https://gitlab.com/honda/shared-lib", tag = "v0.1.0" }
shared-db = { git = "https://gitlab.com/honda/shared-lib", tag = "v0.1.0" }
shared-error = { git = "https://gitlab.com/honda/shared-lib", tag = "v0.1.0" }

# 各サービス（git依存 + InProcess呼び出し）
timecard-service = { git = "https://gitlab.com/honda/timecard-service", tag = "v0.1.0" }
expense-service = { git = "https://gitlab.com/honda/expense-service", tag = "v0.1.0" }
tachograph-service = { git = "https://gitlab.com/honda/tachograph-service", tag = "v0.1.0" }

# フレームワーク
tonic = "0.12"
tokio = { version = "1", features = ["full"] }
tower = "0.4"
```

### 3.2 timecard-service/Cargo.toml

```toml
[package]
name = "timecard-service"
version = "0.1.0"
edition = "2021"

[lib]
name = "timecard_service"
path = "src/lib.rs"

[dependencies]
shared-auth = { git = "https://gitlab.com/honda/shared-lib", tag = "v0.1.0" }
shared-db = { git = "https://gitlab.com/honda/shared-lib", tag = "v0.1.0" }
axum = "0.7"
sqlx = { version = "0.7", features = ["mysql", "runtime-tokio"] }
serde = { version = "1", features = ["derive"] }
```

---

## 4. 通信方式

### 4.1 外部 → gateway（gRPC）

```
[外部クライアント]
       │
       │ gRPC (port 50051)
       ▼
   [gateway]
```

### 4.2 gateway → 各サービス（InProcess）

tower::ServiceExt を使い、ネットワークを経由せず直接呼び出し。

```rust
// gateway/src/router.rs
use tower::ServiceExt;
use timecard_service::TimecardService;

pub async fn handle_timecard(req: Request) -> Result<Response, Status> {
    let service = TimecardService::new();
    service.oneshot(req).await
}
```

### 4.3 メリット
- gRPCのオーバーヘッドは外部通信のみ
- 各サービスはtonicに依存しない（軽量）
- テストが容易（サービス単体で呼び出せる）

---

## 5. 運用構成

### 5.1 本番環境（ConoHa VPS）

```
[nginx/Caddy]
      │
      │ リバースプロキシ
      ▼
  [gateway]  ← systemd管理
      │
      ├── timecard-service   (InProcess)
      ├── expense-service    (InProcess)
      └── tachograph-service (InProcess)
```

### 5.2 メモリ見積もり

| 構成 | メモリ使用量 |
|------|-------------|
| PHP（現状） | 500MB - 1GB |
| Rust gateway + 全サービス | 15 - 30MB |

### 5.3 systemd設定例

```ini
# /etc/systemd/system/gateway.service
[Unit]
Description=API Gateway
After=network.target

[Service]
Type=simple
ExecStart=/opt/services/gateway
Restart=always
MemoryMax=64M

[Install]
WantedBy=multi-user.target
```

---

## 6. 開発ワークフロー

### 6.1 ローカル開発（Windows）

```bash
# Rust環境セットアップ
rustup install stable

# サービス単体開発
cd timecard-service
cargo run
cargo test
```

### 6.2 本番ビルド（GitLab CI）

```yaml
# .gitlab-ci.yml
stages:
  - build
  - deploy

build:
  image: rust:latest
  stage: build
  script:
    - cargo build --release
  artifacts:
    paths:
      - target/release/gateway

deploy:
  stage: deploy
  script:
    - scp target/release/gateway vps:/opt/services/
    - ssh vps "systemctl restart gateway"
  only:
    - main
```

### 6.3 バージョン管理

```bash
# shared-lib更新時
cd shared-lib
git tag v0.2.0
git push origin v0.2.0

# 各サービスで更新
cd timecard-service
# Cargo.tomlのタグを更新
cargo update
```

---

## 7. 移行ステップ

### Phase 1: 基盤構築（1-2週間）
- [ ] shared-lib リポジトリ作成（auth, db, error）
- [ ] gateway リポジトリ作成（gRPC受付のみ）
- [ ] GitLab CI パイプライン構築

### Phase 2: パイロット移行（2-4週間）
- [ ] timecard-service をRustで実装
- [ ] gateway から InProcess 呼び出し
- [ ] 既存PHPと並行稼働でテスト

### Phase 3: 横展開（1-2ヶ月）
- [ ] expense-service 移行
- [ ] tachograph-service 移行
- [ ] 他サービス順次移行

### Phase 4: PHP退役（1ヶ月）
- [ ] 全APIがRustで稼働確認
- [ ] PHPアプリケーション停止
- [ ] Apache/PHP-FPM削除

---

## 8. リスクと対策

| リスク | 対策 |
|--------|------|
| Rustの学習コスト | AIによるコード生成 + コンパイラで品質担保 |
| ビルド時間が長い | CI/CDに任せる、ローカルは差分ビルド |
| 共通ライブラリ変更の影響 | タグでバージョン固定、段階的更新 |
| DBスキーマ変更 | sqlxのコンパイル時チェック活用 |

---

## 9. 成功指標

- メモリ使用量: 500MB → 30MB以下
- 起動時間: 数秒 → 数ms
- ビルド成功 = 型安全性担保
- 100名分タイムカード処理: 数分 → 数秒
