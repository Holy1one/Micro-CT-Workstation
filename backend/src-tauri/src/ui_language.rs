//! Desktop presentation only. This preference never enters the control engine.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub enum InterfaceLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

#[derive(Default)]
pub struct UiLanguage(AtomicBool);

impl UiLanguage {
    pub fn set(&self, language: InterfaceLanguage) {
        self.0.store(matches!(language, InterfaceLanguage::SimplifiedChinese), Ordering::SeqCst);
    }

    pub fn text<'a>(&self, english: &'a str, chinese: &'a str) -> &'a str {
        if self.0.load(Ordering::SeqCst) { chinese } else { english }
    }
}

#[tauri::command]
pub fn set_interface_language(language: InterfaceLanguage, app: tauri::AppHandle, state: tauri::State<'_, UiLanguage>) -> Result<(), String> {
    state.set(language);
    if let Some(window) = app.get_webview_window("main") {
        // Keep native task switching and the custom title bar on the same brand.
        window.set_title("Micro-CT Workstation").map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_english_and_switches_both_ways() {
        let language = UiLanguage::default();
        assert_eq!(language.text("Camera presets", "相机预设"), "Camera presets");
        language.set(InterfaceLanguage::SimplifiedChinese);
        assert_eq!(language.text("Camera presets", "相机预设"), "相机预设");
        language.set(InterfaceLanguage::English);
        assert_eq!(language.text("Camera presets", "相机预设"), "Camera presets");
    }

    #[test]
    fn ipc_accepts_only_supported_language_codes() {
        for code in ["en", "zh-CN"] {
            assert!(serde_json::from_value::<InterfaceLanguage>(serde_json::json!(code)).is_ok());
        }
        for code in ["", "zh", "fr", "__proto__"] {
            assert!(serde_json::from_value::<InterfaceLanguage>(serde_json::json!(code)).is_err());
        }
    }
}
