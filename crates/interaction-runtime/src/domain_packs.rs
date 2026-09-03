//! Built-in versioned Domain Packs (spec §10). These are local reference
//! packages, never prompts with authority. Installation is user-controlled
//! and persisted; uninstalling keeps a pack absent across restarts.

use crate::runtime::Runtime;
use interaction_core::{DomainError, DomainPack, DomainResult};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const INSTALLED_META_KEY: &str = "installed_builtin_domain_packs_v1";

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[allow(clippy::too_many_arguments)]
fn pack(
    id: &str,
    display_name: &str,
    concepts: &[&str],
    principles: &[&str],
    workflow: &[&str],
    heuristics: &[&str],
    failure_patterns: &[&str],
    counterexamples: &[&str],
    quality_rubric: &[&str],
    verification: &[&str],
    applicability: &[&str],
    limitations: &[&str],
) -> DomainPack {
    DomainPack {
        id: id.into(),
        display_name: display_name.into(),
        version: "1.0.0".into(),
        supersedes: vec![],
        concepts: strings(concepts),
        principles: strings(principles),
        workflow: strings(workflow),
        heuristics: strings(heuristics),
        failure_patterns: strings(failure_patterns),
        counterexamples: strings(counterexamples),
        quality_rubric: strings(quality_rubric),
        verification: strings(verification),
        sources: vec![
            "repo://docs/ARCHITECTURE.md".into(),
            "repo://docs/capability-completion-matrix.md".into(),
        ],
        applicability: strings(applicability),
        limitations: strings(limitations),
    }
}

