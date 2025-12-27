# Gateway

Rust製の軽量APIゲートウェイ。gRPC-Web over WebRTC DataChannel対応。

## インストール

### Windows MSI インストーラー（推奨）

[Releases](https://github.com/yhonda-ohishi-pub-dev/rust-router/releases) から最新の `gateway-vX.X.X-windows-x86_64.msi` をダウンロードして実行。

- PATH環境変数に自動追加
- Windowsサービスとして登録（オプション）
- スタートメニューにショートカット作成

### Windows スタンドアロン

```powershell
# 最新版をダウンロード
Invoke-WebRequest -Uri "https://github.com/yhonda-ohishi-pub-dev/rust-router/releases/latest/download/gateway-v0.2.0-windows-x86_64.exe" -OutFile gateway.exe

# 実行
.\gateway.exe --help
```

### ソースからビルド

```bash
# 要件: Rust, MSVC toolchain
rustup default stable-x86_64-pc-windows-msvc

cd gateway
cargo build --release

# 実行ファイル
./target/release/gateway.exe
```

## 使い方

### 基本

```bash
# gRPCサーバー起動
gateway

# P2P接続（WebRTC）
gateway --p2p-run
```

### P2P セットアップ

```bash
# OAuth認証でクレデンシャル取得
gateway --p2p-setup --p2p-auth-url https://cf-wbrtc-auth.m-tama-ramu.workers.dev

# シグナリングサーバーに接続
gateway --p2p-run
```

### Windowsサービス

```bash
# サービスとしてインストール（管理者権限必要）
gateway install

# サービス開始
net start GatewayService

# サービス停止
net stop GatewayService

# サービスをアンインストール
gateway uninstall
```

### 自動更新

```bash
# 更新確認
gateway --check-update

# 更新実行（exe形式）
gateway --update

# 更新実行（MSIインストーラー形式）
gateway --update-msi
```

サービス実行中でも `--update` で自動的にサービス停止→更新→再開されます。

### Windows Event Log

サービスモードで起動時、Application ログに `GatewayService` として出力されます。

```powershell
# ログ確認
Get-WinEvent -FilterHashtable @{LogName='Application'; ProviderName='GatewayService'} -MaxEvents 10
```

MSIインストール版ではEvent Viewerで正常にメッセージが表示されます。

## 機能

- **gRPC-Web over WebRTC**: ブラウザからNAT越えでgRPC通信
- **P2P接続**: WebRTCによるピアツーピア通信
- **自動更新**: GitHub Releasesからのセルフアップデート（exe/MSI対応）
- **Windowsサービス**: バックグラウンド実行

## 更新履歴

### v0.2.0

- **MSIインストーラー対応**: `--update-msi` オプションでMSI形式の更新に対応
- **SHA256検証強化**: sha2クレートによる標準的なハッシュ計算に変更
- **アセット選択改善**: GitHub Releasesのアセット選択ロジックを改善

## ライセンス

MIT
