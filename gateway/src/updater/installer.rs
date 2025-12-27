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
    /// Supports both executable files (.exe) and MSI installers (.msi).
    /// For MSI files, runs msiexec with appropriate flags.
    /// For executables, backs up and replaces the current binary.
    pub async fn install(&self, update_path: &Path) -> Result<(), UpdateError> {
        // Check if this is an MSI file
        let is_msi = update_path
            .extension()
            .map(|ext| ext.to_ascii_lowercase() == "msi")
            .unwrap_or(false);

        if is_msi {
            #[cfg(windows)]
            {
                return self.install_msi(update_path).await;
            }
            #[cfg(not(windows))]
            {
                return Err(UpdateError::Install(
                    "MSI installation is only supported on Windows".to_string()
                ));
            }
        }

        let current_exe = std::env::current_exe()
            .map_err(|e| UpdateError::Install(format!("Failed to get current exe path: {}", e)))?;

        // Create backup path
        let backup_path = current_exe.with_extension("exe.bak");

        tracing::info!("Installing update from {:?} to {:?}", update_path, current_exe);
        tracing::info!("Backup will be created at {:?}", backup_path);

        #[cfg(windows)]
        {
            self.install_windows_exe(update_path, &current_exe, &backup_path).await?;
        }

        #[cfg(not(windows))]
        {
            self.install_unix(update_path, &current_exe, &backup_path).await?;
        }

        Ok(())
    }

    /// Install an MSI package (Windows only)
    #[cfg(windows)]
    async fn install_msi(&self, msi_path: &Path) -> Result<(), UpdateError> {
        use std::process::Command;

        let msi_path_str = msi_path.display().to_string();

        tracing::info!("Installing MSI package: {}", msi_path_str);

        // Create a batch script to run the MSI installer after the current process exits
        let script_path = msi_path.with_extension("bat");
        let script_content = format!(
            r#"@echo off
:: Wait for the original process to exit
ping localhost -n 5 > nul

:: Stop the service if running
sc query GatewayService > nul 2>&1
if %errorlevel% == 0 (
    echo Stopping GatewayService...
    net stop GatewayService > nul 2>&1
    ping localhost -n 3 > nul
)

:: Run the MSI installer silently (upgrade mode)
echo Installing update...
msiexec /i "{msi_path}" /qb /norestart

if errorlevel 1 (
    echo ERROR: MSI installation failed with error %errorlevel%
    pause
)

:: Wait for installation to complete
ping localhost -n 3 > nul

:: Restart the service if it was installed
sc query GatewayService > nul 2>&1
if %errorlevel% == 0 (
    echo Starting GatewayService...
    net start GatewayService
)

:: Clean up
del "{msi_path}"
del "%~f0"
"#,
            msi_path = msi_path_str,
        );

        tokio::fs::write(&script_path, &script_content).await
            .map_err(|e| UpdateError::Install(format!("Failed to write MSI install script: {}", e)))?;

        // Execute the script in a new window so user can see progress
        Command::new("cmd")
            .args(["/C", "start", "Gateway Update", script_path.to_str().unwrap()])
            .spawn()
            .map_err(|e| UpdateError::Install(format!("Failed to spawn MSI install script: {}", e)))?;

        tracing::info!("MSI installation scheduled. Application will restart shortly.");

        // Exit the current process to allow MSI to update files
        std::process::exit(0);
    }

    #[cfg(windows)]
    async fn install_windows_exe(
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
        // Escape paths for batch script (handle spaces and special chars)
        let current_exe_str = current_exe.display().to_string();
        let backup_path_str = backup_path.display().to_string();
        let update_path_str = update_path.display().to_string();

        let script_content = format!(
            r#"@echo off
:: Wait for the original process to exit
ping localhost -n 3 > nul

:: Stop the service first if running (to release file lock)
set SERVICE_WAS_RUNNING=0
sc query GatewayService > nul 2>&1
if %errorlevel% == 0 (
    echo Stopping GatewayService...
    net stop GatewayService > nul 2>&1
    set SERVICE_WAS_RUNNING=1
    ping localhost -n 3 > nul
)

:: Backup current executable
if exist "{current_exe}" (
    copy /Y "{current_exe}" "{backup_path}"
    if errorlevel 1 (
        echo ERROR: Failed to backup current executable
        pause
        exit /b 1
    )
)

:: Replace with new version
copy /Y "{update_path}" "{current_exe}"
if errorlevel 1 (
    echo ERROR: Failed to copy new version
    pause
    exit /b 1
)

:: Clean up downloaded file
del "{update_path}"

:: Wait a moment before restart
ping localhost -n 2 > nul

:: Restart the service if it was running
if %SERVICE_WAS_RUNNING% == 1 (
    echo Starting GatewayService...
    net start GatewayService
) else (
    :: Start as regular application
    echo Starting gateway...
    start "" "{current_exe}" run
)

:: Delete this script
del "%~f0"
"#,
            current_exe = current_exe_str,
            backup_path = backup_path_str,
            update_path = update_path_str,
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
