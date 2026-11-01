//! Calls: the web view's camera, microphone and screen capture. The calls
//! themselves (encoding with WebCodecs, the relay) live in the screens
//! (`ui/calls.js`).

/// Linux's web view (WebKitGTK) keeps cameras and microphones off unless
/// asked. `XENIFY_FAKE_MEDIA=1` gives it made-up ones (for tests on
/// computers without any).
pub fn enable_media(app: &tauri::App) {
    #[cfg(target_os = "linux")]
    {
        use tauri::Manager;
        let fake = std::env::var_os("XENIFY_FAKE_MEDIA").is_some();
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.with_webview(move |wv| {
                use webkit2gtk::{SettingsExt, WebViewExt};
                if let Some(s) = wv.inner().settings() {
                    s.set_enable_media_stream(true);
                    s.set_enable_mock_capture_devices(fake);
                }
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}
