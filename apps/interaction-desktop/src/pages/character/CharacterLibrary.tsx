// 更換或加入角色：內建索引＋已匯入角色的卡片（名稱、內建／第三方、可以接收哪些資料、
// 已測試），選用／停用／移除，以及匯入第三方角色（角色描述檔＋宣告的資產 → host
// `character_import`）。
//
// 一般／進階分層（v0.5 §15）：執行位置、可執行程式、需要網路、貼上原文的輸入框與
// 驗證器原文只在進階模式；一般模式改成一句人話的「額外授權」提示（誠實不隱藏事實）。
//
// 匯入只寫入本機角色資料夾；不執行程式、不連線、不下載。錯誤訊息來自驗證器／host，
// 這裡再過一次 sanitizeErrorText（不回顯路徑）。

import React from "react";
import { validateImportedManifestText } from "../../character/registry";
import { displayNameOf, type ManifestReport } from "../../character/manifest";
import type { AssetDecl, CharacterManifest } from "../../character/protocol";
import { Badge, Section } from "../../ui";
import { ConfirmButton, Dialog } from "../../components/Dialog";
import { desktop, isTauri, type DesktopPrefs } from "../../desktop";
import { exportCompanionSettings, parseCompanionSettingsImport } from "../../companion/settingsTransfer";
import {
  CHARACTER_LOCALE,
  extraPermissionLine,
  locationLabel,
  partyLabel,
  receivesLine,
  sanitizeErrorText,
  type CharacterCard,
  type CharacterCatalog,
} from "./catalog";

export function FlagBadges({
  card,
  advanced = false,
}: {
  card: Pick<CharacterCard, "origin" | "flags" | "valid">;
  /** 進階模式才顯示執行方式旗標；一般模式改用一句人話的額外授權提示。 */
  advanced?: boolean;
}) {
  return (
    <span className="character-flags">
      <Badge kind={card.origin === "builtin" ? "info" : "warn"}>{partyLabel(card.origin)}</Badge>
      {card.origin === "imported" && <Badge kind="muted">匯入</Badge>}
      {advanced && card.flags.external && <Badge kind="warn">外部</Badge>}
      {advanced && card.flags.executable && <Badge kind="bad">有可執行程式</Badge>}
      {advanced && card.flags.network && <Badge kind="warn">需要網路</Badge>}
      {!card.valid && <Badge kind="bad">資料損壞</Badge>}
    </span>
  );
}

/** 需要額外授權時的一句人話（一般模式）；不需要時什麼都不畫。 */
function ExtraPermissionNote({ card }: { card: Pick<CharacterCard, "flags"> }) {
  const line = extraPermissionLine(card);
  if (!line) return null;
  return <p className="muted small character-extra-permission">{line}</p>;
}

function CharacterCardView({
  card,
  active,
  advanced,
  busy,
  onSelect,
  onDisable,
  onRemove,
}: {
  card: CharacterCard;
  active: boolean;
  advanced: boolean;
  busy: boolean;
  onSelect: () => void;
  onDisable: () => void;
  onRemove: () => void;
}) {
  return (
    <article className={active ? "character-card active" : "character-card"} aria-label={`角色 ${card.name}`}>
      <header className="character-card-head">
        <strong>{card.name}</strong>
        {active && <Badge kind="ok">使用中</Badge>}
      </header>
      <FlagBadges card={card} advanced={advanced} />
      {card.error && <p className="cap-card-error">這個角色的資料無法讀取：{card.error}</p>}
      {!advanced && <ExtraPermissionNote card={card} />}
      <dl className="cap-facts">
        <div>
          <dt>來源</dt>
          <dd>{partyLabel(card.origin)}{card.origin === "imported" ? "（匯入）" : card.origin === "external" ? "（外部）" : ""}</dd>
        </div>
        {advanced && (
          <>
            <div>
              <dt>執行位置</dt>
              <dd>{locationLabel(card)}</dd>
            </div>
            <div>
              <dt>可執行程式</dt>
              <dd>{card.flags.executable ? "有（只記錄，不會自動執行）" : "沒有（純資料）"}</dd>
            </div>
            <div>
              <dt>需要網路</dt>
              <dd>{card.flags.network ? "是" : "否"}</dd>
            </div>
          </>
        )}
        <div>
          <dt>可以接收</dt>
          <dd>{receivesLine(card).replace(/^可以接收：/, "")}</dd>
        </div>
        <div>
          <dt>已測試</dt>
          <dd>{card.tested ? "是（隨 App 自動化測試）" : "否（未經本機測試）"}</dd>
        </div>
      </dl>
      <div className="cap-card-actions">
        {active ? (
          card.entrypoint !== "text" && (
            <button onClick={onDisable} disabled={busy} title="停用後改用純文字角色，安全訊息照常顯示">
              停用
            </button>
          )
        ) : (
          <button className="primary" onClick={onSelect} disabled={busy || !card.valid}>
            選用
          </button>
        )}
        {card.origin !== "builtin" && (
          <ConfirmButton label="移除" confirmLabel="確定移除這個角色？" onConfirm={onRemove} disabled={busy} />
        )}
      </div>
    </article>
  );
}

