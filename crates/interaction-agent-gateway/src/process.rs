//! 子程序管理：process-group 生成＋整樹終止（SIGTERM → 寬限 → SIGKILL）。
//! spec §8.2：正確處理 SIGINT/SIGTERM、取消、子程序樹。

use crate::SessionSpec;
use std::process::Stdio;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};

/// Strip all inherited Runtime authority before spawning. The gateway may
/// subsequently inject one short-lived, session/domain/tool-scoped capability
/// from `SessionSpec`; broad human or Runtime-level tokens never cross into the
/// delegated process. Agent-native login environment remains untouched.
pub fn remove_runtime_auth_env(cmd: &mut Command) {
    for key in [
        "INTERACT_AI_TOKEN",
        "INTERACT_AI_AGENT_TOKEN",
        "INTERACT_AI_HUMAN_TOKEN",
        "INTERACT_AI_SESSION_TOKEN",
        "INTERACT_AI_API_URL",
    ] {
        cmd.env_remove(key);
    }
}

/// Inject only the least-privilege session credential selected by Runtime.
/// It is an environment value (not argv), and expires with the session lease.
pub fn apply_session_capability_env(cmd: &mut Command, spec: &SessionSpec) {
    if let Some(token) = &spec.session_capability_token {
        cmd.env("INTERACT_AI_SESSION_TOKEN", token);
    }
    if let Some(base) = &spec.runtime_api_base {
        cmd.env("INTERACT_AI_API_URL", base);
    }
}

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

/// spawn 當下捕捉的 process group id。kill 路徑不得依賴 Child 的存活狀態
/// （child 被收割後 `id()` 是 None）也不得依賴任何 async 鎖。Arc+atomic 讓
/// `codex exec` fallback 每輪重生子程序時，runtime 鎖外持有的 clone 也能立刻
/// 看見目前 pgid；0 表示沒有進行中的子程序。
#[derive(Debug, Clone)]
pub struct ProcessGroup {
    pgid: Arc<AtomicI32>,
}

impl ProcessGroup {
    /// 沒有子程序時的空 group（codex exec fallback 在第一則訊息前還沒
    /// spawn 任何子程序）：所有訊號操作都是 no-op。
    pub fn empty() -> Self {
        Self {
            pgid: Arc::new(AtomicI32::new(0)),
        }
    }

