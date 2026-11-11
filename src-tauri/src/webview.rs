use tauri::WebviewWindow;

// Every window is app UI rather than a web page, so the webview's page features are switched off where it has
// a switch for them. The frontend cancels the same behaviors for every platform (src/webDefaults.ts); these
// settings also cover what a page cannot cancel, such as the status bar and autofill.
#[cfg(windows)]
pub fn harden(window: &WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2Settings4, ICoreWebView2Settings5, ICoreWebView2Settings6};
    use windows::core::Interface;

    let _ = window.with_webview(|webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else { return };
        let Ok(settings) = core.Settings() else { return };
        let _ = settings.SetIsZoomControlEnabled(false);
        let _ = settings.SetAreDefaultContextMenusEnabled(false);
        let _ = settings.SetIsStatusBarEnabled(false);
        // Reload, find, print, zoom, history and caret-browsing keys. Editing keys such as Ctrl+C stay.
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings3>() {
            let _ = settings.SetAreBrowserAcceleratorKeysEnabled(false);
        }
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings4>() {
            let _ = settings.SetIsGeneralAutofillEnabled(false);
            let _ = settings.SetIsPasswordAutosaveEnabled(false);
        }
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings5>() {
            let _ = settings.SetIsPinchZoomEnabled(false);
        }
        if let Ok(settings) = settings.cast::<ICoreWebView2Settings6>() {
            let _ = settings.SetIsSwipeNavigationEnabled(false);
        }
    });
}

#[cfg(not(windows))]
pub fn harden(_window: &WebviewWindow) {}