pub fn builtin_domain_packs() -> Vec<DomainPack> {
    vec![
        pack(
            "human-ai-interaction",
            "Human × AI Interaction",
            &[
                "人類意圖、系統狀態與 AI 建議是不同事實層",
                "非語言回應應先於高打擾對話",
            ],
            &[
                "安全與誠實優先於角色表現",
                "一般介面先回答使用者現在需要知道的事",
            ],
            &["辨識需求", "選擇最低打擾介面", "呈現可取消行動", "觀察結果"],
            &["先用弱訊號，確實需要決定時才升級", "每個技術狀態附人類語意"],
            &["用人格淡化失敗", "把 claimed-completed 畫成 verified"],
            &["工作完成動畫不能取代獨立驗證"],
            &["狀態可理解", "主要操作可撤銷", "安全文字固定"],
            &["以一般模式完成任務", "核對 Runtime receipt 與 UI 一致"],
            &["桌面角色、控制中心與通知流程"],
            &["不是醫療或心理診斷方法"],
        ),
        pack(
            "agent-delegation",
            "Agent Delegation",
            &[
                "Provider、Agent、Session、Task 必須分離",
                "委派權限由 lease 與 scope 限制",
            ],
            &["最小必要 Context Bundle", "Agent claim 不等於結果已驗證"],
            &[
                "預覽資料與成本",
                "建立有期限 Session",
                "監看進度",
                "獨立驗證",
                "關閉 Session",
            ],
            &["程式工作優先 Codex，長文件優先 Claude；模糊任務先讓人選"],
            &["Agent 自行擴張資料", "循環委派", "沒有 watchdog 的長任務"],
            &["第二 Agent 的複審仍不能替代確定性測試"],
            &["scope 最小", "可取消", "有 receipt", "claim/verified 分開"],
            &["檢查 session token、lease、mailbox、取消傳播與驗證收據"],
            &["本機 Codex／Claude Code 工作階段"],
            &["路由只是建議，不是安全決策"],
        ),
        pack(
            "privacy-consent",
            "Privacy／Consent",
            &[
                "Consent、Lease、資料範圍、外部傳送是獨立約束",
                "敏感感測預設關閉",
            ],
            &["有效權限取所有限制的最小值", "撤銷必須向執行中工作傳播"],
            &[
                "說明誰要做什麼",
                "列出資料與去向",
                "取得明確同意",
                "發行短效 lease",
                "到期或撤銷",
            ],
            &["掃描只讀 metadata", "每個 receptor/actuator 個別授權"],
            &["把看見按鈕當成已同意", "把主動發話權當成行動權"],
            &["選擇顯示角色不代表同意麥克風或檔案傳送"],
            &["預設關閉", "範圍明確", "可撤銷", "無 secret 落庫"],
            &["撤銷後核對 Runtime、Tray、UI、API 與 CLI 的一致狀態"],
            &["裝置、Agent、記憶與外部傳送"],
            &["不能取代各平台 OS 權限與法規審查"],
        ),
        pack(
            "task-planning",
            "Task Planning",
            &["目標、限制、依賴、驗證與停止條件", "可逆與不可逆步驟"],
            &["先取得足夠事實再選行動", "長任務必須有期限與取消"],
            &[
                "定義完成條件",
                "分解依賴",
                "標記風險與同意",
                "執行",
                "逐步驗證",
            ],
            &["優先完成可驗證的垂直切片", "在外部副作用前放置檢查點"],
            &["只有待辦沒有驗證", "以 Phase 完成冒充產品完成"],
            &["建立 Schema 但未接 Runtime 不是完成"],
            &["每步有 owner、輸入、輸出、期限與證據"],
            &["從最終 DoD 反向追蹤每項機器證據"],
            &["軟體、研究與內容任務"],
            &["未知領域需要額外專家或來源複審"],
        ),
        pack(
            "result-verification",
            "Result Verification",
            &[
                "acknowledged、completed、verified、uncertain 是不同狀態",
                "Receipt 記錄主張與證據",
            ],
            &["驗證者應盡量獨立於執行者", "沒有證據時維持 Unknown"],
            &[
                "收集 claim",
                "檢查 artifact/hash",
                "執行獨立觀察或測試",
                "寫入 verdict",
            ],
            &["優先使用可重跑測試與內容 hash", "負向測試應證明安全閘門"],
            &["用成功動畫當證據", "只看 exit code 不看實際狀態"],
            &["Agent 回覆 OK 不等於外部效果成功"],
            &["證據可重跑", "correlation 可追蹤", "Unknown 不被掩蓋"],
            &["重跑命令並比對 Receipt、Log、Artifact hash"],
            &["Agent 任務、裝置效果與知識發布"],
            &["部分外部效果沒有可用的獨立觀察器"],
        ),
        pack(
            "desktop-character-behavior-animation",
            "Desktop Character Behavior／Animation",
            &["生命底層、行為層、語意層分離", "attention 與動作可中斷性"],
            &["逐幀生命感不依賴生成式 AI", "Emergency 固定安全姿態"],
            &[
                "察覺",
                "轉移注意",
                "選擇反應強度",
                "執行可中斷行為",
                "自然恢復",
            ],
            &["眼先於頭，頭先於身", "用 hazard sampling 避免固定週期"],
            &[
                "固定 N 秒重播",
                "Unknown 播放 Success",
                "隱藏後仍收角色事件",
            ],
            &["耳朵與尾巴可以表達狀態，但不能暗示取得權限"],
            &["縮小仍可辨識", "Reduced Motion 正確", "短期不重複"],
            &["用 seeded 測試檢查轉場、打斷與恢復"],
            &["Character Pack 與 Behavior Runtime"],
            &["不處理任意動畫程式碼或角色外視窗控制"],
        ),
        pack(
            "software-project-inspection",
            "Software Project Inspection",
            &[
                "基準 commit、dirty worktree、架構邊界、測試層",
                "變更與既有使用者內容分離",
            ],
            &["先讀專案規則與現況", "搜尋優先使用結構化工具與 rg"],
            &[
                "記錄基準",
                "讀 README/架構/Schema/測試",
                "建立差距",
                "小步修改",
                "完整回歸",
            ],
            &["先跑最小失敗測試再擴大", "以 diff 檢查意外變更"],
            &[
                "覆蓋 dirty worktree",
                "用舊 binary 做 E2E",
                "只測 mock provider",
            ],
            &["編譯通過不代表功能 E2E 完成"],
            &["格式、lint、unit、integration、E2E、文件與證據一致"],
            &["核對 git diff、測試數字、實際 binary hash"],
            &["本機軟體 repository 審查與實作"],
            &["不自行 push、release 或改外部狀態"],
        ),
        pack(
            "document-content-organization",
            "Document／Content Organization",
            &["讀者任務、資訊階層、來源、版本", "摘要與原文引用分離"],
            &["先交付答案再交付細節", "保持術語與狀態名稱一致"],
            &["辨識讀者", "建立大綱", "整理證據", "編寫", "校對與連結檢查"],
            &["每節回答一個明確問題", "表格只用於真的需要比較"],
            &["堆疊標題但沒有結論", "複製完整來源而沒有 provenance"],
            &["文件很長不代表涵蓋 DoD"],
            &["可掃讀", "來源可追", "限制可見", "命令可重跑"],
            &["逐節核對主張、來源與最新實作"],
            &["工程文件、使用指南與驗收報告"],
            &["不能取代法律或專業編輯審查"],
        ),
        pack(
            "knowledge-research",
            "Knowledge Research",
            &[
                "Source、Evidence、Claim、Relation、Candidate、Review",
                "相似度不是因果",
            ],
            &["原始素材 write-once", "AI 抽取只進 Candidate"],
            &[
                "取得授權來源",
                "保存 hash 與片段",
                "抽取候選",
                "查衝突與反例",
                "人工複審",
            ],
            &[
                "精確引用頁碼、區域、時碼或程式位置",
                "跨域連結標明認識論類型",
            ],
            &["無證據 claim", "把 analogy 標成 causes", "覆寫原始素材"],
            &["Embedding 命中只能作候選，不能直接發布關係"],
            &["來源完整", "衝突可見", "範圍與信心明確", "版本可追"],
            &["驗 hash、Schema、來源可用性與人工 review receipt"],
            &["多模態素材與版本化 Knowledge Graph"],
            &["外部研究、成本與敏感來源必須另行同意"],
        ),
        pack(
            "learning-from-feedback",
            "Learning from Feedback",
            &[
                "Observation→Experience→Pattern→Candidate→Validated Know-how",
                "偏好不等於普遍規則",
            ],
            &["只有學習價值訊號才反思", "升格需要證據、反例與適用範圍"],
            &[
                "記錄實際結果",
                "比較原假設",
                "找根因",
                "提出候選",
                "複審",
                "驗證重用",
            ],
            &[
                "重現錯誤比單次印象更有升格價值",
                "使用者糾正先成為 User Memory",
            ],
            &["一次成功就普遍化", "Agent claim 當成功經驗", "忽略反例"],
            &["偏好深色介面不能推成所有使用者偏好深色"],
            &["證據可重現", "confidence 合理", "scope 有界", "反例存在"],
            &["在新任務中重用並以 receipt 比對實際改善"],
            &["任務回顧、使用者糾正與 Know-how 升格"],
            &["敏感個人推論預設不保存"],
        ),
    ]
}

