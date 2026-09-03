// 角色預覽——依 manifest.entrypoint 分流（不用 pack id 字串判斷）：
//   builtin shu-rig → 36 表情預覽（與桌面角色同一套 rig 程式即時繪製）
//   builtin sprite  → 動作預覽（有同源 assetBase 時取真實 sheet 幀；匯入角色只列動作名）
//   builtin text    → 文字範例（含誠實對照：聲稱完成沒有綠勾）
//   其他            → 控制中心無法預覽
// 誠實對照固定：「聲稱完成」只點頭、沒有綠勾；綠勾只在「驗證成功」。

import React from "react";
import { PackManifest, validateManifest } from "../../companion/renderer";
import { drawPreviewExpression, PALETTES, previewExpressions } from "../../character/adapters/shu";
import { intentLine } from "../../character/lines";
import { rigPaletteFor } from "../../companion/gatewayWiring";
import type { CharacterCard } from "./catalog";

/** sprite 預覽狀態清單：名稱＋對應動畫＋誠實幀選擇。 */
const SPRITE_PREVIEW_STATES: { label: string; animation: string; frame?: number; note?: string }[] = [
  { label: "待機", animation: "idle" },
  { label: "察覺", animation: "notice", frame: 2 },
  { label: "聆聽", animation: "listening" },
  { label: "思考", animation: "thinking", frame: 2 },
  { label: "工作中", animation: "act", frame: 1 },
  { label: "等待", animation: "waiting", frame: 1 },
  { label: "完成（未驗證）", animation: "success", frame: 0, note: "只點頭，沒有綠勾" },
  { label: "完成（已驗證）", animation: "success", frame: 2, note: "綠勾只在驗證後" },
  { label: "結果不確定", animation: "unknown", frame: 1 },
  { label: "失敗", animation: "failed", frame: 1 },
  { label: "被阻擋", animation: "blocked", frame: 1 },
  { label: "緊急停止", animation: "emergency", frame: 0 },
];

export function CharacterPreview({ card, name }: { card: CharacterCard | null; name?: string }) {
  if (!card) {
    return <p className="muted small">角色資料尚未載入，無法預覽。</p>;
  }
  switch (card.entrypoint) {
    case "shu-rig":
      return <RigPreview card={card} />;
    case "sprite":
      return <SpritePreview card={card} />;
    case "text":
      // 文案跟著使用者取的名字（沒有解析出名字時才退回角色原名）。
      return <TextPreview name={name && name.length > 0 ? name : card.name} />;
    default:
      return (
        <div className="character-preview" data-preview="none">
          <h3>預覽</h3>
          <p className="muted small">
            這個角色由外部程式或裝置呈現；控制中心不會啟動它，也無法在這裡預覽。
          </p>
        </div>
      );
  }
}

function RigPreview({ card }: { card: CharacterCard }) {
  // 表情清單與預設配色都由 Reference Adapter 宣告；這一頁不寫死任何表情名或配色 id。
  const palette = card.manifest ? rigPaletteFor(card.manifest) : PALETTES[0].id;
  const expressions = React.useMemo(() => previewExpressions(), []);
  return (
    <div className="character-preview" data-preview="rig">
      <h3>36 表情預覽</h3>
      <p className="muted small">
        每一格由與桌面角色相同的程式即時繪出，不是靜態圖片。誠實對照：「聲稱完成」只點頭、
        沒有綠勾；綠勾與慶祝只出現在「驗證成功」。
      </p>
      <div className="preview-grid">
        {expressions.map((e) => (
          <RigPreviewCell key={e.id} exprId={e.id} label={e.label} palette={palette} />
        ))}
      </div>
    </div>
  );
}

function RigPreviewCell({ exprId, label, palette }: { exprId: string; label: string; palette: string }) {
  const ref = React.useRef<HTMLCanvasElement>(null);
  React.useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    try {
      drawPreviewExpression(ctx, exprId, palette, 96);
    } catch {
      /* 預覽畫不出來就留空格；不影響其他格 */
    }
  }, [exprId, palette]);
  const note =
    exprId === "success-claimed"
      ? "只點頭，沒有綠勾"
      : exprId === "success-verified"
        ? "綠勾只在驗證後"
        : null;
  return (
    <div className="preview-cell">
      <canvas ref={ref} width={96} height={96} aria-label={label} />
      <div className="small">{label}</div>
      {note && <div className="muted small">{note}</div>}
    </div>
  );
}