export function CharacterLibrarySection({
  catalog,
  activeId,
  prefs,
  busy,
  advanced = false,
  onSelect,
  onDisable,
  onRemove,
  onPatch,
  setError,
}: {
  catalog: CharacterCatalog & { reload: () => void };
  activeId: string;
  prefs: DesktopPrefs | null;
  busy: boolean;
  /** 進階模式：執行方式旗標／欄位與貼上角色描述檔原文的輸入框。 */
  advanced?: boolean;
  onSelect: (characterId: string) => Promise<void>;
  onDisable: () => Promise<void>;
  onRemove: (characterId: string) => Promise<void>;
  /** 回傳 `true` 只在 host 真的接受寫入時；失敗時呼叫端不得顯示成功文案。 */
  onPatch: (patch: Partial<DesktopPrefs>) => Promise<boolean>;
  setError: (message: string | null) => void;
}) {
  const [importOpen, setImportOpen] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);

  /**
   * 角色專屬欄位（說話風格、使魔）只用**目標角色**的 adapter 宣告驗證：這裡把
   * 「這個 characterId 在這台電腦上是哪個 adapter」告訴 settingsTransfer，
   * 不認得的角色回 null（那些欄位就會被誠實拒絕，不會靜默存成死值）。
   */
  const entrypointFor = React.useCallback(
    (characterId: string): string | null =>
      catalog.cards.find((c) => c.characterId === characterId)?.entrypoint ?? null,
    [catalog.cards]
  );

  const doExport = () => {
    if (!prefs) return;
    const data = JSON.stringify(exportCompanionSettings(prefs, { entrypointFor }), null, 2);
    const a = document.createElement("a");
    a.href = `data:application/json;charset=utf-8,${encodeURIComponent(data)}`;
    a.download = "companion-settings.json";
    a.click();
    setNotice("已匯出角色設定（不含權限、位置與歷史）。");
  };

  const doImportSettings = async (file: File) => {
    try {
      const parsed = parseCompanionSettingsImport(JSON.parse(await file.text()), {
        knownCharacterIds: catalog.knownIds,
        entrypointFor,
      });
      // 送出 ≠ 套用：host 拒絕（例如瀏覽器檢視沒有桌面 host）時不得說已套用。
      if (!(await onPatch(parsed))) {
        setError("匯入設定失敗：桌面角色設定沒有寫入成功（設定未變更）。");
        return;
      }
      setNotice("已匯入角色設定並套用。");
      setError(null);
    } catch (e) {
      setError(`匯入設定失敗：${sanitizeErrorText(e)}（設定未變更）`);
    }
  };

  return (
    <Section
      title="更換或加入角色"
      actions={
        <button onClick={() => setImportOpen(true)} disabled={busy}>
          匯入角色…
        </button>
      }
    >
      <p className="muted small">
        內建角色隨 App 提供並經自動化測試。匯入的第三方角色只存在本機角色資料夾：不會自動執行程式、
        不會自動連線。任何角色都無法改寫安全訊息或取得權限。
      </p>
      {catalog.errors.map((e) => (
        <p key={e} className="cap-card-error" role="alert">
          {e}
        </p>
      ))}
      {!catalog.loaded ? (
        <div className="state-box">正在讀取角色清單…</div>
      ) : catalog.cards.length === 0 ? (
        <div className="state-box">沒有可用的角色；安全訊息仍會以固定文字顯示。</div>
      ) : (
        <div className="character-cards">
          {catalog.cards.map((card) => (
            <CharacterCardView
              key={card.characterId}
              card={card}
              active={card.characterId === activeId}
              advanced={advanced}
              busy={busy}
              onSelect={() => void onSelect(card.characterId)}
              onDisable={() => void onDisable()}
              onRemove={() => void onRemove(card.characterId)}
            />
          ))}
        </div>
      )}
      <div className="row wrap" style={{ marginTop: 10 }}>
        <button onClick={doExport} disabled={!prefs}>
          匯出角色設定
        </button>
        <label className="button-like">
          匯入角色設定
          <input
            type="file"
            accept="application/json"
            className="visually-hidden"
            aria-label="選擇角色設定檔"
            disabled={!prefs}
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) void doImportSettings(file);
              e.target.value = "";
            }}
          />
        </label>
      </div>
      {notice && (
        <p className="muted small" role="status">
          {notice}
        </p>
      )}
      {importOpen && (
        <ImportCharacterDialog
          advanced={advanced}
          onClose={() => setImportOpen(false)}
          onImported={(_characterId, name) => {
            catalog.reload();
            setNotice(`已匯入「${name}」。選用後桌面角色視窗會改用這個角色；無法顯示時會改用文字。`);
          }}
          onSelect={onSelect}
        />
      )}
    </Section>
  );
}

