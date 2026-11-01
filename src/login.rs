use objc2_service_management::{SMAppService, SMAppServiceStatus};

fn status() -> SMAppServiceStatus {
    unsafe { SMAppService::mainAppService().status() }
}

/// Whether EQ is set to open when the user logs in, counting a login item that macOS holds
/// until the user allows it.
pub fn enabled() -> bool {
    matches!(status(), SMAppServiceStatus::Enabled | SMAppServiceStatus::RequiresApproval)
}

/// True if EQ will open at login only once the user allows it in System Settings.
pub fn needs_approval() -> bool {
    status() == SMAppServiceStatus::RequiresApproval
}

/// Opens Login Items in System Settings, where the user allows EQ to open at login.
pub fn open_settings() {
    unsafe { SMAppService::openSystemSettingsLoginItems() };
}

/// Adds EQ to, or removes it from, the Login Items in System Settings.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let service = unsafe { SMAppService::mainAppService() };
    let result = unsafe {
        if enabled {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|error| error.localizedDescription().to_string())
}
