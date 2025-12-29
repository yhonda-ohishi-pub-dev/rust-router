# MSI インストール・サービス管理

Gateway の MSI ビルドとインストール、サービス管理の手順。

## MSI ビルド

```bash
cd /c/rust/rust-router/gateway
cargo build --release
cargo wix
```

MSI は `target/wix/gateway-{version}-x86_64.msi` に生成される。

## MSI インストール（管理者権限で実行）

```bash
powershell -Command "Start-Process msiexec -ArgumentList '/i','C:\rust\rust-router\gateway\target\wix\gateway-0.2.39-x86_64.msi','/qb' -Verb RunAs"
```

バージョンは `Cargo.toml` の version に合わせて変更。

## サービス管理

### サービス起動

```bash
powershell -Command "Start-Process sc.exe -ArgumentList 'start','GatewayService' -Verb RunAs"
```

### サービス停止

```bash
powershell -Command "Start-Process sc.exe -ArgumentList 'stop','GatewayService' -Verb RunAs"
```

### サービス状態確認

```bash
powershell -Command "Get-Service GatewayService"
```

## ログ確認

### Event Log 確認

```bash
powershell -Command "Get-WinEvent -FilterHashtable @{LogName='Application'; ProviderName='GatewayService'} -MaxEvents 20 | Format-Table TimeCreated, Message -Wrap"
```

### 特定メッセージでフィルタ

```powershell
Get-WinEvent -FilterHashtable @{LogName='Application'; ProviderName='GatewayService'} -MaxEvents 50 |
  Where-Object { $_.Message -match 'error|Error|failed|Failed' }
```

## 典型的なワークフロー

1. コード修正
2. `cargo build --release`
3. `cargo wix`
4. MSI インストール（上記コマンド）
5. サービス自動起動を待つ（または手動起動）
6. ログ確認

## 注意事項

- MSI インストール時にサービスは自動的に停止・再起動される
- `msiexec` を直接実行すると昇格しないため、`Start-Process -Verb RunAs` を使用
- サービス起動に失敗した場合は Event Log でエラー内容を確認