    /// 必須在 spawn 成功後立刻呼叫（此時 `child.id()` 一定還在）。
    pub fn of(child: &Child) -> Self {
        #[cfg(unix)]
        {
            // process_group(0) ⇒ pgid == 領頭子程序 pid。
            Self {
                pgid: Arc::new(AtomicI32::new(child.id().map(|p| p as i32).unwrap_or(0))),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            Self::empty()
        }
    }

    /// 更新為最新一輪子程序的 group。clone 共用同一 atomic，因此 estop／close
    /// 不必取得 handle 鎖也能終止剛 spawn 的 fallback turn。
    pub fn set_from_child(&self, child: &Child) {
        #[cfg(unix)]
        self.pgid
            .store(child.id().map(|p| p as i32).unwrap_or(0), Ordering::SeqCst);
        #[cfg(not(unix))]
        let _ = child;
    }

    /// 只在這個 pgid 仍是目前值時清空，避免舊 reader 收尾蓋掉下一輪。
    pub fn clear_if(&self, expected: Option<i32>) {
        if let Some(expected) = expected {
            let _ = self
                .pgid
                .compare_exchange(expected, 0, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    pub fn pgid(&self) -> Option<i32> {
        match self.pgid.load(Ordering::SeqCst) {
            0 => None,
            pgid => Some(pgid),
        }
    }

    /// 整組是否還有成員（含未收割的 zombie——kill(-pgid, 0) 對 zombie 也成立，
    /// 所以呼叫端的等待迴圈必須有界）。
    #[cfg(unix)]
    fn alive(&self) -> bool {
        match self.pgid() {
            Some(pgid) => unsafe { libc::kill(-pgid, 0) == 0 },
            None => false,
        }
    }

    #[cfg(unix)]
    fn signal(&self, sig: libc::c_int) {
        if let Some(pgid) = self.pgid() {
            // 負 pid = 整個 process group。
            unsafe {
                libc::kill(-pgid, sig);
            }
        }
    }

    /// 中斷整組（SIGINT；agent 自行決定是否優雅收尾）。不持 Child 也可用
    /// ——codex exec fallback 的 Child 由 reader task 持有，interrupt 只能
    /// 靠 spawn 當下捕捉的 pgid。
    pub fn interrupt(&self) {
        #[cfg(unix)]
        self.signal(libc::SIGINT);
    }

    /// 鎖外整組終止：SIGTERM → 最多 `grace_ms` → SIGKILL，之後有界探測。
    /// 不收割領頭（收割屬於持有 Child 的 kill_tree）；領頭的 zombie 會讓
    /// 探測持續為真，因此兩個迴圈都有界，SIGKILL 對 zombie 是無害 no-op。
    /// 非 unix 沒有 process-group 訊號，由 kill_tree 的 child.kill 收尾。
    pub async fn terminate(&self, grace_ms: u64) {
        #[cfg(unix)]
        {
            if self.pgid().is_none() {
                return;
            }
            self.signal(libc::SIGTERM);
            let steps = (grace_ms / 50).max(1);
            for _ in 0..steps {
                if !self.alive() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            self.reap_stragglers().await;
        }
        #[cfg(not(unix))]
        {
            let _ = grace_ms;
        }
    }

    /// 對殘存成員升級 SIGKILL：探測到 ESRCH（整組清空）或有界期限為止。
    /// 孤兒成員死後由 init 收割，不會永久卡在 zombie；領頭 zombie 只能由
    /// 持有 Child 的一方收割，所以期限用盡就返回（SIGKILL 已送出）。
    #[cfg(unix)]
    async fn reap_stragglers(&self) {
        for _ in 0..20 {
            if !self.alive() {
                return;
            }
            self.signal(libc::SIGKILL);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// 中斷目前工作（SIGINT 給整組；agent 自行決定是否優雅收尾）。
pub fn interrupt_tree(child: &Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGINT);
        }
    }
    #[cfg(not(unix))]
    let _ = child;
}

/// 終止整棵子程序樹：SIGTERM → 最多 `grace_ms` → SIGKILL。
/// 領頭在寬限內退出**不代表**整組已清空：忽略 SIGTERM 的孫程序仍要升級
/// SIGKILL；領頭已被收割（`child.id()` 是 None）時也要照樣對整組送訊——
/// 兩者都靠 spawn 當下捕捉的 pgid，而不是 kill 當下的 child 狀態。
pub async fn kill_tree(child: &mut Child, group: &ProcessGroup, grace_ms: u64) {
    #[cfg(unix)]
    {
        if group.pgid().is_none() {
            // 極端情況：spawn 後沒捕捉到 pid（不應發生）。退回單程序終止。
            let _ = child.kill().await;
            let _ = child.wait().await;
            return;
        }
        group.signal(libc::SIGTERM);
        if child.id().is_some() {
            let waited = tokio::time::timeout(Duration::from_millis(grace_ms), child.wait()).await;
            if waited.is_err() {
                group.signal(libc::SIGKILL);
                let _ = child.wait().await;
            }
        }
        // 領頭已退出並收割；清掉殘存的組員（有界，絕不無限等待）。
        group.reap_stragglers().await;
    }
    #[cfg(not(unix))]
    {
        let _ = (grace_ms, group);
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delegated_process_never_inherits_runtime_capability_tokens() {
        std::env::set_var("INTERACT_AI_TOKEN", "human-secret");
        std::env::set_var("INTERACT_AI_AGENT_TOKEN", "agent-secret");
        let mut cmd = Command::new("sh");
        remove_runtime_auth_env(&mut cmd);
        let output = cmd
            .arg("-c")
            .arg("test -z \"$INTERACT_AI_TOKEN\" && test -z \"$INTERACT_AI_AGENT_TOKEN\"")
            .output()
            .await
            .unwrap();
        std::env::remove_var("INTERACT_AI_TOKEN");
        std::env::remove_var("INTERACT_AI_AGENT_TOKEN");
        assert!(output.status.success());
    }

    fn pid_alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    async fn read_pidfile(path: &std::path::Path) -> i32 {
        for _ in 0..100 {
            if let Some(pid) = std::fs::read_to_string(path)
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
            {
                if pid > 0 {
                    return pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("pidfile never appeared at {}", path.display());
    }

    async fn assert_dies(pid: i32, what: &str) {
        for _ in 0..100 {
            if !pid_alive(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // 清掉殘留避免污染測試機。
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        panic!("{what} (pid {pid}) survived kill_tree");
    }

    /// regression（kill_tree 曾在領頭於寬限內退出時跳過 SIGKILL 升級）：
    /// 領頭收到 SIGTERM 就退出、孫程序忽略 SIGTERM ⇒ 整組仍須被清空。
    #[tokio::test]
    async fn kill_tree_escalates_to_group_even_when_leader_exits_in_grace() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(format!(
            r#"bash -c 'trap "" TERM; while :; do sleep 1; done' & echo $! > "{}"; wait"#,
            pidfile.display()
        ));
        let mut child = spawn_grouped(cmd).unwrap();
        let group = ProcessGroup::of(&child);
        let gpid = read_pidfile(&pidfile).await;
        assert!(pid_alive(gpid), "grandchild running before kill");

        kill_tree(&mut child, &group, 1000).await;
        assert_dies(gpid, "SIGTERM-ignoring grandchild").await;
    }

    /// regression（kill_tree 曾在 child.id() 為 None 時直接 return）：
    /// 領頭已被收割後再呼叫 kill_tree，仍須以 spawn 當下的 pgid 清掉組員。
    #[tokio::test]
    async fn kill_tree_signals_group_even_after_leader_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let mut cmd = Command::new("bash");
        // 領頭把孫程序丟到背景後立刻退出（不 wait）。
        cmd.arg("-c").arg(format!(
            r#"bash -c 'trap "" TERM; while :; do sleep 1; done' & echo $! > "{}""#,
            pidfile.display()
        ));
        let mut child = spawn_grouped(cmd).unwrap();
        let group = ProcessGroup::of(&child);
        let gpid = read_pidfile(&pidfile).await;
        let _ = child.wait().await; // 領頭收割完畢，child.id() 變 None
        assert!(child.id().is_none(), "leader reaped before kill_tree");
        assert!(pid_alive(gpid), "grandchild survives leader exit");

        kill_tree(&mut child, &group, 500).await;
        assert_dies(gpid, "grandchild after leader reaped").await;
    }

    /// 鎖外終止路徑：ProcessGroup::terminate 單獨（沒有 Child）也要能
    /// 升級 SIGKILL 清掉忽略 SIGTERM 的成員。
    #[tokio::test]
    async fn process_group_terminate_kills_sigterm_ignoring_members() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(format!(
            r#"bash -c 'trap "" TERM; while :; do sleep 1; done' & echo $! > "{}"; wait"#,
            pidfile.display()
        ));
        let mut child = spawn_grouped(cmd).unwrap();
        let group = ProcessGroup::of(&child);
        let gpid = read_pidfile(&pidfile).await;

        group.terminate(500).await;
        assert_dies(gpid, "grandchild via terminate").await;
        // 領頭仍由持有 Child 的一方收割。
        let _ = child.wait().await;
    }
}
