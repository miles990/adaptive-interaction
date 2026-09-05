//! 發布來源可信度（release provenance）的回歸測試。
//!
//! 這一組測試不連網、不跑 daemon：它們把「安裝器／CLI 對使用者的宣稱」與
//! 「Release workflow 實際會產出什麼」放在一起比對。宣稱超出實際產出時必須紅燈，
//! 因為那正是使用者會拿到 404 或未驗證二進位的路徑。

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// release.yml 的 CLI matrix 實際會建置的 target triple。
fn built_targets() -> Vec<String> {
    read(".github/workflows/release.yml")
        .lines()
        .filter_map(|l| l.trim().strip_prefix("target: ").map(str::to_string))
        .collect()
}

/// release-provenance-080：安裝器與 CLI 只能宣稱 Release 真的會建置的平台。
#[test]
fn every_triple_the_installer_claims_is_actually_built_by_release_yml() {
    let built = built_targets();
    assert!(
        built.len() >= 4,
        "release.yml CLI matrix 解析失敗（只找到 {built:?}）"
    );

    // CLI：`target_triple()` 的 Ok 分支。
    let selfmgmt = read("crates/interaction-cli/src/selfmgmt.rs");
    let triples = selfmgmt
        .split("=> Ok(\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .filter(|t| t.contains('-') && t.split('-').count() >= 3)
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(
        !triples.is_empty(),
        "selfmgmt.rs 解析不到任何 target triple"
    );
    for t in &triples {
        assert!(
            built.contains(t),
            "selfmgmt.rs 的 target_triple() 宣稱支援 {t}，但 release.yml 從未建置它；\
             使用者只會拿到 404。要嘛把它加進 matrix，要嘛改成 bail!(build from source)"
        );
    }

    // 安裝器：get.sh 的平台偵測。
    let get_sh = read("scripts/get.sh");
    let claimed = get_sh
        .split("TRIPLE=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(!claimed.is_empty(), "get.sh 解析不到任何 TRIPLE");
    for t in &claimed {
        assert!(
            built.contains(t),
            "scripts/get.sh 把某個平台對應到 {t}，但 release.yml 從未建置它"
        );
    }
}

/// release-provenance-080：Linux 桌面 bundle 名稱寫死 amd64，因此非 x86_64 的
/// Linux 必須誠實拒絕，而不是去抓一個不存在的 AppImage。
#[test]
fn install_desktop_does_not_promise_a_bundle_for_unbuilt_linux_arches() {
    let selfmgmt = read("crates/interaction-cli/src/selfmgmt.rs");
    let body = selfmgmt.split("pub fn desktop_asset_name").nth(1).expect(
        "desktop_asset_name 必須是可單獨測試的純函式（cmd_install_desktop 內聯時無法回歸）",
    );
    assert!(
        body.contains("aarch64") || body.contains("x86_64"),
        "desktop_asset_name 必須依 arch 決定 Linux bundle，不能對所有 Linux 一律 amd64"
    );
}

/// release-provenance-078：agent gateway 的版本會送到外部 agent（clientInfo.version），
/// 必須跟著 workspace 版本走，否則 release-prepare 永遠改不到它。
#[test]
fn agent_gateway_version_follows_the_workspace() {
    let manifest = read("crates/interaction-agent-gateway/Cargo.toml");
    assert!(
        manifest.contains("version.workspace = true"),
        "crates/interaction-agent-gateway/Cargo.toml 沒有 `version.workspace = true`；\
         release-prepare.sh 只改 workspace 版本，這個 crate 的版本會停在舊值並經由 \
         codex.rs 的 clientInfo.version 洩漏給外部 agent"
    );
}

/// release-provenance-078（v0.6.x 收尾）：crates/ 與 adapters/ 底下**每一個** crate 都必須
/// `version.workspace = true`。v0.6.0 時 interaction-adapter-declarative／adapters-media 寫死
/// 0.2.0、release-verify 以白名單放行；白名單已移除，這裡守住不再有人寫死自有版本。
#[test]
fn every_crate_version_follows_the_workspace() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for base in ["crates", "adapters"] {
        let Ok(entries) = std::fs::read_dir(root.join(base)) else {
            continue;
        };
        for entry in entries.flatten() {
            let manifest = entry.path().join("Cargo.toml");
            let Ok(src) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let package = src.split("[package]").nth(1).unwrap_or("");
            let package = package.split("\n[").next().unwrap_or("");
            let follows = package
                .lines()
                .any(|line| line.trim_start().starts_with("version.workspace") && line.contains("true"));
            if !follows {
                offenders.push(format!("{base}/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "這些 crate 沒有 `version.workspace = true`（release-prepare.sh 只改 workspace 版本，\
         它們會永遠停在舊值）：{offenders:?}"
    );
}

/// release-provenance-074：缺少 `<asset>.sha256` 必須讓更新失敗，而不是印一行 warning 照裝。
///
/// 政策本身的行為由 `selfmgmt::tests::missing_checksum_is_fail_closed_unless_explicitly_allowed`
/// （`cargo test -p interaction-cli --bins`）覆蓋；這裡守的是「接線」：`verify_checksum` 的
/// 缺校驗檔分支必須真的 `bail!`，而唯一的 `Ok` 必須被明示逃生門擋著。
#[test]
fn missing_checksum_aborts_the_update_instead_of_installing_unverified_bytes() {
    let selfmgmt = read("crates/interaction-cli/src/selfmgmt.rs");
    assert!(
        selfmgmt.contains("fn missing_checksum_policy"),
        "缺少 checksum 的政策必須是可單獨測試的純函式 missing_checksum_policy()"
    );

    // verify_checksum 的 `Err(e) => { … }` 分支（下載 .sha256 失敗時走的那條）。
    let branch = selfmgmt
        .split("async fn verify_checksum")
        .nth(1)
        .expect("verify_checksum 不存在")
        .split("Err(e) =>")
        .nth(1)
        .expect("verify_checksum 沒有處理『抓不到 .sha256』的分支")
        .split("\n}")
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        branch.contains("bail!"),
        "verify_checksum 的『抓不到 .sha256』分支沒有 bail!（fail-open）：\
         中間人只要丟掉那一個請求就能讓 self update 安裝未驗證的二進位。分支內容：{branch}"
    );
    // 分支裡唯一允許回 Ok 的位置，必須在明示逃生門的判斷之後。
    if let Some(ok_at) = branch.find("Ok(())") {
        let policy_at = branch
            .find("missing_checksum_policy")
            .expect("回 Ok 之前沒有先問 missing_checksum_policy()：那就是無條件 fail-open");
        assert!(
            policy_at < ok_at,
            "verify_checksum 在問過 missing_checksum_policy() 之前就回 Ok(())"
        );
        assert!(
            branch.contains("ALLOW_UNVERIFIED_ENV") || branch.contains("UNVERIFIED"),
            "跳過驗證的路徑必須由明示的環境變數逃生門開啟，而不是任何錯誤都放行"
        );
    }
}

/// release-provenance-075：桌面安裝包下載後必須驗證，且 Release 必須發布它的 .sha256。
#[test]
fn desktop_bundles_are_verified_and_have_published_checksums() {
    let selfmgmt = read("crates/interaction-cli/src/selfmgmt.rs");
    let cmd = selfmgmt
        .split("pub async fn cmd_install_desktop")
        .nth(1)
        .expect("cmd_install_desktop 不存在");
    let cmd = cmd.split("\npub ").next().unwrap_or(cmd);
    let verify_at = cmd.find("verify_checksum");
    let open_at = cmd.find("Proc::new(\"open\")");
    assert!(
        verify_at.is_some(),
        "cmd_install_desktop 下載 dmg／AppImage／exe 後完全沒有驗證完整性"
    );
    if let (Some(v), Some(o)) = (verify_at, open_at) {
        assert!(v < o, "cmd_install_desktop 必須先驗證再 open");
    }

    let release = read(".github/workflows/release.yml");
    let desktop = release
        .split("\n  desktop:")
        .nth(1)
        .expect("release.yml 沒有 desktop job")
        .split("\n  extras:")
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        desktop.contains(".sha256"),
        "release.yml 的 desktop job 沒有為 bundle 產生／上傳 .sha256，\
         install-desktop 因此無從驗證"
    );
}

/// release-provenance-072／079：tag → Release 必須先確認被 tag 的 commit 的 CI 全綠，
/// 且 Release 在所有建置 job 成功之前不得公開。
#[test]
fn release_is_gated_on_ci_and_published_only_after_every_build_succeeds() {
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("check-runs"),
        "release.yml 沒有任何 job 查被 tag 的 commit 的 CI check-runs：\
         ci.yml 不在 tag 上跑，這條鏈因此完全沒有 CI 關卡"
    );
    assert!(
        release.contains("--draft"),
        "release.yml 的 create-release 沒有以 --draft 建立：建置期間 Release 就已公開，\
         get.sh／self update 會抓到資產不全的版本"
    );
    assert!(
        release.contains("--draft=false"),
        "release.yml 沒有在所有建置成功後把 draft 轉為已發布的 finalize job"
    );
    assert!(
        !release.contains("releaseDraft: false"),
        "tauri-action 仍帶 releaseDraft: false，會在建置中把 Release 公開"
    );
    let finalize = release
        .split("--draft=false")
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        finalize.contains("needs: [cli, desktop, extras]")
            || finalize.contains("needs:\n      - cli"),
        "finalize job 必須 needs: [cli, desktop, extras]，否則資產不全也會被公開"
    );
}

/// release-provenance-077：CI 關卡必須逐一確認四個必需 job 都在場且成功。
#[test]
fn the_ci_gate_requires_every_job_ci_yml_defines() {
    let helper = repo_root().join("scripts/ci-required-checks.sh");
    assert!(
        helper.exists(),
        "缺少 scripts/ci-required-checks.sh：CI 關卡沒有必需 job 清單，\
         只要有一筆綠色 check-run 就會宣告全綠"
    );
    let verify = read("scripts/release-verify.sh");
    assert!(
        verify.contains("ci-required-checks.sh"),
        "release-verify.sh 的 CI 關卡沒有用必需 job 清單"
    );
    assert!(
        verify.contains("--paginate"),
        "release-verify.sh 的 gh api 沒有 --paginate，check-run 超過 30 筆會靜默截斷"
    );
}

/// release-provenance-073：跳過的關卡不得被呈現成通過。
#[test]
fn release_verify_never_calls_a_skipped_gate_passed() {
    let verify = read("scripts/release-verify.sh");
    assert!(
        verify.contains("SKIPPED") || verify.contains("跳過"),
        "release-verify.sh 跳過 CI／測試時沒有任何輸出"
    );
    assert!(
        verify.contains("passed-with-skips"),
        "release-verify.sh 在有關卡被跳過時仍收尾成無限定詞的 all gates passed"
    );
}

/// release-provenance-071：bash 3.2（macOS 預設）對 `set -u` 下的空陣列展開會中止。
#[test]
fn release_scripts_guard_empty_array_expansion_for_bash_3_2() {
    for script in [
        "scripts/release.sh",
        "scripts/release-prepare.sh",
        "scripts/release-verify.sh",
        "scripts/release-tag.sh",
    ] {
        let body = read(script);
        for (i, line) in body.lines().enumerate() {
            // 未保護的 "${ARR[@]}"：bash 3.2 在 set -u 下會以 unbound variable 結束。
            let unguarded = line
                .match_indices("[@]}\"")
                .any(|(idx, _)| !line[..idx].ends_with("[@]+\"${"));
            assert!(
                !unguarded || line.contains("[@]+"),
                "{script}:{} 有未保護的陣列展開，bash 3.2 下會 unbound variable：{line}",
                i + 1
            );
        }
    }
}
