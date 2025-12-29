//! Update installation functionality

use super::UpdateError;
use std::path::Path;

/// Service status check result
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    /// Service does not exist
    NotInstalled,
    /// Service is running
    Running,
    /// Service is stopped
    Stopped,
    /// Service is marked for deletion (requires reboot)
    PendingDeletion,
    /// Unknown state
    Unknown(String),
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceStatus::NotInstalled => write!(f, "Not installed"),
            ServiceStatus::Running => write!(f, "Running"),
            ServiceStatus::Stopped => write!(f, "Stopped"),
            ServiceStatus::PendingDeletion => write!(f, "Pending deletion (reboot required)"),
            ServiceStatus::Unknown(s) => write!(f, "Unknown: {}", s),
        }
    }
}

/// Check if the GatewayService is in a clean state for installation
#[cfg(windows)]
pub fn check_service_status() -> ServiceStatus {
    use std::process::Command;

    // First check if service exists using sc query
    let output = Command::new("sc")
        .args(["query", "GatewayService"])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            // Service doesn't exist
            if stderr.contains("1060") || stdout.contains("1060") {
                return ServiceStatus::NotInstalled;
            }

            // Check for "PENDING" or deletion markers
            if stdout.contains("DELETE_PENDING") || stdout.contains("STOP_PENDING") {
                return ServiceStatus::PendingDeletion;
            }

            // Parse state
            if stdout.contains("RUNNING") {
                return ServiceStatus::Running;
            }
            if stdout.contains("STOPPED") {
                return ServiceStatus::Stopped;
            }

            // Try to get more info - check if service can be queried
            let qc_output = Command::new("sc")
                .args(["qc", "GatewayService"])
                .output();

            if let Ok(qc) = qc_output {
                let qc_stderr = String::from_utf8_lossy(&qc.stderr);
                // Error 1072: The specified service has been marked for deletion
                if qc_stderr.contains("1072") {
                    return ServiceStatus::PendingDeletion;
                }
            }

            ServiceStatus::Unknown(stdout.to_string())
        }
        Err(e) => ServiceStatus::Unknown(format!("Failed to query service: {}", e)),
    }
}

#[cfg(not(windows))]
pub fn check_service_status() -> ServiceStatus {
    ServiceStatus::NotInstalled
}

/// Check if service is ready for MSI installation
/// Returns Ok(()) if ready, Err with message if not
pub fn check_service_ready_for_install() -> Result<(), String> {
    let status = check_service_status();

    match status {
        ServiceStatus::NotInstalled => Ok(()),
        ServiceStatus::Stopped => Ok(()),
        ServiceStatus::Running => {
            // Running is OK - MSI will stop it via StopServiceBeforeUpgrade
            Ok(())
        }
        ServiceStatus::PendingDeletion => {
            Err("Service is marked for deletion. Please reboot your computer first.".to_string())
        }
        ServiceStatus::Unknown(s) => {
            tracing::warn!("Unknown service status: {}", s);
            Ok(()) // Allow installation attempt
        }
    }
}

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

        // Check if service is in a clean state before attempting install
        if let Err(msg) = check_service_ready_for_install() {
            return Err(UpdateError::Install(msg));
        }

        let status = check_service_status();
        tracing::info!("Service status before install: {}", status);

        let msi_path_str = msi_path.display().to_string();

        tracing::info!("Installing MSI package: {}", msi_path_str);

        // Use PowerShell script for better process control
        let script_path = msi_path.with_extension("ps1");
        let script_content = format!(
            r#"# Wait for the original process to exit
Start-Sleep -Seconds 5

# Kill any existing msiexec processes (from previous failed installs)
Get-Process -Name msiexec -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

# Stop the service if running
Stop-Service -Name GatewayService -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 3

# Kill any remaining gateway.exe processes
Get-Process -Name gateway -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

# Run the MSI installer with Basic UI (upgrade mode)
Write-Host "Installing update..."
$process = Start-Process -FilePath "msiexec.exe" -ArgumentList "/i", '"{msi_path}"', "/qb", "/norestart" -PassThru
$timeout = 120
$waited = 0
while (!$process.HasExited -and $waited -lt $timeout) {{
    Start-Sleep -Seconds 1
    $waited++
}}
if (!$process.HasExited) {{
    Write-Host "ERROR: MSI installation timed out after $timeout seconds"
    $process | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 3
    exit 1
}}
if ($process.ExitCode -ne 0) {{
    Write-Host "ERROR: MSI installation failed with exit code $($process.ExitCode)"
    Start-Sleep -Seconds 5
    exit 1
}}

# Wait for installation to complete
Start-Sleep -Seconds 3

# Restart the service if it was installed
$service = Get-Service -Name GatewayService -ErrorAction SilentlyContinue
if ($service) {{
    Write-Host "Starting GatewayService..."
    Start-Service -Name GatewayService -ErrorAction SilentlyContinue
}}

Write-Host "Update completed successfully."
Start-Sleep -Seconds 2

# Clean up MSI file
Remove-Item -Path "{msi_path}" -Force -ErrorAction SilentlyContinue

# Clean up this script
Remove-Item -Path $MyInvocation.MyCommand.Path -Force -ErrorAction SilentlyContinue

exit
"#,
            msi_path = msi_path_str.replace('\\', "\\\\"),
        );

        tokio::fs::write(&script_path, &script_content).await
            .map_err(|e| UpdateError::Install(format!("Failed to write MSI install script: {}", e)))?;

        // Execute the PowerShell script with UAC elevation (Run as Administrator)
        Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "Start-Process powershell -Verb RunAs -ArgumentList '-ExecutionPolicy Bypass -NoProfile -File \"{}\"'",
                    script_path.display()
                )
            ])
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
        echo Press any key to exit...
        pause > nul
        exit /b 1
    )
)

:: Replace with new version
copy /Y "{update_path}" "{current_exe}"
if errorlevel 1 (
    echo ERROR: Failed to copy new version
    echo Press any key to exit...
    pause > nul
    exit /b 1
)

:: Clean up downloaded file
del "{update_path}" > nul 2>&1

:: Wait a moment before restart
ping localhost -n 2 > nul

:: Restart the service if it was running
if %SERVICE_WAS_RUNNING% == 1 (
    echo Starting GatewayService...
    net start GatewayService
)

echo Update completed successfully.

:: Delete this script
del "%~f0" > nul 2>&1
exit
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
