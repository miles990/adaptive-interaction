//! 子程序管理：process-group 生成＋整樹終止（SIGTERM → 寬限 → SIGKILL）。
//! spec §8.2：正確處理 SIGINT/SIGTERM、取消、子程序樹。

use std::process::Stdio;
use tokio::process::{Child, Command};

/// 以自己的 process group 生成（unix），之後可整樹送訊號。
pub fn spawn_grouped(mut cmd: Command) -> std::io::Result<Child> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    cmd.spawn()
}

#[cfg(unix)]
fn signal_group(pid: u32, sig: libc::c_int) {
    // 負 pid = 整個 process group。
    unsafe {
        libc::kill(-(pid as i32), sig);
    }
}

/// 中斷目前工作（SIGINT 給整組；agent 自行決定是否優雅收尾）。
pub fn interrupt_tree(child: &Child) {
    if let Some(pid) = child.id() {
        #[cfg(unix)]
        signal_group(pid, libc::SIGINT);
        #[cfg(not(unix))]
        let _ = pid;
    }
}

/// 終止整棵子程序樹：SIGTERM → 最多 `grace_ms` → SIGKILL。
pub async fn kill_tree(child: &mut Child, grace_ms: u64) {
    let Some(pid) = child.id() else {
        return; // 已離開
    };
    #[cfg(unix)]
    {
        signal_group(pid, libc::SIGTERM);
        let waited =
            tokio::time::timeout(std::time::Duration::from_millis(grace_ms), child.wait()).await;
        if waited.is_err() {
            signal_group(pid, libc::SIGKILL);
            let _ = child.wait().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = grace_ms;
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}
