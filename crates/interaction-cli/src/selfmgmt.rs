//! `interact-ai self …`: install / update / remove / version-check the CLI,
//! install the cross-AI skill, and fetch the desktop control center — all
//! against versioned GitHub Releases with matching asset names.
//!
//! Download strategy: prefer the `gh` CLI when present (works for private
//! repos with the user's auth); fall back to anonymous HTTPS (works once the
//! repo is public, or with GITHUB_TOKEN set).

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

pub const REPO: &str = "miles990/adaptive-interaction";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Rust target triple of THIS build, resolved at runtime (used to pick the
/// matching release asset).
pub fn target_triple() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        // Linux aarch64（樹莓派／Graviton／ARM 容器）目前不在 release.yml 的 CLI matrix 內。
        // 宣稱支援只會讓使用者拿到 HTTP 404，所以誠實說沒有預編譯檔。
        ("linux", "aarch64") => bail!(
            "no prebuilt CLI is published for linux/aarch64 (release.yml does not build it); \
             build from source: cargo install --path crates/interaction-cli"
        ),
        (os, arch) => bail!("unsupported platform {os}/{arch}; build from source with cargo"),
    }
}

/// CLI archive asset name for a tag + triple (must match release.yml).
pub fn cli_asset_name(tag: &str, triple: &str) -> String {
    let ext = if triple.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("interact-ai-{tag}-{triple}.{ext}")
}

