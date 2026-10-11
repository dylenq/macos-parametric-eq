use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Whether EQ is set to open when the user logs in.
pub fn enabled() -> bool {
    unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
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
