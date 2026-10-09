//! Load embedded ICO frames through Windows, preserving its alpha/mask handling.
//! Windows selects exact DPI frames and decodes their transparency/masks instead
//! of resizing the framework's single RGBA image for every system icon size.
//! This module owns only its replacement handles, never Tauri's handles.
use std::sync::Mutex;
use tauri::Manager;
use windows_sys::Win32::UI::{
    HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
    WindowsAndMessaging::{
        DestroyIcon, GetWindowLongPtrW, LoadImageW, SendMessageW, GWLP_HINSTANCE, ICON_BIG,
        ICON_SMALL, IMAGE_ICON, SM_CXICON, SM_CXSMICON, SM_CYICON,
        SM_CYSMICON, WM_SETICON,
    },
};

struct NativeIcon(usize);

impl Drop for NativeIcon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0 as _); }
    }
}

#[derive(Default)]
pub(crate) struct Icons(Mutex<Vec<NativeIcon>>);

pub(crate) fn apply(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    // Early DPI events can precede setup. Startup applies icons before showing.
    let Some(state) = window.app_handle().try_state::<Icons>() else { return Ok(()); };
    let mut owned = state.0.lock().map_err(|_| std::io::Error::other("Window icon lock poisoned"))?;
    let hwnd = window.hwnd()?.0;
    // tauri-winres embeds the maintained bundle ICO as resource 32512.
    unsafe {
        let module = GetWindowLongPtrW(hwnd, GWLP_HINSTANCE) as _;
        let dpi = GetDpiForWindow(hwnd).max(96);
        let mut icons = Vec::with_capacity(2);
        for (kind, width_metric, height_metric) in [
            (ICON_BIG, SM_CXICON, SM_CYICON),
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
        ] {
            let icon = LoadImageW(
                module, 32512usize as *const u16, IMAGE_ICON,
                GetSystemMetricsForDpi(width_metric, dpi),
                GetSystemMetricsForDpi(height_metric, dpi), 0,
            );
            if icon.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            icons.push((kind, NativeIcon(icon as usize)));
        }
        for (kind, icon) in &icons {
            SendMessageW(hwnd, WM_SETICON, *kind as usize, icon.0 as isize);
        }
        // WM_SETICON has replaced both handles; previous owned handles can drop.
        *owned = icons.into_iter().map(|(_, icon)| icon).collect();
    }
    Ok(())
}
