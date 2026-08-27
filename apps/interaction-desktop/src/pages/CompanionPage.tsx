// 小樞頁（spec §16-1.C）：顯示／外觀／表現、真實狀態預覽（取自實際 sprite
// sheet，不是設計稿）、Behavior 說明、presence 誠實狀態。
// 安全語句固定不可覆寫；成功綠勾只在 verified 幀。

import React from "react";
import { api } from "../api";
import { desktop, DesktopPrefs, isTauri } from "../desktop";
import { Badge, Section } from "../ui";
import { PackManifest, validateManifest } from "../companion/renderer";

/** 預覽狀態清單（spec §C）：名稱＋對應動畫＋誠實幀選擇。 */
const PREVIEW_STATES: { label: string; animation: string; frame?: number; note?: string }[] = [
  { label: "待機 Idle", animation: "idle" },
  { label: "察覺 Notice", animation: "notice", frame: 2 },
  { label: "好奇 Curious", animation: "curious", frame: 2 },
  { label: "聆聽 Listening", animation: "listening" },
  { label: "思考 Thinking", animation: "thinking", frame: 2 },
  { label: "工作 Working", animation: "act", frame: 1 },
  { label: "等待 Waiting", animation: "waiting", frame: 1 },
  { label: "完成（未驗證）", animation: "success", frame: 0, note: "只點頭，沒有綠勾" },
  { label: "完成（已驗證）", animation: "success", frame: 2, note: "綠勾只在驗證後" },
  { label: "結果未知 Unknown", animation: "unknown", frame: 1 },
  { label: "失敗 Failed", animation: "failed", frame: 1 },
  { label: "被阻擋 Blocked", animation: "blocked", frame: 1 },
  { label: "緊急停止 Emergency", animation: "emergency", frame: 0 },
];

