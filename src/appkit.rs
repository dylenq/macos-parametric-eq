use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

fn app() -> Option<objc2::rc::Retained<NSApplication>> {
    MainThreadMarker::new().map(NSApplication::sharedApplication)
}

/// Hides EQ the way ⌘H does, handing the foreground to the app behind it.
pub fn hide() {
    if let Some(app) = app() {
        app.hide(None);
    }
}

/// Brings EQ to the foreground.
pub fn activate() {
    if let Some(app) = app() {
        app.activate();
    }
}

/// True while EQ is the foreground app, as it becomes when its Dock icon is clicked.
pub fn is_active() -> bool {
    app().is_some_and(|app| app.isActive())
}
