mod file_store;

#[cfg(not(target_os = "android"))]
use keyring::Entry;

#[cfg(not(target_os = "android"))]
const SERVICE: &str = "tw.wishly.app";
#[cfg(not(target_os = "android"))]
const USER: &str = "session";

#[cfg(not(target_os = "android"))]
fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, USER).map_err(|e| e.to_string())
}

/// 非空、<= 512 bytes、可列印 ASCII（0x21..=0x7E，不含空白）。
fn validate_token(t: &str) -> Result<(), String> {
    if t.is_empty() || t.len() > 512 {
        return Err("invalid token length".into());
    }
    if !t.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("invalid token characters".into());
    }
    Ok(())
}

/// 權杖後端：桌面 / iOS 走 OS keychain；Android 沒有 keyring 後端，改存 App 私有目錄（見 file_store）。
#[cfg(not(target_os = "android"))]
mod backend {
    pub fn get() -> Result<Option<String>, String> {
        match super::entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn set(t: &str) -> Result<(), String> {
        super::entry()?.set_password(t).map_err(|e| e.to_string())
    }
    pub fn clear() -> Result<(), String> {
        match super::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(target_os = "android")]
mod backend {
    use std::{path::PathBuf, sync::OnceLock};
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    pub fn init(dir: PathBuf) {
        let _ = DIR.set(dir);
    }
    fn dir() -> Result<&'static PathBuf, String> {
        DIR.get()
            .ok_or_else(|| "session store not initialised".to_string())
    }
    pub fn get() -> Result<Option<String>, String> {
        crate::file_store::get(dir()?)
    }
    pub fn set(t: &str) -> Result<(), String> {
        crate::file_store::set(dir()?, t)
    }
    pub fn clear() -> Result<(), String> {
        crate::file_store::clear(dir()?)
    }
}

#[tauri::command]
fn session_token_get() -> Result<Option<String>, String> {
    backend::get()
}

#[tauri::command]
fn session_token_set(token: String) -> Result<(), String> {
    validate_token(&token)?;
    backend::set(&token)
}

#[tauri::command]
fn session_token_clear() -> Result<(), String> {
    backend::clear()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // 桌面端：第二次 deep link 會啟動第二個行程，須先註冊（且要在 deep-link 之前）才會轉送給既有視窗。
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        use tauri::Manager;
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }));
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            session_token_get,
            session_token_set,
            session_token_clear
        ])
        .setup(|app| {
            let _ = &app; // 僅 Android / Windows·Linux dev 會用到
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                backend::init(app.path().app_local_data_dir()?);
            }
            // Windows / Linux 開發時沒有安裝包，執行期註冊 scheme（macOS 只認打包後的 .app）
            #[cfg(all(debug_assertions, any(windows, target_os = "linux")))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register_all()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Wishly");
}

#[cfg(test)]
mod tests {
    use super::validate_token;

    #[test]
    fn token_validation() {
        assert!(validate_token("abcDEF123-_.~").is_ok());
        assert!(validate_token(&"a".repeat(512)).is_ok());
        assert!(validate_token("").is_err());
        assert!(validate_token(&"a".repeat(513)).is_err());
        assert!(validate_token("has space").is_err());
        assert!(validate_token("new\nline").is_err());
        assert!(validate_token("中文").is_err());
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod command_tests {
    use super::*;
    use keyring::credential::{Credential, CredentialApi, CredentialBuilderApi};
    use keyring::mock::MockCredential;
    use std::{any::Any, sync::Arc};

    // The built-in mock is EntryOnly; commands create a new Entry each time.
    // Share its credential through a test-only builder to model real persistence.
    struct Store(Arc<MockCredential>);

    impl CredentialApi for Store {
        fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
            self.0.set_secret(secret)
        }
        fn get_secret(&self) -> keyring::Result<Vec<u8>> {
            self.0.get_secret()
        }
        fn delete_credential(&self) -> keyring::Result<()> {
            self.0.delete_credential()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl CredentialBuilderApi for Store {
        fn build(
            &self,
            target: Option<&str>,
            service: &str,
            user: &str,
        ) -> keyring::Result<Box<Credential>> {
            assert_eq!((target, service, user), (None, "tw.wishly.app", "session"));
            Ok(Box::new(Store(self.0.clone())))
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn commands_persist_replace_clear_and_propagate_keychain_errors() {
        let mock = Arc::new(MockCredential::default());
        keyring::set_default_credential_builder(Box::new(Store(mock.clone())));
        assert_eq!(session_token_get().unwrap(), None);
        assert!(session_token_clear().is_ok());
        session_token_set("first-token".into()).unwrap();
        assert_eq!(session_token_get().unwrap().as_deref(), Some("first-token"));
        session_token_set("replacement-token".into()).unwrap();
        assert_eq!(
            session_token_get().unwrap().as_deref(),
            Some("replacement-token")
        );

        for invalid in [
            "".into(),
            "a".repeat(513),
            "has space".into(),
            "中文".into(),
        ] {
            assert!(session_token_set(invalid).is_err());
            assert_eq!(
                session_token_get().unwrap().as_deref(),
                Some("replacement-token")
            );
        }
        session_token_clear().unwrap();
        assert_eq!(session_token_get().unwrap(), None);
        assert!(session_token_clear().is_ok());

        mock.set_error(keyring::Error::Invalid(
            "store".into(),
            "unavailable".into(),
        ));
        assert!(session_token_get().unwrap_err().contains("unavailable"));
        mock.set_error(keyring::Error::Invalid(
            "store".into(),
            "unavailable".into(),
        ));
        assert!(session_token_set("valid-token".into())
            .unwrap_err()
            .contains("unavailable"));
        assert_eq!(session_token_get().unwrap(), None);
        session_token_set("kept-token".into()).unwrap();
        mock.set_error(keyring::Error::Invalid(
            "store".into(),
            "unavailable".into(),
        ));
        assert!(session_token_clear().unwrap_err().contains("unavailable"));
        assert_eq!(session_token_get().unwrap().as_deref(), Some("kept-token"));
        session_token_clear().unwrap();
    }
}
