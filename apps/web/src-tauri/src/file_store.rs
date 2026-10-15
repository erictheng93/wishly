//! Android 的權杖存放：沒有 keyring 後端，寫進 App 私有目錄（沙箱內、權限 0600、原子替換）。
//! ponytail: 僅靠 OS 沙箱保護，未用 Android Keystore 加密；上限是 root / 允許備份時可被取出。
//! 升級路徑：Kotlin 版 Keystore plugin（AES-GCM 金鑰存 AndroidKeyStore）。須在 gen/android 的 manifest 設 allowBackup=false。
#![cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
use std::{fs, io, path::Path};

const FILE: &str = "session_token";

pub fn get(dir: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(dir.join(FILE)) {
        Ok(t) if t.is_empty() => Ok(None),
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set(dir: &Path, token: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    write_private(&tmp, token).map_err(|e| e.to_string())?;
    fs::rename(&tmp, dir.join(FILE)).map_err(|e| e.to_string())
}

pub fn clear(dir: &Path) -> Result<(), String> {
    match fs::remove_file(dir.join(FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn write_private(path: &Path, data: &str) -> io::Result<()> {
    use std::io::Write;
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    f.write_all(data.as_bytes())?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_replace_clear() {
        let dir = std::env::temp_dir().join(format!("ws-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(get(&dir).unwrap(), None);
        assert!(clear(&dir).is_ok());
        set(&dir, "first").unwrap();
        assert_eq!(get(&dir).unwrap().as_deref(), Some("first"));
        set(&dir, "second-longer").unwrap();
        set(&dir, "short").unwrap();
        assert_eq!(get(&dir).unwrap().as_deref(), Some("short"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
            assert_eq!(m & 0o777, 0o600);
        }
        clear(&dir).unwrap();
        assert_eq!(get(&dir).unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