impl Runtime {
    fn installed_domain_pack_ids(&self) -> DomainResult<BTreeSet<String>> {
        match self.store.get_meta(INSTALLED_META_KEY)? {
            None => Ok(builtin_domain_packs()
                .into_iter()
                .map(|pack| pack.id)
                .collect()),
            Some(raw) => serde_json::from_str::<BTreeSet<String>>(&raw).map_err(|error| {
                DomainError::Storage(format!("installed domain pack metadata invalid: {error}"))
            }),
        }
    }

    fn persist_installed_domain_pack_ids(&self, ids: &BTreeSet<String>) -> DomainResult<()> {
        let encoded = serde_json::to_string(ids)
            .map_err(|error| DomainError::Internal(format!("serialize domain packs: {error}")))?;
        self.store.set_meta(INSTALLED_META_KEY, &encoded)
    }

    pub fn domain_packs_list(&self) -> DomainResult<Value> {
        let installed = self.installed_domain_pack_ids()?;
        let packs = builtin_domain_packs()
            .into_iter()
            .map(|pack| {
                let is_installed = installed.contains(&pack.id);
                json!({"pack": pack, "installed": is_installed})
            })
            .collect::<Vec<_>>();
        Ok(json!({"packs": packs, "count": packs.len()}))
    }

    pub fn domain_pack_install(&self, id: &str) -> DomainResult<Value> {
        let pack = builtin_domain_packs()
            .into_iter()
            .find(|pack| pack.id == id)
            .ok_or_else(|| DomainError::NotFound(format!("domain pack {id}")))?;
        pack.validate().map_err(DomainError::Validation)?;
        let mut installed = self.installed_domain_pack_ids()?;
        installed.insert(id.to_string());
        self.persist_installed_domain_pack_ids(&installed)?;
        self.store.audit(
            "domain-pack.installed",
            "human",
            &json!({"domainPackId": id, "version": pack.version}),
        )?;
        Ok(json!({"pack": pack, "installed": true}))
    }

    pub fn domain_pack_uninstall(&self, id: &str) -> DomainResult<Value> {
        if !builtin_domain_packs().iter().any(|pack| pack.id == id) {
            return Err(DomainError::NotFound(format!("domain pack {id}")));
        }
        let mut installed = self.installed_domain_pack_ids()?;
        let removed = installed.remove(id);
        self.persist_installed_domain_pack_ids(&installed)?;
        self.store.audit(
            "domain-pack.uninstalled",
            "human",
            &json!({"domainPackId": id, "removed": removed}),
        )?;
        Ok(json!({"domainPackId": id, "installed": false, "removed": removed}))
    }

    pub(crate) fn domain_pack_context_entries(
        &self,
        domains: &[String],
    ) -> DomainResult<Vec<Value>> {
        if domains.is_empty() {
            return Ok(vec![]);
        }
        let installed = self.installed_domain_pack_ids()?;
        let entries = builtin_domain_packs()
            .into_iter()
            .filter(|pack| installed.contains(&pack.id) && domains.contains(&pack.id))
            .map(|pack| {
                json!({
                    "domainPackId": pack.id,
                    "layer": "domain-pack",
                    "kind": "know-how",
                    "title": pack.display_name,
                    "version": pack.version,
                    "content": pack,
                })
            })
            .collect();
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ten_builtin_packs_are_complete_and_versioned() {
        let packs = builtin_domain_packs();
        assert_eq!(packs.len(), 10);
        let ids = packs
            .iter()
            .map(|pack| pack.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 10);
        for pack in packs {
            pack.validate().unwrap();
            assert_eq!(pack.version, "1.0.0");
        }
    }
}