export function CompanionPage({ refreshKey }: { refreshKey: number }) {
  const [prefs, setPrefs] = React.useState<DesktopPrefs | null>(null);
  const [presence, setPresence] = React.useState<Record<string, unknown> | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    try {
      setPresence(await api.presentationStatus());
    } catch (e) {
      setError(String(e));
    }
    if (isTauri) {
      try {
        setPrefs(await desktop.prefsGet());
      } catch {
        /* 桌面 prefs 只在 Tauri 存在 */
      }
    }
  }, []);
  React.useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => window.clearInterval(timer);
  }, [load, refreshKey]);

  const patch = async (p: Partial<DesktopPrefs>) => {
    try {
      setPrefs(await desktop.prefsPatch(p));
      await desktop.companionApplyPrefs();
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const connected = presence?.connected === true;
  const visible = presence?.visible === true;
  const pack = String(prefs?.companionPack ?? presence?.packId ?? "shu-agile");

  return (
    <div>
      <Section title="目前狀態">
        <div className="row wrap">
          {connected ? (
            visible ? (
              <Badge kind="ok">角色視窗運作中</Badge>
            ) : (
              <Badge kind="warn">已連線但隱藏中</Badge>
            )
          ) : (
            <Badge kind="bad">角色視窗未連線</Badge>
          )}
          <span className="muted small">
            待確認呈現命令：{String(presence?.pendingCommands ?? 0)}
          </span>
        </div>
        <p className="muted small">
          隱藏角色只會停止角色視窗內的感知與呈現；Runtime、狀態列與 AI 工作階段都會繼續。
          隱藏不等於緊急停止。
        </p>
        {error && (
          <p className="cap-card-error" role="alert">
            {error}
          </p>
        )}
      </Section>

      {isTauri && prefs && (
        <Section title="顯示與外觀">
          <label className="toggle">
            <input
              type="checkbox"
              checked={prefs.companionVisible}
              onChange={(e) => void patch({ companionVisible: e.target.checked })}
            />
            <span>顯示桌面角色</span>
          </label>
          <div className="settings-grid">
            <label className="field-label">
              大小
              <select
                value={String(prefs.companionSize?.[0] ?? 200)}
                onChange={(event) => {
                  const width = Number(event.target.value);
                  void patch({ companionSize: [width, Math.round(width * 1.05)] });
                }}
              >
                <option value="160">小</option>
                <option value="200">標準</option>
                <option value="260">大</option>
                <option value="320">特大</option>
              </select>
            </label>
            <label className="field-label">
              透明度 {Math.round((prefs.companionOpacity ?? 1) * 100)}%
              <input
                type="range"
                min={20}
                max={100}
                step={5}
                value={Math.round((prefs.companionOpacity ?? 1) * 100)}
                onChange={(event) => void patch({ companionOpacity: Number(event.target.value) / 100 })}
              />
            </label>
          </div>
          <label className="field-label">
            外觀（共用同一骨架與能力；表現不同、權限相同）
            <select
              value={prefs.companionPack}
              onChange={(e) => void patch({ companionPack: e.target.value })}
            >
              <option value="shu-agile">小樞・靈巧型（預設）</option>
              <option value="shu-lazy">小樞・慵懶型</option>
              <option value="shu-lively">小樞・活潑型</option>
              <option value="shu-standard">小樞・標準型（v1 經典）</option>
              <option value="shu-minimal">小樞・極簡型（v1 經典）</option>
            </select>
          </label>
          <label className="field-label">
            表現程度（只影響表演頻率，不影響任何權限）
            <select
              value={prefs.companionExpressiveness}
              onChange={(e) => void patch({ companionExpressiveness: e.target.value })}
            >
              <option value="quiet">安靜</option>
              <option value="natural">自然</option>
              <option value="lively">活潑</option>
            </select>
          </label>
          <label className="toggle">
            <input
              type="checkbox"
              checked={prefs.companionAlwaysOnTop}
              onChange={(e) => void patch({ companionAlwaysOnTop: e.target.checked })}
            />
            <span>保持在其他視窗上方</span>
          </label>
          <button
            onClick={async () => {
              try {
                await desktop.companionResetPosition();
                setPrefs(await desktop.prefsGet());
              } catch (reason) {
                setError(String(reason));
              }
            }}
          >
            重設角色位置
          </button>
        </Section>
      )}
      {!isTauri && (
        <Section title="顯示與外觀">
          <div className="state-box">桌面角色設定需要桌面版控制中心（此為瀏覽器檢視）。</div>
        </Section>
      )}

      <BehaviorStateCard status={presence} />

      <CharacterPackDetails pack={pack} />

      <Section title="狀態預覽（取自實際角色素材）">
        <p className="muted small">
          每張預覽都直接取自目前角色包的 sprite sheet——與桌面上實際顯示完全一致。
          誠實對照：「完成（未驗證）」只點頭；綠勾只出現在「已驗證」。
        </p>
        <StatePreviewGrid pack={pack} />
      </Section>

      <Section title="行為方式（Behavior Runtime）">
        <p className="muted small">
          小樞的生命感由本機確定性系統驅動，不用生成式 AI 逐幀控制：
        </p>
        <ul className="plain-list muted small">
          <li>生命底層：呼吸、眨眼、耳動、伸展、晃腳等微動作——間隔隨機（非固定週期）、避免重複、被真實事件立即打斷。</li>
          <li>行為層：注意力優先序固定為 緊急 &gt; 感測與安全 &gt; 等待確認 &gt; 你的直接互動 &gt; 任務狀態 &gt; 建議 &gt; 世界觀 &gt; 待機。</li>
          <li>語意層：AI 只能提出白名單內的高層 behaviorIntent；成功／阻擋／緊急等「真相狀態」只能由 Runtime 事件驅動，AI 不能點播。</li>
          <li>慵懶動作（趴下、抱尾巴）只在長時間無任務、無風險時出現；有事立刻專注。</li>
          <li>Reduced Motion 開啟時只保留眨眼；勿擾／安靜時停止玩鬧。</li>
        </ul>
      </Section>
    </div>
  );
}

function BehaviorStateCard({ status }: { status: Record<string, unknown> | null }) {
  const state = (status?.behaviorState as Record<string, unknown> | null | undefined) ?? null;
  const percent = (key: string) =>
    Math.round(Math.max(0, Math.min(1, Number(state?.[key] ?? 0))) * 100);
  return (
    <Section title="現在的 Behavior State">
      {!state ? (
        <div className="state-box">
          尚未收到角色視窗的即時狀態。角色隱藏、離線或剛啟動時，系統不會用預設值冒充現況。
        </div>
      ) : (
        <>
          <p className="muted small" role="status">
            {String(status?.behaviorExplanation ?? state.explanation ?? "目前狀態原因未知。")}
          </p>
          <div className="settings-grid" aria-label="角色行為狀態數值">
            {([
              ["activation", "喚起度"],
              ["attention", "注意力"],
              ["taskLoad", "任務負載"],
              ["interactionReadiness", "互動準備度"],
              ["familiarity", "熟悉度（只影響呈現）"],
            ] as const).map(([key, label]) => (
              <div className="field-label" key={key}>
                <span>{label}：{percent(key)}%</span>
                <progress value={percent(key)} max={100} aria-label={label} />
              </div>
            ))}
          </div>
          <p className="muted small">
            基態：{String(state.base)}；目前行為：{String(state.transient ?? "自然待機")}；
            語意焦點：{String(state.currentFocus ?? "無")}；最近中斷：
            {Number(state.recentInterruptions ?? 0).toFixed(1)}。焦點只保存事件名稱，不保存原始游標軌跡。
          </p>
        </>
      )}
    </Section>
  );
}

function CharacterPackDetails({ pack }: { pack: string }) {
  const [manifest, setManifest] = React.useState<PackManifest | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    let disposed = false;
    void fetch(`/packs/${pack}/manifest.json`)
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.json() as Promise<PackManifest>;
      })
      .then((value) => {
        const issues = validateManifest(value);
        if (issues.length > 0) throw new Error(issues.join("; "));
        if (!disposed) {
          setManifest(value);
          setError(null);
        }
      })
      .catch((reason) => {
        if (!disposed) setError(String(reason));
      });
    return () => {
      disposed = true;
    };
  }, [pack]);

  return (
    <Section title="Character Pack 詳情">
      {error ? (
        <div className="state-box state-error">角色包驗證失敗：{error}</div>
      ) : !manifest ? (
        <div className="state-box">正在驗證角色包 manifest 與素材相容性…</div>
      ) : (
        <dl className="definition-list small">
          <dt>名稱／ID</dt>
          <dd>{manifest.name["zh-TW"] ?? manifest.id}（{manifest.id}）</dd>
          <dt>版本／Schema</dt>
          <dd>{manifest.version ?? "未標示"}／{manifest.schemaVersion}</dd>
          <dt>作者／授權</dt>
          <dd>{manifest.author ?? "未標示"}／{manifest.license ?? "未標示"}</dd>
          <dt>來源</dt>
          <dd>App 同源內建資產 `/packs/{manifest.id}`；產生器：{manifest.generator ?? "未標示"}</dd>
          <dt>簽章</dt>
          <dd>跟隨 Adaptive Interaction App bundle 的程式碼簽章；不接受遠端程式碼或任意動畫指令。</dd>
          <dt>相容性</dt>
          <dd>manifest、sprite sheet、frame 與動畫索引已由載入器驗證。</dd>
          <dt>更新／解除安裝</dt>
          <dd>隨 App 更新；內建安全 fallback 不可單獨解除安裝（not-applicable：避免失去安全狀態素材）。</dd>
        </dl>
      )}
    </Section>
  );
}

