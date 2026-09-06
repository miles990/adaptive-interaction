# 受限裝置套用確認演練

從乾淨 checkout 執行：

```sh
python3 scripts/drills/restricted-device.py --evidence-dir /tmp/adaptive-restricted-device-evidence
```

需求：Rust workspace 的既有工具鏈、Python 3、Unix pty；不需要配對真板、手機、使用者 home 或額外分支。
腳本以 production `Runtime::register_declarative_spec` → `DeviceBinding` → `DeviceLink` → serial adapter
呼叫路徑測試；對端是 `scripts/esp32-serial-sim.py` **pty 模擬器**。

`state-applied-serial.template.json` 是可交给 production loader 的 DeclarativeSpec。
測試只替換隔離的 spec/device ID 和 pty 路徑；pairing code `9927` 只供此模擬器。
人類仍須選擇裝置、核對實體身分、授予所需 consent；此演練不代替那些決策。

演練分別驗證：

- `--no-frag` 的既有裝置：呈現意圖為 `intent-only`，完整狀態拒絕寫出並留稽核。
- 只宣告事件的裝置：使用既有 event-source spec/announcement，保持 `event-source` 和 input-device 角色。
- 可分片的舊裝置：完整寫出後仍沒有已套用確認。
- 選用 `aip.applied/1` 的模擬器：有界重組、consumer schema 與 hash 驗證、原子套用後回執；
  後續 patch 未確認會失去已同步，舊回執不能令新連線變成已同步；resume 可重新確認。

報告 `restricted-device.json` 記錄執行命令、時間、退出碼、HEAD、dirty 狀態、用到的模組、
必要背景與人類決策，各步完整 stdout/stderr 同存旁邊。測試自行建立暫存 Runtime、唯一 provider ID
及 pty process；每個 fixture 結束後清除自己資源。不要把 dirty tree 的證據標成某個乾淨發布 SHA。

639-byte 是 Serial/MQTT 參考 wire frame（不含換行）的 UTF-8 上限；BLE host 既有上限是 480 bytes。
測量包含 envelope extension、JSON escaping 與分片 framing。MQTT/BLE 共用 DeviceLink 契約及有界
tracker，但本演練没有其專屬 state-applied session 閉環。參考 ESP32 firmware 仍不宣告 fragmentation
或 applied profile。韌體編譯、pty/TLS 模擬器、真板及真 iPhone 證據必須分開記錄。
