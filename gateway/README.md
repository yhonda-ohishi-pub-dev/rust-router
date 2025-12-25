# Gateway

API Gateway for gRPC requests - integrates ETC Scraper and Timecard services.

## ビルド

```bash
cargo build --release
```

## 使用方法

### コンソールアプリケーションとして実行

```bash
gateway.exe run
```

### Windows サービスとして使用

#### インストール（管理者権限が必要）

```cmd
gateway.exe install
```

#### サービス開始

```cmd
sc start GatewayService
```

#### サービス停止

```cmd
sc stop GatewayService
```

#### アンインストール（管理者権限が必要）

```cmd
gateway.exe uninstall
```

### ヘルプ

```bash
gateway.exe --help
```

## 設定

環境変数で設定可能：

| 環境変数 | 説明 | デフォルト |
|----------|------|------------|
| `GRPC_ADDR` | gRPC サーバーアドレス | `[::1]:50051` |
| `DOWNLOAD_PATH` | CSV ダウンロード先 | `./downloads` |
| `MAX_CONCURRENT_JOBS` | 最大並列ジョブ数 | `1` |
| `JOB_TIMEOUT_SECS` | ジョブタイムアウト（秒） | `300` |
| `ACCOUNT_DELAY_SECS` | アカウント間の待機時間（秒） | `2` |
| `DEFAULT_HEADLESS` | ヘッドレスモードのデフォルト値 | `true` |
| `RUST_LOG` | ログレベル | `gateway=info` |

## gRPC API

ポート `50051` で以下のサービスを提供：

### GatewayService
- `HealthCheck` - ヘルスチェック
- `GetTimecard` - タイムカード取得
- `CreateTimecard` - タイムカード作成

### ETCScraper
- `Health` - スクレイパーヘルスチェック
- `Scrape` - 単一アカウントのスクレイピング
- `ScrapeMultiple` - 複数アカウントの非同期スクレイピング
- `GetDownloadedFiles` - ダウンロードファイル取得
- `StreamDownload` - ファイルストリーミング

Proto 定義: `proto/gateway.proto`
