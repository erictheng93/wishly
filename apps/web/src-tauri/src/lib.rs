use keyring::Entry;

const SERVICE: &str = "tw.wishsync.app";
const USER: &str = "session";

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

#[tauri::command]
fn session_token_get() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
fn session_token_set(token: String) -> Result<(), String> {
    validate_token(&token)?;
    entry()?.set_password(&token).map_err(|e| e.to_string())
}

#[tauri::command]
fn session_token_clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            session_token_get,
            session_token_set,
            session_token_clear
        ])
        .run(tauri::generate_context!())
        .expect("error while running WishSync");
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