function animationNamesOf(card: CharacterCard): string[] {
  const m = card.manifest;
  if (!m) return [];
  const variants = m.capabilities["visual.expression"]?.variants;
  if (Array.isArray(variants) && variants.length > 0) return variants.slice(0, 64);
  return m.states.slice(0, 64);
}

function SpritePreview({ card }: { card: CharacterCard }) {
  const names = animationNamesOf(card);
  return (
    <div className="character-preview" data-preview="sprite">
      <h3>動作預覽</h3>
      {names.length > 0 ? (
        <ul className="character-chip-list" aria-label="動作清單">
          {names.map((n) => (
            <li key={n} className="character-chip">
              {n}
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted small">這個角色沒有宣告任何動作。</p>
      )}
      {card.assetBase ? (
        <>
          <p className="muted small">
            每張預覽都直接取自這個角色的實際圖檔——與桌面上顯示完全一致。誠實對照：「完成（未驗證）」只點頭；綠勾只出現在「已驗證」。
          </p>
          <SpriteSheetGrid packBase={card.assetBase} />
        </>
      ) : (
        <p className="muted small">匯入角色的圖檔只在桌面角色視窗載入；這裡只列出動作名稱。</p>
      )}
    </div>
  );
}

/** 從真實 pack sheet 取幀渲染預覽格（只讀同源 assetBase）。 */
function SpriteSheetGrid({ packBase }: { packBase: string }) {
  const [manifest, setManifest] = React.useState<PackManifest | null>(null);
  const [sheet, setSheet] = React.useState<HTMLImageElement | null>(null);
  const [loadError, setLoadError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let disposed = false;
    setManifest(null);
    setSheet(null);
    (async () => {
      try {
        const res = await fetch(`${packBase}/manifest.json`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const m = (await res.json()) as PackManifest;
        const issues = validateManifest(m);
        if (issues.length > 0) throw new Error(issues.join("; "));
        const img = new Image();
        img.src = `${packBase}/${m.sheet}`;
        await img.decode();
        if (!disposed) {
          setManifest(m);
          setSheet(img);
          setLoadError(null);
        }
      } catch (e) {
        if (!disposed) setLoadError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      disposed = true;
    };
  }, [packBase]);

  if (loadError) return <div className="state-box state-error">角色圖檔載入失敗：{loadError}</div>;
  if (!manifest || !sheet) return <div className="state-box">載入角色圖檔…</div>;
  return (
    <div className="preview-grid">
      {SPRITE_PREVIEW_STATES.map((s) => (
        <SpriteCell key={s.label} manifest={manifest} sheet={sheet} spec={s} />
      ))}
    </div>
  );
}

function SpriteCell({
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
      {!anim && <div className="muted small">這個角色沒有這個動作（會以安全姿態代替）</div>}
      {spec.note && <div className="muted small">{spec.note}</div>}
    </div>
  );
}

function TextPreview({ name }: { name: string }) {
  const samples = [
    { label: "打招呼", line: intentLine("greet", "none") },
    { label: "聲稱完成", line: intentLine("claim-completed", "claimed") },
    { label: "驗證成功", line: intentLine("verified-success", "verified") },
    { label: "被阻擋", line: intentLine("blocked", "none") },
  ];
  return (
    <div className="character-preview" data-preview="text">
      <h3>文字範例</h3>
      <p className="muted small">{name}以一行文字出現在桌面。安全訊息是固定文字，角色無法改寫；綠勾只在驗證後。</p>
      <ul className="plain-list character-text-samples" aria-label="文字範例">
        {samples.map((s) => (
          <li key={s.label}>
            <span className="muted small">{s.label}：</span>
            <span data-marker={s.line.marker}>
              {s.line.marker === "verified" ? "✓ " : ""}
              {s.line.text}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}
