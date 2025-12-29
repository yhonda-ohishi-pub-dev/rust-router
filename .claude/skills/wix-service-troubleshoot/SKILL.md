---
name: wix-service-troubleshoot
description: WiX MSI インストーラーで Windows Service のアップグレード時にハングする問題のトラブルシューティング
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, WebSearch
---

# WiX Windows Service アップグレード トラブルシューティング

## 問題の症状

- MSI アップグレード時にインストーラーがハングする
- サービスが停止しない（Running のまま）
- ファイルがロックされて更新されない
- Error 1921: Service could not be stopped

## 根本原因

1. **ServiceControl の Stop が効かない**: サービスがシャットダウンに時間がかかる、または応答しない
2. **CustomAction の権限不足**: `Execute='immediate'` では管理者権限がない
3. **タイミング問題**: InstallValidate が StopServices より前に実行される

## 解決策

### 1. WixQuietExec で強制停止（推奨）

```xml
<!-- Stop service before file operations to prevent file lock issues -->
<!-- Use deferred execution with elevated privileges (Impersonate=no) -->
<!-- Use taskkill to forcibly terminate the service process -->
<CustomAction Id='StopServiceBeforeUpgrade_SetProp'
    Property='StopServiceBeforeUpgrade'
    Value='"[SystemFolder]cmd.exe" /c "net stop GatewayService /y &amp; taskkill /f /im gateway-service.exe 2>nul &amp; exit /b 0"'/>
<CustomAction Id='StopServiceBeforeUpgrade'
    BinaryKey='WixCA'
    DllEntry='WixQuietExec'
    Execute='deferred'
    Impersonate='no'
    Return='ignore'/>

<InstallExecuteSequence>
    <Custom Action='StopServiceBeforeUpgrade_SetProp' Before='StopServices'>1</Custom>
    <Custom Action='StopServiceBeforeUpgrade' After='StopServiceBeforeUpgrade_SetProp'>1</Custom>
</InstallExecuteSequence>
```

### 2. ServiceControl のベストプラクティス

```xml
<ServiceControl
    Id='GatewayServiceControl'
    Name='GatewayService'
    Start='install'
    Stop='both'
    Remove='uninstall'
    Wait='yes'/>
```

- `Start='install'`: インストール後にサービス起動
- `Stop='both'`: インストール・アンインストール両方で停止
- `Wait='yes'`: 必須！タイミング競合を防ぐ

### 3. MajorUpgrade の設定

```xml
<MajorUpgrade
    Schedule='afterInstallExecute'
    AllowSameVersionUpgrades='yes'
    DowngradeErrorMessage='...'/>
```

## 重要なポイント

### Deferred CustomAction の要件

- `Execute='deferred'`: 管理者権限で実行
- `Impersonate='no'`: LocalSystem として実行
- `BinaryKey='WixCA'` + `DllEntry='WixQuietExec'`: WiX 標準の静かな実行
- スケジュールは `InstallInitialize` の後でなければならない

### プロパティの渡し方（Deferred CA）

Deferred CustomAction では直接プロパティを参照できないため、SetProperty で同名のプロパティを設定：

```xml
<CustomAction Id='MyAction_SetProp'
    Property='MyAction'
    Value='コマンドライン'/>
<CustomAction Id='MyAction'
    BinaryKey='WixCA'
    DllEntry='WixQuietExec'
    Execute='deferred'
    .../>
```

### net stop vs taskkill

- `net stop ServiceName /y`: 正常停止を試みる（依存サービスも停止）
- `taskkill /f /im process.exe`: 強制終了（応答しないプロセスに有効）
- 両方を組み合わせるのがベスト

## デバッグ方法

### 1. MSI ログを取得

```powershell
msiexec /i "installer.msi" /qb /l*v "C:\Users\username\msi_log.txt"
```

### 2. ログの確認ポイント

```bash
# Unicode ログを変換して検索
iconv -f UTF-16LE -t UTF-8 msi_log.txt | grep -i "Error 1921\|StopService\|WixQuietExec"
```

### 3. 確認すべきエラー

- `Error 1921`: サービス停止失敗
- `value 3`: 致命的エラー
- `Return value`: 各アクションの戻り値

## 参考リンク

- [ServiceControl Element - WiX Documentation](https://wixtoolset.org/docs/schema/wxs/servicecontrol/)
- [Quiet Execution Custom Action](https://wixtoolset.org/docs/v3/customactions/qtexec/)
- [WiX CustomAction Elevated Privileges](https://seshuk.com/2021-07-18-wix-customaction-admin/)
