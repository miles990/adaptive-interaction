//! §4 版本規則與協商。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 解析 `aip/<major>.<minor>`；其他格式一律 None（不猜）。
///
/// 語法是精確的：**不** trim 前後空白（空白不是版本的一部分，容忍它只會讓三個語言
/// 的界線不一樣——Swift 的 `.whitespaces` 不含換行），major／minor 溢出 u32 也回 None
/// （→ `schema-invalid`，不是 `unsupported-version`：看不懂的字串不叫「不支援的版本」）。
pub fn parse_spec_version(value: &str) -> Option<(u32, u32)> {
    let rest = value.strip_prefix("aip/")?;
    let (major, minor) = rest.split_once('.')?;
    if major.is_empty()
        || minor.is_empty()
        || !major.bytes().all(|b| b.is_ascii_digit())
        || !minor.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

pub fn format_spec_version(major: u32, minor: u32) -> String {
    format!("aip/{major}.{minor}")
}

/// 版本協商結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NegotiatedVersion {
    /// 雙方都能用的版本（同 major、min minor）。
    pub spec_version: String,
    /// 對方 minor 比本實作新（未知選填欄位保留、不崩潰）。
    pub newer_minor: bool,
}

/// 與本實作協商：major 不同 → `unsupported-version`。
pub fn negotiate_version(remote: &str) -> Result<NegotiatedVersion, crate::AipError> {
    let Some((major, minor)) = parse_spec_version(remote) else {
        return Err(crate::AipError::new(
            crate::ErrorCode::SchemaInvalid,
            "specVersion must look like aip/<major>.<minor>",
        ));
    };
    if major != crate::SPEC_MAJOR {
        return Err(crate::AipError::new(
            crate::ErrorCode::UnsupportedVersion,
            format!(
                "unsupported major {major}; this runtime speaks aip/{}.x",
                crate::SPEC_MAJOR
            ),
        ));
    }
    // SPEC_MINOR 目前是 0，所以 min 恆為 0；之後 minor 升級時這行才有效果。
    #[allow(clippy::unnecessary_min_or_max)]
    let negotiated_minor = minor.min(crate::SPEC_MINOR);
    Ok(NegotiatedVersion {
        spec_version: format_spec_version(major, negotiated_minor),
        newer_minor: minor > crate::SPEC_MINOR,
    })
}

/// 從候選清單挑第一個能協商的版本（`capability.specVersions`）。
pub fn negotiate_versions(remote: &[String]) -> Result<NegotiatedVersion, crate::AipError> {
    let mut last = None;
    for candidate in remote {
        match negotiate_version(candidate) {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        crate::AipError::new(
            crate::ErrorCode::UnsupportedVersion,
            "no specVersions offered",
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_negotiate() {
        assert_eq!(parse_spec_version("aip/1.0"), Some((1, 0)));
        assert_eq!(parse_spec_version("aip/1.7"), Some((1, 7)));
        assert_eq!(parse_spec_version("1.0"), None);
        assert_eq!(parse_spec_version("aip/1"), None);
        assert_eq!(parse_spec_version("aip/a.b"), None);
        let same = negotiate_version("aip/1.0").unwrap();
        assert_eq!(same.spec_version, "aip/1.0");
        assert!(!same.newer_minor);
        let newer = negotiate_version("aip/1.3").unwrap();
        assert_eq!(newer.spec_version, "aip/1.0");
        assert!(newer.newer_minor);
        let err = negotiate_version("aip/2.0").unwrap_err();
        assert_eq!(err.code, crate::ErrorCode::UnsupportedVersion);
        let picked = negotiate_versions(&["aip/2.0".into(), "aip/1.0".into()]).unwrap();
        assert_eq!(picked.spec_version, "aip/1.0");
        assert!(negotiate_versions(&[]).is_err());
    }

    /// §4.1 的版本字串是精確語法，不是「大概像」：前後空白、換行都不算合法，
    /// major／minor 溢出 u32 也一律不猜。三個語言必須在同一條界線上（conformance fixture 釘住）。
    #[test]
    fn version_syntax_has_no_fuzzy_edges() {
        for padded in ["aip/1.0\n", " aip/1.0", "aip/1.0 ", "\taip/1.0"] {
            assert_eq!(
                parse_spec_version(padded),
                None,
                "surrounding whitespace must not be trimmed away"
            );
        }
        assert_eq!(parse_spec_version("aip/1.99999999999"), None);
        assert_eq!(parse_spec_version("aip/5000000000.0"), None);
        assert_eq!(
            parse_spec_version("aip/4294967295.4294967295"),
            Some((u32::MAX, u32::MAX))
        );
        // 溢出不是「不支援的版本」，是看不懂的字串。
        assert_eq!(
            negotiate_version("aip/5000000000.0").unwrap_err().code,
            crate::ErrorCode::SchemaInvalid
        );
    }
}