/// Normalize a version string: `v0.1.0` / `0.1.0` → (0,1,0).
pub fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn gh_available() -> bool {
    Proc::new("gh")
        .args(["--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Latest release tag, e.g. `v0.1.0`.
pub async fn latest_tag() -> Result<String> {
    if gh_available() {
        let out = Proc::new("gh")
            .args([
                "api",
                &format!("repos/{REPO}/releases/latest"),
                "--jq",
                ".tag_name",
            ])
            .output()
            .context("run gh api")?;
        if out.status.success() {
            let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !tag.is_empty() {
                return Ok(tag);
            }
        }
    }
    // Anonymous / token fallback.
    let client = reqwest::Client::new();
    let mut req = client
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("User-Agent", "interact-ai");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.context("query GitHub releases")?;
    if !resp.status().is_success() {
        bail!(
            "cannot query latest release (HTTP {}); for a private repo install the gh CLI \
             (gh auth login) or set GITHUB_TOKEN",
            resp.status()
        );
    }
    let body: Value = resp.json().await?;
    body["tag_name"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("release response has no tag_name"))
}

/// Download one release asset into `dir`; returns the file path.
pub async fn download_asset(tag: &str, asset: &str, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(asset);
    if gh_available() {
        let out = Proc::new("gh")
            .args([
                "release",
                "download",
                tag,
                "--repo",
                REPO,
                "--pattern",
                asset,
                "--dir",
                dir.to_str().unwrap_or("."),
                "--clobber",
            ])
            .output()
            .context("run gh release download")?;
        if out.status.success() && dest.exists() {
            return Ok(dest);
        }
        eprintln!(
            "gh download failed ({}); trying direct HTTPS…",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
    let client = reqwest::Client::new();
    let mut req = client.get(&url).header("User-Agent", "interact-ai");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!(
            "download failed (HTTP {}). Private repo? Install gh CLI and `gh auth login`, \
             or set GITHUB_TOKEN.",
            resp.status()
        );
    }
    let bytes = resp.bytes().await?;
    std::fs::write(&dest, &bytes)?;
    Ok(dest)
}

fn sha256_hex(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// 明示的逃生門：設成 `1`／`true` 才允許安裝未經 sha256 驗證的位元組。
pub const ALLOW_UNVERIFIED_ENV: &str = "INTERACT_AI_ALLOW_UNVERIFIED_DOWNLOAD";

/// 抓不到 `<asset>.sha256` 時該怎麼辦（純函式，方便回歸測試）。
///
/// 預設 fail-closed：中間人只要丟掉那一個請求，就能讓 `self update` 安裝未驗證的
/// 二進位，所以「校驗檔拿不到」一律當成更新失敗，除非使用者自己開了逃生門。
pub fn missing_checksum_policy(allow_unverified: Option<&str>) -> bool {
    matches!(
        allow_unverified
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Verify `<asset>.sha256` against the downloaded file. Fail-closed: a missing or
/// unreachable checksum aborts the install instead of silently skipping verification.
async fn verify_checksum(tag: &str, asset: &str, file: &Path, dir: &Path) -> Result<()> {
    match download_asset(tag, &format!("{asset}.sha256"), dir).await {
        Ok(sum_file) => {
            let expected = std::fs::read_to_string(&sum_file)?
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_lowercase();
            let actual = sha256_hex(file)?;
            if expected != actual {
                bail!("checksum mismatch for {asset}: expected {expected}, got {actual}");
            }
            eprintln!("checksum ok ({})", &actual[..16]);
            Ok(())
        }
        Err(e) => {
            // 誠實階梯：沒有校驗檔＝完整性未知，不得當成已驗證。
            if missing_checksum_policy(std::env::var(ALLOW_UNVERIFIED_ENV).ok().as_deref()) {
                eprintln!(
                    "warning: no checksum published for {asset} ({e}); installing UNVERIFIED bytes \
                     because {ALLOW_UNVERIFIED_ENV} is set"
                );
                return Ok(());
            }
            bail!(
                "no checksum published for {asset} ({e}); refusing to install unverified bytes. \
                 Re-run when the release is complete, or set {ALLOW_UNVERIFIED_ENV}=1 to accept \
                 an unverified download."
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub async fn cmd_version(check: bool, json: bool) -> Result<i32> {
    if !check {
        if json {
            println!(
                "{}",
                serde_json::json!({"version": CURRENT_VERSION, "repo": REPO})
            );
        } else {
            println!("interact-ai v{CURRENT_VERSION} ({REPO})");
        }
        return Ok(0);
    }
    let latest = latest_tag().await?;
    let current = parse_semver(CURRENT_VERSION);
    let newest = parse_semver(&latest);
    let update_available = matches!((current, newest), (Some(c), Some(n)) if n > c);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "version": CURRENT_VERSION,
                "latest": latest,
                "updateAvailable": update_available,
            })
        );
    } else if update_available {
        println!("v{CURRENT_VERSION} → {latest} 可更新：interact-ai self update");
    } else {
        println!("v{CURRENT_VERSION} 已是最新（latest: {latest}）");
    }
    Ok(0)
}

pub async fn cmd_update(version: Option<String>, dry_run: bool) -> Result<i32> {
    let tag = match version {
        Some(v) if v.starts_with('v') => v,
        Some(v) => format!("v{v}"),
        None => latest_tag().await?,
    };
    if parse_semver(&tag) == parse_semver(CURRENT_VERSION) {
        eprintln!("已是 {tag}，無需更新。");
        return Ok(0);
    }
    let triple = target_triple()?;
    let asset = cli_asset_name(&tag, triple);
    let exe = std::env::current_exe().context("locate current binary")?;
    eprintln!("update: {tag} / {asset}\n  → {}", exe.display());
    if dry_run {
        println!(
            "dry-run: would download {asset} and replace {}",
            exe.display()
        );
        return Ok(0);
    }
    let work = tempfile::tempdir().context("create temp dir")?;
    let archive = download_asset(&tag, &asset, work.path()).await?;
    verify_checksum(&tag, &asset, &archive, work.path()).await?;

    // Extract with the platform tar (bsdtar on macOS/Windows handles both
    // tar.gz and zip; GNU tar handles tar.gz on Linux).
    let status = Proc::new("tar")
        .args(["-xf", archive.to_str().unwrap_or_default()])
        .current_dir(work.path())
        .status()
        .context("extract archive with tar")?;
    if !status.success() {
        bail!("failed to extract {}", archive.display());
    }
    let bin_name = if cfg!(windows) {
        "interact-ai.exe"
    } else {
        "interact-ai"
    };
    let new_bin = work.path().join(bin_name);
    if !new_bin.exists() {
        bail!("archive did not contain {bin_name}");
    }
    // Atomic-ish self replace: move the running binary aside, then move the
    // new one into place (same filesystem as the temp copy next to it).
    let staged = exe.with_extension("new");
    std::fs::copy(&new_bin, &staged).context("stage new binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    let backup = exe.with_extension("old");
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(&exe, &backup).context("move current binary aside")?;
    if let Err(e) = std::fs::rename(&staged, &exe) {
        // Roll back.
        let _ = std::fs::rename(&backup, &exe);
        return Err(anyhow!("install new binary: {e}"));
    }
    let _ = std::fs::remove_file(&backup);
    println!("已更新至 {tag}：{}", exe.display());
    println!("（daemon 若在執行中，重啟後生效：重新執行 interact-ai serve）");
    Ok(0)
}

pub fn cmd_uninstall(purge: bool, yes: bool) -> Result<i32> {
    let exe = std::env::current_exe()?;
    let home = interaction_runtime::Paths::resolve(None).home;
    eprintln!("將移除：{}", exe.display());
    if purge {
        eprintln!("並清除資料目錄：{}", home.display());
    }
    if !yes {
        eprintln!("確認請加 --yes");
        return Ok(2);
    }
    if purge && home.exists() {
        std::fs::remove_dir_all(&home).with_context(|| format!("remove {}", home.display()))?;
        println!("removed {}", home.display());
    }
    // Removing the running binary works on unix; on Windows, schedule-after-exit
    // is not attempted — instruct instead.
    #[cfg(unix)]
    {
        std::fs::remove_file(&exe).with_context(|| format!("remove {}", exe.display()))?;
        println!("removed {}", exe.display());
    }
    #[cfg(windows)]
    {
        println!("Windows 上請於離開後手動刪除：{}", exe.display());
    }
    println!("（skill 若裝過：rm -rf ~/.claude/skills/orchestrate-adaptive-interaction）");
    Ok(0)
}

// ---- embedded skill package (kept in sync with skills/ at build time) ----

const SKILL_FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../../../skills/orchestrate-adaptive-interaction/SKILL.md")),
    (
        "references/cli.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/cli.md"),
    ),
    (
        "references/api.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/api.md"),
    ),
    (
        "references/capabilities.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/capabilities.md"),
    ),
    (
        "references/recipes.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/recipes.md"),
    ),
    (
        "references/receipts.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/receipts.md"),
    ),
    (
        "references/safety.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/safety.md"),
    ),
    (
        "references/tools.md",
        include_str!("../../../skills/orchestrate-adaptive-interaction/references/tools.md"),
    ),
    (
        "scripts/interact-ai",
        include_str!("../../../skills/orchestrate-adaptive-interaction/scripts/interact-ai"),
    ),
    (
        "assets/default-recipes/adaptive-task-completion.yaml",
        include_str!(
            "../../../skills/orchestrate-adaptive-interaction/assets/default-recipes/adaptive-task-completion.yaml"
        ),
    ),
];

pub const SKILL_DIR_NAME: &str = "orchestrate-adaptive-interaction";

/// Agent homes we know how to install skills into. The skill format itself is
/// the open Agent Skills layout (SKILL.md + references/ + scripts/), so any
/// host that reads that layout works; these are just the conventional paths.
const AGENT_HOMES: &[(&str, &str)] = &[
    ("Claude Code", ".claude"),
    ("Codex CLI", ".codex"),
    ("通用 agents 目錄", ".agents"),
    ("Gemini CLI", ".gemini"),
    ("GitHub Copilot CLI", ".copilot"),
];

/// Detect installed agents under `home`: an agent counts as present when its
/// home directory exists; the `skills/` subdir is created on install.
pub fn detect_skill_targets(home: &Path) -> Vec<(String, PathBuf)> {
    AGENT_HOMES
        .iter()
        .filter(|(_, dir)| home.join(dir).is_dir())
        .map(|(name, dir)| {
            (
                name.to_string(),
                home.join(dir).join("skills").join(SKILL_DIR_NAME),
            )
        })
        .collect()
}

fn write_skill_files(dest: &Path) -> Result<()> {
    for (rel, content) in SKILL_FILES {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        if rel.starts_with("scripts/") {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// Install the cross-AI skill from the files embedded in this binary — the
/// skill version always matches the CLI version.
///
/// Cross-AI installation: with no `--dest`, every detected agent home
/// (Claude Code / Codex / ~/.agents / Gemini / Copilot) gets the skill.
pub fn cmd_install_skill(dest: Option<PathBuf>) -> Result<i32> {
    if let Some(dest) = dest {
        write_skill_files(&dest)?;
        println!("skill 已安裝（v{CURRENT_VERSION}）→ {}", dest.display());
        return Ok(0);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mut targets = detect_skill_targets(&home);
    if targets.is_empty() {
        // No agent detected: fall back to the Claude Code convention.
        targets.push((
            "Claude Code（預設）".to_string(),
            home.join(".claude/skills").join(SKILL_DIR_NAME),
        ));
    }
    // Menu on a TTY: all detected agents pre-selected, numbers toggle.
    let mut selected = vec![true; targets.len()];
    {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            loop {
                println!();
                println!("跨 AI Skill 安裝 — 偵測到的 agent（預設全選，輸入編號取消）：");
                for (i, ((agent, dest), on)) in targets.iter().zip(&selected).enumerate() {
                    println!(
                        "  [{}] {}. {agent} → {}",
                        if *on { "x" } else { " " },
                        i + 1,
                        dest.display()
                    );
                }
                print!("切換編號，a=全選，Enter=開始安裝 > ");
                use std::io::Write;
                std::io::stdout().flush().ok();
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line).is_err() {
                    break;
                }
                let choice = line.trim();
                if choice.is_empty() {
                    break;
                }
                if choice.eq_ignore_ascii_case("a") {
                    selected.iter_mut().for_each(|s| *s = true);
                } else if let Ok(n) = choice.parse::<usize>() {
                    if n >= 1 && n <= selected.len() {
                        selected[n - 1] = !selected[n - 1];
                    }
                } else {
                    println!("？輸入編號、a 或直接 Enter");
                }
            }
        }
    }
    let chosen: Vec<&(String, PathBuf)> = targets
        .iter()
        .zip(&selected)
        .filter(|(_, on)| **on)
        .map(|(t, _)| t)
        .collect();
    if chosen.is_empty() {
        println!("未選擇任何 agent；略過 skill 安裝。");
        return Ok(0);
    }
    println!(
        "跨 AI 安裝 skill（v{CURRENT_VERSION}）到 {} 個 agent：",
        chosen.len()
    );
    for (agent, dest) in chosen {
        write_skill_files(dest)?;
        println!("  ✓ {agent} → {}", dest.display());
    }
    println!("其他位置可用 --dest 指定，例如 --dest ~/.config/my-agent/skills/{SKILL_DIR_NAME}");
    println!("（純 function-calling／HTTP 的 AI 不需要 skill：改用 interact-ai tools export）");
    Ok(0)
}

/// 桌面安裝包的資產名稱＋安裝提示（純函式，方便回歸測試平台宣稱）。
///
/// `bare` 是不含 `v` 的版本號。Release 只建置 macOS arm64、Linux x86_64、Windows x64；
/// 其他 arch 沒有 bundle，必須誠實拒絕而不是去抓一個不存在的檔名。
pub fn desktop_asset_name(bare: &str, os: &str, arch: &str) -> Result<(String, &'static str)> {
    match (os, arch) {
        ("macos", "aarch64") => Ok((
            format!("interaction-control-center_{bare}_aarch64.dmg"),
            "打開 dmg 後把 app 拖進 Applications；未簽章／未公證：首次啟動用右鍵→打開，或執行 \
             xattr -dr com.apple.quarantine '/Applications/interaction-control-center.app'",
        )),
        ("linux", "x86_64") => Ok((
            format!("interaction-control-center_{bare}_amd64.AppImage"),
            "chmod +x 後直接執行；或改抓 .deb 用 apt 安裝",
        )),
        ("windows", "x86_64") => Ok((
            format!("interaction-control-center_{bare}_x64-setup.exe"),
            "執行安裝程式（未簽章，Windows SmartScreen 會警告）",
        )),
        (os, arch) => bail!(
            "no desktop bundle is published for {os}/{arch} (release.yml builds macOS arm64, \
             Linux x86_64 and Windows x64 only); build it from source with \
             `cd apps/interaction-desktop && pnpm tauri build`"
        ),
    }
}

/// Download the desktop control center bundle for this platform, verify its
/// published sha256, and only then hand it to the OS installer.
pub async fn cmd_install_desktop(version: Option<String>, out_dir: Option<PathBuf>) -> Result<i32> {
    let tag = match version {
        Some(v) if v.starts_with('v') => v,
        Some(v) => format!("v{v}"),
        None => latest_tag().await?,
    };
    let bare = tag.trim_start_matches('v');
    let (asset, hint) = desktop_asset_name(bare, std::env::consts::OS, std::env::consts::ARCH)?;
    let dir = out_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Downloads")
    });
    eprintln!("downloading {asset} ({tag}) → {}", dir.display());
    let path = download_asset(&tag, &asset, &dir).await?;
    // 074／075：先驗證再交給 OS。校驗失敗或校驗檔缺席都不得往下走 —— 這是
    // 唯一擋在「下載到的位元組」與「使用者按下安裝」之間的關卡（沒有簽章、沒有公證）。
    if let Err(e) = verify_checksum(&tag, &asset, &path, &dir).await {
        let _ = std::fs::remove_file(&path);
        return Err(e).with_context(|| {
            format!("refusing to install {asset}: integrity unverified (downloaded file removed)")
        });
    }
    println!("已下載並通過 sha256 驗證：{}", path.display());
    println!("安裝：{hint}");
    println!(
        "注意：桌面安裝包沒有程式碼簽章／公證，也沒有 SBOM 或 build provenance；\
         sha256 只證明「與 Release 上發布的位元組一致」，不證明來源。"
    );
    #[cfg(target_os = "macos")]
    {
        let _ = Proc::new("open").arg(&path).status();
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parsing_and_ordering() {
        assert_eq!(parse_semver("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("v1.2.3-rc.1"), Some((1, 2, 3)));
        assert!(parse_semver("v0.2.0") > parse_semver("v0.1.9"));
        assert!(parse_semver("v1.0.0") > parse_semver("v0.99.99"));
        assert_eq!(parse_semver("garbage"), None);
    }

    #[test]
    fn asset_names_are_deterministic() {
        assert_eq!(
            cli_asset_name("v0.1.0", "aarch64-apple-darwin"),
            "interact-ai-v0.1.0-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            cli_asset_name("v0.1.0", "x86_64-pc-windows-msvc"),
            "interact-ai-v0.1.0-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn this_platform_has_a_triple() {
        // The test suite only runs on supported platforms.
        assert!(target_triple().is_ok());
    }

    /// release-provenance-080：release.yml 不建置 linux/aarch64，就不能宣稱支援。
    #[test]
    fn unbuilt_platforms_are_refused_instead_of_404ing() {
        // target_triple() 只看真實平台，改不了；改測它可回傳的集合是否越界由
        // tests/release_provenance.rs 比對 release.yml。這裡釘住 desktop 的分支。
        assert!(desktop_asset_name("0.6.0", "linux", "aarch64").is_err());
        assert!(desktop_asset_name("0.6.0", "macos", "x86_64").is_err());
        let (asset, _) = desktop_asset_name("0.6.0", "linux", "x86_64").expect("linux x86_64");
        assert_eq!(asset, "interaction-control-center_0.6.0_amd64.AppImage");
        let (asset, _) = desktop_asset_name("0.6.0", "macos", "aarch64").expect("macos arm64");
        assert_eq!(asset, "interaction-control-center_0.6.0_aarch64.dmg");
    }

    /// release-provenance-074：缺 `<asset>.sha256` 預設必須讓安裝失敗。
    #[test]
    fn missing_checksum_is_fail_closed_unless_explicitly_allowed() {
        assert!(!missing_checksum_policy(None));
        assert!(!missing_checksum_policy(Some("")));
        assert!(!missing_checksum_policy(Some("0")));
        assert!(!missing_checksum_policy(Some("no")));
        assert!(missing_checksum_policy(Some("1")));
        assert!(missing_checksum_policy(Some("true")));
        assert!(missing_checksum_policy(Some(" TRUE ")));
    }

    #[test]
    fn skill_targets_detect_present_agents_only() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude/skills")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap(); // skills/ 缺也算偵測到
                                                                      // .gemini / .agents / .copilot 不存在 → 不列入
        let targets = detect_skill_targets(home.path());
        let agents: Vec<&str> = targets.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(agents, vec!["Claude Code", "Codex CLI"]);
        assert!(targets
            .iter()
            .all(|(_, p)| p.ends_with(format!("skills/{SKILL_DIR_NAME}"))));
    }

    #[test]
    fn embedded_skill_is_complete() {
        let names: Vec<&str> = SKILL_FILES.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"SKILL.md"));
        assert!(
            names
                .iter()
                .filter(|n| n.starts_with("references/"))
                .count()
                >= 7
        );
        // SKILL.md frontmatter intact.
        let skill = SKILL_FILES
            .iter()
            .find(|(n, _)| *n == "SKILL.md")
            .unwrap()
            .1;
        assert!(skill.starts_with("---\nname: orchestrate-adaptive-interaction"));
    }
}
