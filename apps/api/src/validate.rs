//! 集中式輸入驗證（F-03 / F-11 / F-16 / F-17）。回傳 422 VALIDATION_FAILED 並指出欄位 pointer。
use crate::error::AppError;
use chrono::NaiveDate;

fn zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}
fn bidi_override(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// trim 後檢查：拒絕控制字元（含 NUL；`multi` 時保留 \n \r \t）與雙向覆寫字元。
/// 去除零寬字元後為空（只剩空白/零寬）者視為空字串回傳，由呼叫端依「必填/選填」處理。
pub fn text(s: &str, pointer: &str, multi: bool) -> Result<String, AppError> {
    let s = s.trim();
    let bad = s.chars().any(|c| {
        bidi_override(c) || (c.is_control() && !(multi && matches!(c, '\n' | '\r' | '\t')))
    });
    if bad {
        return Err(AppError::invalid(
            pointer,
            "INVALID_CHARS",
            "含有不允許的控制或方向覆寫字元",
        ));
    }
    if s.chars().all(|c| c.is_whitespace() || zero_width(c)) {
        return Ok(String::new());
    }
    Ok(s.to_string())
}

/// 活動日限制 2000-01-01..2100-12-31
pub fn event_date(d: NaiveDate, pointer: &str) -> Result<NaiveDate, AppError> {
    let (lo, hi) = (
        NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2100, 12, 31).unwrap(),
    );
    if (lo..=hi).contains(&d) {
        Ok(d)
    } else {
        Err(AppError::invalid(
            pointer,
            "RANGE",
            "日期須介於 2000-01-01 與 2100-12-31",
        ))
    }
}

/// 分享代碼：10 碼 ASCII 英數（含 %00 等一律視為不存在）
pub fn slug_ok(s: &str) -> bool {
    s.len() == 10 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// 統一編號檢查碼（財政部 2023 新制）：權重 1,2,1,2,1,2,4,1，各積的十位與個位相加後總和須被 5 整除；
/// 第 7 碼為 7 時，總和 +1 被 5 整除也算通過。
pub fn tax_id_ok(s: &str) -> bool {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let d: Vec<u32> = s.bytes().map(|b| (b - b'0') as u32).collect();
    let sum: u32 = d.iter().zip([1, 2, 1, 2, 1, 2, 4, 1]).map(|(a, w)| { let p = a * w; p / 10 + p % 10 }).sum();
    sum % 5 == 0 || (d[6] == 7 && (sum + 1) % 5 == 0)
}

#[cfg(test)]
mod tax_id_tests {
    #[test]
    fn tax_id_checksum() {
        assert!(super::tax_id_ok("04595257"));
        assert!(super::tax_id_ok("12345675")); // 第 7 碼 7 的特例
        assert!(!super::tax_id_ok("12345678"));
        assert!(!super::tax_id_ok("1234567"));
        assert!(!super::tax_id_ok("1234567a"));
    }
}
