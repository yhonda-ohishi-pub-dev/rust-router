//! Update installation functionality

use super::UpdateError;
use std::path::Path;

/// Installs downloaded updates
pub struct UpdateInstaller;

impl UpdateInstaller {
    /// Create a new UpdateInstaller
    pub fn new() -> Self {
        Self
    }

    /// Install an update from the given path
    ///
    /// This will:
    /// 1. Backup the current executable
    /// 2. Replace it with the new version
    /// 3. Schedule a restart (on Windows) or exec the new binary (on Unix)
    pub async fn install(&self, update_path: &Path) -> Result<(), UpdateError> {
        let current_exe = std::env::current_exe()
            .map_err(|e| UpdateError::Install(format!("Failed to get current exe path: {}", e)))?;

        // Create backup path
        let backup_path = current_exe.with_extension("exe.bak");

        tracing::info!("Installing update from {:?} to {:?}", update_path, current_exe);
        tracing::info!("Backup will be created at {:?}", backup_path);

        #[cfg(windows)]
        {
            self.install_windows(update_path, &current_exe, &backup_path).await?;
        }

        #[cfg(not(windows))]
        {
            self.install_unix(update_path, &current_exe, &backup_path).await?;
        }

        Ok(())
    }

    #[cfg(windows)]
    async fn install_windows(
        &self,
        update_path: &Path,
        current_exe: &Path,
        backup_path: &Path,
    ) -> Result<(), UpdateError> {
        use std::process::Command;

        // On Windows, we can't replace a running executable directly.
        // We need to create a batch script that will:
        // 1. Wait for the current process to exit
        // 2. Replace the executable
        // 3. Restart the service or application

        let script_path = update_path.with_extension("bat");
        let script_content = format!(
            r#"@echo off
:: Wait for the original process to exit
ping localhost -n 3 > nul

:: Backup current executable
if exist "{current_exe}" (
    copy /Y "{current_exe}" "{backup_path}"
)

:: Replace with new version
copy /Y "{update_path}" "{current_exe}"

:: Clean up
del "{update_path}"

:: Restart the service if it was running as a service
sc query GatewayService > nul 2>&1
if %errorlevel% == 0 (
    net stop GatewayService > nul 2>&1
    net start GatewayService
) else (
    :: Start as regular application
    start "" "{current_exe}" run
)

:: Delete this script
del "%~f0"
"#,
            current_exe = current_exe.display(),
            backup_path = backup_path.display(),
            update_path = update_path.display(),
        );

        tokio::fs::write(&script_path, &script_content).await
            .map_err(|e| UpdateError::Install(format!("Failed to write update script: {}", e)))?;

        // Execute the script in a detached process
        Command::new("cmd")
            .args(["/C", "start", "/B", script_path.to_str().unwrap()])
            .spawn()
            .map_err(|e| UpdateError::Install(format!("Failed to spawn update script: {}", e)))?;

        tracing::info!("Update script scheduled. Application will restart shortly.");

        Ok(())
    }

    #[cfg(not(windows))]
    async fn install_unix(
        &self,
        update_path: &Path,
        current_exe: &Path,
        backup_path: &Path,
    ) -> Result<(), UpdateError> {
        use std::os::unix::fs::PermissionsExt;

        // Backup current executable
        if current_exe.exists() {
            tokio::fs::copy(current_exe, backup_path).await
                .map_err(|e| UpdateError::Install(format!("Failed to backup current exe: {}", e)))?;
        }

        // Replace with new version
        tokio::fs::copy(update_path, current_exe).await
            .map_err(|e| UpdateError::Install(format!("Failed to copy new exe: {}", e)))?;

        // Set executable permissions
        let mut perms = tokio::fs::metadata(current_exe).await
            .map_err(|e| UpdateError::Install(format!("Failed to get file metadata: {}", e)))?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(current_exe, perms).await
            .map_err(|e| UpdateError::Install(format!("Failed to set permissions: {}", e)))?;

        // Clean up downloaded file
        let _ = tokio::fs::remove_file(update_path).await;

        tracing::info!("Update installed. Please restart the application.");

        Ok(())
    }

    /// Rollback to the backup version
    pub async fn rollback(&self) -> Result<(), UpdateError> {
        let current_exe = std::env::current_exe()
            .map_err(|e| UpdateError::Install(format!("Failed to get current exe path: {}", e)))?;

        #[cfg(windows)]
        let backup_path = current_exe.with_extension("exe.bak");
        #[cfg(not(windows))]
        let backup_path = current_exe.with_extension("bak");

        if !backup_path.exists() {
            return Err(UpdateError::Install("No backup found for rollback".to_string()));
        }

        tracing::info!("Rolling back from {:?}", backup_path);

        tokio::fs::copy(&backup_path, &current_exe).await
            .map_err(|e| UpdateError::Install(format!("Failed to restore backup: {}", e)))?;

        Ok(())
    }
}

impl Default for UpdateInstaller {
    fn default() -> Self {
        Self::new()
    }
}
