# 本輪效能比較：採樣計畫

以下計畫在本輪效能結果產出前固定（2026-09-06）。結果與原始樣本由 evidence index 連結，
不得用歷史單次數字替代本輪基線。

- 同一台 Apple M2 Pro、相同 Node/pnpm/Playwright Chromium、相同 esbuild `es2020` IIFE，
  800×600、DPR 2；啟動旗標沿用 `scripts/shu/perf-rig.mjs`。
- 基線 source 為 `622c8bf70963e343e51f92f5ecdf7fce4b37f516`，candidate 為獨立凍結的本輪 source；
  保存完整輸入檔 digest。依賴使用同一份已安裝 lockfile 對應的 node_modules。
- 每個版本先完整暖機一次，不納入比較；之後 baseline/candidate 交錯各三次。
  每次 drawRig 72 樣本、stage 120 樣本、rAF 600 幀、各輸入操作 20 次、60 秒 soak。
  原始每次 median/p95 保存；比較三次 median 的中位數與三次 p95 的中位數，不冒稱合併分布的 p95。
- 不與編譯、整套測試或其他自動 UI 工作並行。文件閱讀與編輯不影響量測排程。
- 事先影響預算：繪製/舞台 p95 增加同時超過 20% 與 0.2 ms 才觸發調查；rAF/輸入 p95
  同時超過 20% 與 2 ms 觸發調查。任一 run 自動降到半幀、輸入確認未全數完成、無界容器超限，
  或 soak GC 後成長超過 1 MiB，均調查。這是工程調查門檻，不是統計顯著性檢定。
- 報告絕對差與相對差；基線為零時相對差不定義。60 秒 soak 不足以排除長期 leak。
  此量測是 Chromium WebView 內的角色／舞台路徑，不包含 WKWebView、OS 點擊穿透、Runtime HTTP、
  新語意 validator 或真實裝置往返，不能據此宣稱整個產品無效能退步。

真 Tauri 任務另外由相同 AX driver、clean/legacy fixture 各走一次，保存每步 script seconds、
成功 click 數與 driver 命令數。單次操作差異只作描述，不宣稱使用者完成率改善；求助、回頭、
主要決策與真人耗時沒有真人參與時一律 not-measured。