// ---------------------------------------------------------------------------
// 匯入對話框
// ---------------------------------------------------------------------------

type LocalValidation =
  | { ok: true; manifest: CharacterManifest; report: ManifestReport }
  | { ok: false; errors: string[] }
  | null;

interface PickedAsset {
  base64: string;
  bytes: number;
  fileName: string;
}

/**
 * 單一資產的硬上限。
 *
 * 必須和 host 的 `MAX_ASSET_DATA_URL_BYTES`（8 MB）一致：host 讀回資產時就是用這個
 * 數字擋，匯入時放到 32 MB 只會讓使用者匯入一個永遠載不起來的角色
 * （對抗審查 character-package-019）。
 */
const MAX_ASSET_BYTES_HARD = 8 * 1024 * 1024;

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("檔案讀取失敗"));
    reader.onload = () => {
      const result = String(reader.result ?? "");
      const at = result.indexOf(",");
      resolve(at >= 0 ? result.slice(at + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}

export function ImportCharacterDialog({
  onClose,
  onImported,
  onSelect,
  advanced = false,
}: {
  onClose: () => void;
  onImported: (characterId: string, name: string) => void;
  onSelect: (characterId: string) => Promise<void>;
  /** 進階模式才有貼上原文的輸入框與驗證器原文；一般模式只有選檔＋人話摘要。 */
  advanced?: boolean;
}) {
  const [manifestText, setManifestText] = React.useState("");
  const [validation, setValidation] = React.useState<LocalValidation>(null);
  const [assets, setAssets] = React.useState<Record<string, PickedAsset>>({});
  const [assetErrors, setAssetErrors] = React.useState<Record<string, string>>({});
  const [busy, setBusy] = React.useState(false);
  const [hostError, setHostError] = React.useState<string | null>(null);
  const [done, setDone] = React.useState<{ characterId: string; name: string } | null>(null);

  const applyText = (text: string) => {
    setManifestText(text);
    setHostError(null);
    setDone(null);
    setAssets({});
    setAssetErrors({});
    if (text.trim().length === 0) {
      setValidation(null);
      return;
    }
    const r = validateImportedManifestText(text);
    setValidation(r.ok ? { ok: true, manifest: r.manifest, report: r.report } : { ok: false, errors: r.errors });
  };

  const pickAsset = async (decl: AssetDecl, file: File | undefined) => {
    if (!file) return;
    const limit = Math.min(validation?.ok ? validation.manifest.resourceLimits.maxAssetBytes : MAX_ASSET_BYTES_HARD, MAX_ASSET_BYTES_HARD);
    if (file.size === 0 || file.size > limit) {
      setAssetErrors((prev) => ({
        ...prev,
        [decl.id]: file.size === 0 ? "檔案是空的" : "檔案超過可匯入的大小上限（單一資產最多 8 MB）",
      }));
      return;
    }
    try {
      const base64 = await readFileAsBase64(file);
      setAssets((prev) => ({ ...prev, [decl.id]: { base64, bytes: file.size, fileName: file.name } }));
      setAssetErrors((prev) => {
        const next = { ...prev };
        delete next[decl.id];
        return next;
      });
    } catch (e) {
      setAssetErrors((prev) => ({ ...prev, [decl.id]: sanitizeErrorText(e) }));
    }
  };

  const declared = validation?.ok ? validation.manifest.assets : [];
  const missing = declared.filter((a) => !assets[a.id]);
  const canSubmit = isTauri && validation?.ok === true && missing.length === 0 && !busy && !done;

  const submit = async () => {
    if (!validation?.ok) return;
    setBusy(true);
    setHostError(null);
    try {
      const result = await desktop.characterImport({
        manifestText,
        assets: declared.map((a) => ({ id: a.id, base64: assets[a.id]?.base64 ?? "" })),
      });
      const name = displayNameOf({ displayName: result.displayName ?? validation.manifest.displayName }, CHARACTER_LOCALE);
      setDone({ characterId: result.characterId, name });
      onImported(result.characterId, name);
    } catch (e) {
      setHostError(sanitizeErrorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog title="匯入角色" onClose={onClose}>
      <div className="character-import">
        {!isTauri && (
          <div className="notice-box">
            匯入需要桌面版控制中心（此為瀏覽器檢視）。你仍可以在這裡檢查角色檔是否合法。
          </div>
        )}
        <p className="muted small">
          {advanced ? "貼上或選擇角色描述檔" : "選擇角色描述檔"}，再附上它宣告的圖檔等資產。匯入只會存到本機角色資料夾：
          不會執行任何程式、不會連線、不會下載。需要執行程式或需要網路的角色會被明確標示。
        </p>
        <label className="button-like">
          選擇角色描述檔
          <input
            type="file"
            accept="application/json,.json"
            className="visually-hidden"
            aria-label="選擇角色描述檔"
            onChange={async (e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (!file) return;
              if (file.size > 256 * 1024) {
                setValidation({ ok: false, errors: ["角色描述檔超過 256 KB"] });
                return;
              }
              applyText(await file.text());
            }}
          />
        </label>
        {advanced && (
          <label className="field-label">
            或直接貼上角色描述檔內容
            <textarea
              className="editor small-editor"
              aria-label="角色描述檔內容"
              value={manifestText}
              spellCheck={false}
              onChange={(e) => applyText(e.target.value)}
            />
          </label>
        )}
        {validation && !validation.ok && (
          <div className="state-box state-error" role="alert">
            <div>
              角色描述檔不符合規格{advanced ? "：" : `（${validation.errors.length} 個問題）；請向角色作者索取正確的檔案。`}
            </div>
            {advanced ? (
              <ul className="plain-list small">
                {validation.errors.slice(0, 12).map((err) => (
                  <li key={err}>{sanitizeErrorText(err)}</li>
                ))}
              </ul>
            ) : (
              <details className="tech-details">
                <summary>問題明細</summary>
                <ul className="plain-list small">
                  {validation.errors.slice(0, 12).map((err) => (
                    <li key={err}>{sanitizeErrorText(err)}</li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
        {validation?.ok && (
          <div className="character-import-preview">
            <strong>{displayNameOf(validation.manifest, CHARACTER_LOCALE)}</strong>{" "}
            <FlagBadges
              advanced={advanced}
              card={{
                origin: validation.report.flags.external ? "external" : "imported",
                flags: {
                  external: validation.report.flags.external,
                  executable: validation.report.flags.executable,
                  network: validation.report.flags.network,
                },
                valid: true,
              }}
            />
            {!advanced && (
              <ExtraPermissionNote
                card={{
                  flags: {
                    external: validation.report.flags.external,
                    executable: validation.report.flags.executable,
                    network: validation.report.flags.network,
                  },
                }}
              />
            )}
            {validation.report.warnings.length > 0 &&
              (advanced ? (
                <ul className="plain-list muted small">
                  {validation.report.warnings.slice(0, 8).map((w) => (
                    <li key={w}>提醒：{sanitizeErrorText(w)}</li>
                  ))}
                </ul>
              ) : (
                <details className="tech-details">
                  <summary>這個角色檔有 {validation.report.warnings.length} 個提醒</summary>
                  <ul className="plain-list muted small">
                    {validation.report.warnings.slice(0, 8).map((w) => (
                      <li key={w}>提醒：{sanitizeErrorText(w)}</li>
                    ))}
                  </ul>
                </details>
              ))}
            {declared.length > 0 ? (
              <div className="character-import-assets">
                <div className="small">這個角色宣告了 {declared.length} 個資產，請逐一附上：</div>
                {declared.map((decl) => (
                  <div className="row wrap" key={decl.id}>
                    <label className="button-like">
                      {assets[decl.id] ? `已選：${assets[decl.id].fileName}` : `附上「${decl.id}」`}
                      <input
                        type="file"
                        className="visually-hidden"
                        aria-label={`附上資產 ${decl.id}`}
                        accept={decl.mediaType ?? undefined}
                        onChange={(e) => {
                          const file = e.target.files?.[0];
                          e.target.value = "";
                          void pickAsset(decl, file);
                        }}
                      />
                    </label>
                    <span className="muted small">{decl.mediaType ?? "類型未標示"}</span>
                    {assetErrors[decl.id] && <span className="cap-card-error">{assetErrors[decl.id]}</span>}
                  </div>
                ))}
              </div>
            ) : (
              <p className="muted small">這個角色沒有宣告任何資產。</p>
            )}
          </div>
        )}
        {hostError && (
          <p className="cap-card-error" role="alert">
            匯入失敗：{hostError}
          </p>
        )}
        {done && (
          <p className="muted small" role="status">
            已匯入「{done.name}」。
          </p>
        )}
        <div className="row wrap" style={{ marginTop: 12 }}>
          {done ? (
            <>
              <button
                className="primary"
                onClick={async () => {
                  await onSelect(done.characterId);
                  onClose();
                }}
              >
                選用「{done.name}」
              </button>
              <button onClick={onClose}>關閉</button>
            </>
          ) : (
            <>
              <button className="primary" onClick={() => void submit()} disabled={!canSubmit}>
                {busy ? "匯入中…" : "匯入"}
              </button>
              <button onClick={onClose}>取消</button>
            </>
          )}
        </div>
      </div>
    </Dialog>
  );
}