/** 從真實 pack sheet 取幀渲染預覽格。 */
function StatePreviewGrid({ pack }: { pack: string }) {
  const [manifest, setManifest] = React.useState<PackManifest | null>(null);
  const [sheet, setSheet] = React.useState<HTMLImageElement | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let disposed = false;
    setManifest(null);
    setSheet(null);
    (async () => {
      try {
        const res = await fetch(`/packs/${pack}/manifest.json`);
        const m = (await res.json()) as PackManifest;
        const issues = validateManifest(m);
        if (issues.length > 0) throw new Error(issues.join("; "));
        const img = new Image();
        img.src = `/packs/${pack}/${m.sheet}`;
        await img.decode();
        if (!disposed) {
          setManifest(m);
          setSheet(img);
          setLoadError(null);
        }
      } catch (e) {
        if (!disposed) setLoadError(String(e));
      }
    })();
    return () => {
      disposed = true;
    };
  }, [pack]);

  if (loadError) return <div className="state-box state-error">角色包載入失敗：{loadError}</div>;
  if (!manifest || !sheet) return <div className="state-box">載入角色素材…</div>;

  return (
    <div className="preview-grid">
      {PREVIEW_STATES.map((s) => (
        <StatePreviewCell key={s.label} manifest={manifest} sheet={sheet} spec={s} />
      ))}
    </div>
  );
}

function StatePreviewCell({
  manifest,
  sheet,
  spec,
}: {
  manifest: PackManifest;
  sheet: HTMLImageElement;
  spec: { label: string; animation: string; frame?: number; note?: string };
}) {
  const ref = React.useRef<HTMLCanvasElement>(null);
  const anim = manifest.animations[spec.animation];
  React.useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (!anim) return;
    const frameIdx = anim.frames[Math.min(spec.frame ?? 0, anim.frames.length - 1)];
    const [fw, fh] = manifest.frameSize;
    const col = frameIdx % manifest.columns;
    const row = Math.floor(frameIdx / manifest.columns);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(sheet, col * fw, row * fh, fw, fh, 0, 0, canvas.width, canvas.height);
  }, [manifest, sheet, spec, anim]);
  return (
    <div className="preview-cell">
      <canvas ref={ref} width={96} height={96} aria-label={spec.label} />
      <div className="small">{spec.label}</div>
      {!anim && <div className="muted small">此角色包沒有這個動畫（將以安全姿態代替）</div>}
      {spec.note && <div className="muted small">{spec.note}</div>}
    </div>
  );
}
