# Adaptive Interaction v0.5：核心體驗重構完整實作題詞

你現在要接手並實際修改以下最新專案：

- Repository：`https://github.com/miles990/adaptive-interaction/`
- 先同步並確認最新 `main`、版本、commit、dirty worktree、既有測試與文件。
- 保留使用者現有修改；不得以 reset、checkout 或覆蓋方式破壞未提交內容。
- 不要只輸出分析、規格、UI mockup、範例程式碼或待辦清單。必須檢查實際程式碼、先跑回歸、分階段實作、驗證、更新文件，並提供機器證據。
- 不可用 Mock UI、假裝置、硬編 success payload 或文件敘述冒充真功能。模擬器必須明確標示為模擬器，真實硬體能力必須由實際 Adapter、事件與回報證明。

---

## 一、產品重新定位

這個專案最重要的三個核心是：

1. **連接與實際操作硬體裝置。**
2. **角色與玩家互動的呈現、生命感與遊戲感。**
3. **角色與 AI Agent 的連接、對話、任務委派及結果呈現。**

目前專案的 Runtime、Governor、Consent、Receipt、Lease、Token、Emergency Stop、記憶與知識治理已經相當完整，但產品重心失衡：安全與治理層比角色遊戲性、真實硬體 Adapter 和 AI 角色體驗成熟。

本輪不得繼續擴張新的治理概念。安全功能保留為底層護欄，但不得成為一般 UI 的主角，也不得讓低風險角色動畫因 Consent、Receipt 或 AI 往返而失去即時性。

新的產品定義：

> Adaptive Interaction 是一個讓桌面角色能感知玩家與裝置、以具有生命感的方式呈現狀態，並透過 AI Agent 完成真實工作的互動 Runtime。

新的優先順序：

```text
角色生命感與遊戲互動
> 真實硬體閉環
> AI Agent 工作與對話閉環
> 一般人可理解的設定
> 必要安全底線
> 記憶與知識
> 高階治理與技術詳情
```

---

## 二、不得破壞的安全底線

保留並回歸驗證：

- Emergency Stop。
- 麥克風、攝影機使用中的持續可見指示與立即停止。
- Human Token、Agent Token、Session Scope 分離。
- Agent 不得自行授權、解除 Emergency Stop 或擴大資料範圍。
- 指定工作區寫入、外部資料傳送與實體效果的明確授權。
- Agent claimed-completed 不等於 verified。
- 外部動作結果未知不得顯示成功，也不得自動重試可能造成重複副作用的動作。
- 硬體強度、時間、頻率與韌體硬限制。
- Secret 使用 Keychain／Credential Reference，不寫入 YAML 或 log。
- Session 可取消、到期、撤銷；子程序樹可終止。

但安全 UI 必須改成風險分級：

| 等級 | 例子 | 預設處理 |
|---|---|---|
| L0 純角色呈現 | 眨眼、耳朵、尾巴、姿勢、散步、玩耍、氣泡 | 預設開啟，不逐次詢問、不產生干擾性 Receipt UI |
| L1 本機低風險 | 本機通知、短音效、角色移動 | 一次設定，可隨時關閉 |
| L2 個人資料 | 指定檔案、偏好記憶、Context Bundle | 首次或範圍改變時詢問 |
| L3 外部或實體效果 | 燈光、震動、裝置命令、外部傳送 | 明確授權、強度與時間硬限制 |
| L4 高敏感／高風險 | 攝影機、持續麥克風、定位、Agent 寫入 | 每次或短效授權，持續指示 |

底層仍可保留 Audit，但一般使用者不應被 Provider lifecycle、Lease、Token、Candidate、Receipt 等技術術語淹沒。

---

## 三、先建立三個不可妥協的產品閉環

### 3.1 玩家與角色

```text
玩家靠近／點擊／拖曳／投擲玩具／輸入文字
→ 本機即時感知
→ Interaction Director 選擇注意力與反應
→ 小樞以眼睛、耳朵、尾巴、身體、表情、氣泡及音效組合呈現
→ 必要時才呼叫 AI
→ 自然回到先前狀態
```

目標：不連 AI、不連硬體時，小樞本身仍然可愛、好玩、像在生活，而不是停止不動的 Runtime 狀態圖示。

### 3.2 角色與 AI Agent

```text
玩家向小樞提出任務
→ 顯示交給 Codex 或 Claude Code、資料範圍、workdir、工具與取消方式
→ 建立真 Agent Session
→ 小樞依真實事件呈現 queued／fetched／working／waiting-for-consent／claimed-completed／verified／failed／unknown
→ 只有獨立驗證後才播放 verified 成功演出
→ 結果以角色可理解的方式交付，同時保留技術詳情入口
```

### 3.3 角色與硬體

```text
真實裝置被發現
→ 配對與能力識別
→ 人類選擇允許的感知與動作
→ 小樞或玩家觸發實際命令
→ Adapter 執行
→ 裝置回報／獨立 Observation
→ 小樞依真實結果呈現成功、失敗或未知
```

至少交付一套可重現的 ESP32 參考硬體閉環，不能只停在 metadata scan。

---

## 四、小樞正式角色重製

### 4.1 視覺方向

小樞正式版本以「菲比啾比 Q 版表情包」所帶來的圓潤比例、柔軟變形、誇張表情、高情緒辨識力、貼圖式瞬間笑點與讓人想戳弄的可愛感作為私人非商用參考基準。

不要只把現有工程角色換色。重新設計為具有以下特徵的 Q 版貓娘女僕數位精靈：

- 約 2.5～2.7 頭身。
- 大頭、小身體、低重心、圓潤輪廓。
- 女性化但沒有成熟成人感，不強調胸、腰、臀。
- 聰明、俏皮、機靈、可愛，有時慵懶。
- 柔黑或深灰紫短髮，帶不對稱髮束。
- 大而機靈、眼尾微揚的紫灰色眼睛。
- 得意時偶爾露出一顆小虎牙。
- 一對會真實參與表演的貓耳；不另加人類耳朵。
- 左耳為冷藍「感知耳」，右耳為暖橘「行動耳」。
- 長而柔軟、能高度表意並參與操作的貓尾。
- 胸前具有呼吸般發光的小型結晶／蝴蝶結核心。

### 4.2 女僕裝

服裝是可愛的遊戲式女僕工作服，不是性感女僕裝：

- 奶白泡泡袖、完整圓領或小立領。
- 深灰紫短外衣或小披肩。
- 小型分體女僕頭飾，中央為貓耳保留活動空間。
- 胸前蝴蝶結與發光核心整合。
- 蓬鬆多層裙擺，內搭不透明燈籠安全短褲。
- 工具圍裙與兩側口袋。
- 稍大的袖口、手套與圓頭軟底短靴。
- 奶白、深灰紫為主色，冷藍／暖橘為能力訊號色，低飽和粉紫作肉球細節。
- 不使用胸口開洞、吊帶襪、貼身曲線、過短裙擺或成熟誘惑姿勢。

服裝必須參與功能呈現：

| 部位 | 呈現用途 |
|---|---|
| 左耳 | 受器／感知活動 |
| 右耳 | 動器／行動活動 |
| 胸前核心 | Runtime、AI 思考與工作狀態 |
| 頭飾 | 網路與 Agent 連線狀態；奔跑後可歪掉再扶正 |
| 圍裙口袋 | 取出玩具、檔案、知識卡片與工具 |
| 袖口 | 展開小型工作面板 |
| 尾巴 | 指向、拖取物件、表達情緒 |
| 裙擺細光 | waiting、unknown、blocked 等輔助狀態 |

### 4.3 個性不只存在於對話

個性必須影響 Attention、Utility score、動作選擇、速度、距離、表情與恢復方式：

- 聰明：先動耳朵、再移動視線、最後才轉頭；動作有效率。
- 機靈：事件剛出現便提前接住或避開。
- 俏皮：偶爾假裝沒看到、藏起物件、從另一側探頭。
- 慵懶：趴著操作、用尾巴拖工作、慢半拍起身。
- 得意：半瞇眼、抬下巴、尾巴豎起，等待稱讚卻嘴硬。
- 好奇：歪頭、靠近、瞳孔放大、耳朵朝向目標。
- 被抓包：瞬間定格、移開視線、假裝整理袖口或工具。
- 失敗：不崩潰、不責怪玩家，先確認現場再提出下一步。

她不是服從型女僕，不要持續鞠躬、敬禮或稱呼主人。她是自己喜歡穿女僕工作服的數位精靈。

---

## 五、完整追平並超越 VS Code Pets

以下功能全部要有，不能只做視覺 Mock：

### 5.1 VS Code Pets 基準能力

- 自主散步、奔跑、停下、坐下、趴下、休息與睡覺。
- 對滑鼠游標靠近、進入、離開、點擊、連點產生反應。
- Hover／靠近時可顯示短氣泡，但不能每次都打擾。
- 可投擲玩具；角色會預備、追逐、撲抓、撿回或拒絕歸還。
- 可用滑鼠決定投擲方向、速度與落點。
- 支援多角色／小型使魔的架構。
- 多角色能互相注意、打招呼、出現愛心、追逐及玩耍。
- 可選角色外觀、顏色與名稱。
- 可個別顯示、隱藏或移除角色。
- 可匯入／匯出角色設定與互動偏好。
- 可切換桌面巢穴、工作桌、窗台、夜間等背景／場景概念；透明桌面模式仍須正常。
- 提供類似 Roll Call 的「現在大家在做什麼」，但使用人類語言。

### 5.2 超越基準的桌面互動

- 玩家拖曳小樞時有被抱起、懸空、掙扎或好奇反應。
- 放下時依速度、高度與位置選擇站穩、踉蹌、滑倒或輕巧落地。
- 小樞可以坐在視窗邊緣、從螢幕邊緣探頭、躲到視窗後再出現。
- 對視窗開啟、關閉、移動、下載完成、測試失敗、任務完成做語意反應。
- 能接住拖入檔案，先顯示檔名、大小、類型、資料去向與可讀 Agent，再確認。
- Agent 工作時可抱著檔案、閱讀、書寫、翻找、等待、戳進度條。
- 硬體上線、離線、執行動作時有對應表演。
- 長時間無操作時自然進入休息；玩家回來時注意到，但不必每次說話。
- 每個高頻反應至少 3～6 個變體，具防重複與冷卻。
- 玩耍、主動靠近、氣泡、音效、追逐游標、桌面移動均可分別關閉。

### 5.3 第一批玩具

- 毛球。
- 紙團。
- 光點。
- 逗貓棒。
- 小紙飛機。
- 可拖曳的小物件。

玩具資料模型至少包含位置、速度、重力、碰撞、抓取狀態、擁有者、角色興趣值、冷卻與生命週期。使用輕量 2D 物理，不要為此引入重量級 3D 遊戲引擎。

---

## 六、Interaction Director 與動畫系統

新增統一 Interaction Director，不准事件直接任意切換完整動畫：

```text
Receptors／玩家／硬體／Agent 事件
→ Event Normalizer
→ Attention Manager
→ Character Context
→ Utility Scoring
→ Behavior Intent
→ Action Scheduler
→ Animation／Gaze／Ear／Tail／Bubble／Audio Mixer
→ Presentation Ack
```

### 6.1 必要能力

- 多事件注意力競爭與優先度。
- 高風險安全事件可搶占任何表演。
- 動作可中斷、可恢復、可取消。
- 進入、保持、小循環、離開四段式狀態。
- 前一動作到下一動作的自然 Transition。
- 同一意圖的變體選擇與近期防重複。
- 動作冷卻、頻率及最大連續時間。
- AI 延遲時的自然等待與降級表現。
- Reduced Motion、Quiet Hours、Fullscreen、勿擾模式。
- 角色被隱藏時停止 Presentation receptors，但 Runtime、Tray、Agent 保持正確狀態。

### 6.2 組合式角色通道

角色動器不要只剩「播放動畫」。至少拆成：

- Body pose／locomotion。
- Head pose。
- Gaze target。
- Eyes／brows／mouth expression。
- Cat ears。
- Tail。
- Hair／sleeves／headpiece secondary motion。
- Hand／held prop。
- Bubble／text。
- Audio／voice／purr／SFX。
- Position／desktop anchor。
- Particles／status effect。

允許組合，例如：

```text
趴著＋看向玩家＋左耳注意＋尾尖輕擺＋胸前核心顯示 Agent 工作中
```

### 6.3 Game Feel

重要動作須包含：

- Anticipation。
- Squash and Stretch。
- Overshoot。
- Follow-through。
- Secondary motion。
- 1～2 幀適量 hit-stop。
- 細小粒子、灰塵、睏意與撞擊效果。
- 音效變體。
- 動作取消與恢復。

不要過度晃動 UI；Game Feel 主要作用在角色與玩具。

### 6.4 動畫技術

從單純固定 Sprite Sheet 升級為混合管線：

- 分層 2D 骨架／mesh deformation 處理身體、頭、耳、尾、頭髮、袖口與配件。
- 臉部使用可替換表情圖層與參數化眼睛／眉毛／嘴形。
- 撲抓、滑倒、壓扁、驚訝等誇張動作使用逐幀手繪或變形 Sprite。
- 渲染層可採 Canvas／WebGL；需評估現有 Tauri WebView、包體、效能、跨平台及授權。
- 不得為了技術方便犧牲 390px、Reduced Motion 或透明視窗效能。
- 保留舊 Character Pack 相容層與 fallback，但新版格式須支援分層骨架、表情、通道、變體與行為 metadata。

---

## 七、正式動畫與表情清單

### 7.1 基礎生活

- 站立呼吸、坐下、趴下、打瞌睡、熟睡、驚醒。
- 伸懶腰、整理貓耳／頭髮／頭飾／裙擺。
- 左右張望、放空、偷懶被發現、從邊緣探頭。

### 7.2 移動

- 走路、小跑、奔跑、急停、轉身、跳起、落地、滑倒、攀爬。
- 被拖曳、懸空、放下、重新站穩。

### 7.3 玩家互動

- 被點擊、連點、看向游標、靠近、躲開、伸手擋住游標。
- 追逐光點、撲毛球、撲空、抱住、帶回、不想歸還。
- 接住拖入檔案、拒絕未確認附件。

### 7.4 AI 與工作

- 閱讀、思考、快速書寫、操作工具、翻找資料。
- 等待 Codex、等待 Claude、等待玩家確認。
- queued、fetched、working、waiting、blocked、claimed-completed、verified、failed、unknown、cancelled。
- claimed-completed 只能呈現「對方說完成了」；verified 才能出現綠色勾勾與正式成功演出。

### 7.5 首批 36 表情

包含：疑問、偷看、歪頭、探頭、無語、放空、哈欠、趴平、伸懶腰、被吵醒、假裝沒聽見、悄悄靠近、被點、被連戳、被拖起、落地站不穩、抱球、不還球、撲空、滑倒裝沒事、被稱讚、偷懶被抓、等玩家、玩家回來、思考、找資料、努力工作、等 Codex、等 Claude、需要確認、權限不足、找不到、結果未知、聲稱完成、驗證成功、工作失敗。

每個表情定義進入、保持、小循環、離開；不得只做一張靜態圖片。

---

## 八、本機反應、對話 AI 與工作 Agent 分層

### 8.1 L0 本機即時層

目標反應時間 16～100ms，不呼叫 AI：

- 游標、點擊、拖曳、玩具與物理。
- 眼睛、耳朵、尾巴、眨眼、姿勢及短氣泡。
- Runtime、裝置與 Agent 狀態映射。
- 動作變體、冷卻、注意力與玩耍。

### 8.2 L1 短語意互動層

建立可插拔 Conversation Provider 介面，但本輪不要求直接接模型 API。負責：

- 簡短輸入理解。
- 決定是否回話。
- 主動問候與一句短回應。
- 根據近期情境選語氣與 behaviorIntent。
- 判斷是否建議建立 Codex／Claude 任務。

如果沒有合適的對話 Provider，必須自然降級為本機規則與有限模板，不能為普通反應啟動昂貴工作 Agent。

### 8.3 L2 工作 Agent 層

沿用並強化真實 Codex app-server／exec fallback 與 Claude Code stream-json：

- Discovery、版本、登入狀態。
- 建立、續租、取消、interrupt、close、resume。
- workdir、read/write scope、tool scope、資料範圍、成本、時間與取消方式預覽。
- Mailbox 真實 fetched／working 狀態。
- Approval 對應人類 UI。
- 將 Agent 事件標準化成 Character Behavior Intent。
- Agent claim 永不冒充 Verified Receipt。

角色可以利用 Agent 做事，但角色本身不持有不受限權限。

---

## 九、真實硬體連接

### 9.1 優先 Adapter

在現有 HTTP／SSE 基礎上，依序完成：

1. USB Serial。
2. Bluetooth LE。
3. MQTT。
4. WebSocket。
5. HID（平台允許範圍）。
6. Home Assistant bridge。
7. ESP32／Arduino Reference Adapter。

每個 Adapter 必須有：

- Discovery。
- Stable identity 或誠實回報無法穩定識別。
- Pairing／verification。
- Capability Manifest。
- Read／write schema。
- Timeout、cancel、reconnect、backoff。
- Idempotency／replay protection，適用時加入 nonce。
- 硬體硬限制與 Runtime 限制。
- Acknowledged、Observed、Verified 的誠實區分。
- 模擬器與真硬體測試分開標示。

### 9.2 官方 ESP32 參考裝置

提供可實際製作的參考韌體、接線圖、BOM、Flash 步驟與測試：

- RGB LED。
- 按鈕。
- 距離感測器。
- 環境光。
- 溫度感測器。
- 震動馬達。
- 小型伺服馬達。
- 蜂鳴器／小型揚聲器。

至少支援 BLE 與 Wi-Fi/MQTT 其中兩種連線。韌體強制限制震動、伺服與聲音的強度、持續時間及頻率。

### 9.3 一般使用者 UI

不要先顯示 receptor／actuator。以裝置為中心：

```text
書桌互動裝置

小樞可以知道：
✓ 有人靠近
✓ 房間亮度
✓ 裝置是否在線

小樞可以做：
✓ 改變燈光
✓ 播放提示音
○ 震動（每次先詢問）
```

掃描到 metadata 不等於連線完成。UI 必須清楚顯示「只發現」、「已配對」、「已測試」、「已啟用」的差異。

---

## 十、iPhone Mobile Provider

新增原生 iOS Companion App，將 iPhone 視為 Mobile Provider，而不是假設桌面可直接任意讀取 iPhone。

### 10.1 連線架構

```text
iPhone App
├── iPhone 本身的 receptors／actuators
├── iPhone 連接的 BLE／網路裝置
└── Bonjour discovery＋QR pairing＋TLS WebSocket
        ↓
Desktop Adaptive Interaction Runtime
        ↓
小樞／AI Agent／自動互動
```

要求：

- Swift／SwiftUI 原生 App；可共用 Rust domain schema，但不要為共用而犧牲 iOS 權限與生命週期正確性。
- Bonjour 自動發現。
- QR Code 或配對碼。
- 每台 iPhone 獨立金鑰與 challenge-response。
- TLS WebSocket。
- iPhone 清楚顯示連接的電腦、能力、活動中感測器與立即中斷。
- 斷線後能力自動 unavailable；重連不得自動恢復高風險能力。
- 桌面 Consent 不能取代 iOS 系統權限。

### 10.2 iPhone receptors

- Touch／gesture。
- Accelerometer。
- Gyroscope／device motion／orientation。
- Battery／charging／foreground state。
- Microphone level／audio，需權限。
- Camera／QR／capture，需權限。
- Location／geofence，需權限。
- BLE device discovery／state。
- Local-network device events。

不可用的感測器要依機型與系統 API 誠實標示 unavailable，不得假設所有 iPhone 都有相同能力。

### 10.3 iPhone actuators

- Character presentation。
- Custom haptic。
- Notification。
- Audio／SFX。
- TTS。
- Screen color／flash effect。
- Torch，需明確用途與限制。
- Live Activity／鎖定畫面狀態，平台允許時。

### 10.4 iPhone 作為硬體閘道

- 第一優先支援 BLE GATT 掃描、連線、Service／Characteristic 探索、read、write、subscribe。
- 第二優先支援 Bonjour、HTTP、WebSocket、MQTT 區域網路裝置。
- External Accessory 僅用於明確支援的 MFi／廠商配件。
- 不得宣稱 iPhone 可任意操作所有 USB／Lightning／USB-C 裝置。
- ESP32 的 iPhone 連線優先 BLE 或 Wi-Fi，不以通用 USB Serial 為第一版方案。

### 10.5 跨載體角色體驗

- 小樞可從桌面「前往 iPhone」，但必須對應真實 connected presentation surface。
- iPhone 被拿起時，可在授權與設定允許下觸發桌面小樞注意。
- 桌面任務進行時，iPhone 可顯示簡化角色狀態與必要確認。
- iPhone haptic 可表現輕敲、呼嚕、心跳或提醒，但可分別關閉並限制頻率。
- 不保存原始 motion 軌跡；優先輸出 lifted、shaken、placed、rotated 等語意事件。

---

## 十一、記憶與知識重新分層

保留現有 10 層記憶與知識圖譜後端，但一般 UI 簡化。

分開三種資料：

1. **角色互動記憶**：最喜歡的玩具、玩家偏好距離、常被關掉的反應、近期玩耍、互動熟悉度。
2. **工作與個人記憶**：使用者明確偏好、任務脈絡、Agent Context Bundle。
3. **正式知識**：來源、領域、證據、期限、衝突與更新。

角色互動記憶不得因一次行為就推論人格，也不得自動升級為跨領域正式知識。

一般 UI 只顯示：

- 關於我的記憶。
- 小樞學會的知識。
- 素材與來源。

Candidate、Active、Stale、Disputed、Superseded、Knowledge Receipt、Context Bundle 等移至技術詳情／進階模式，並使用人類文案：等待確認、已採用、可能過期、有不同說法、已被新版取代。

---

## 十二、控制中心資訊架構簡化

目前 9 個一般一級頁面過多；進階模式加上技術頁後側欄更長。重構為 5 個主要入口：

### 12.1 現在

> **2026-09-03 落地標籤（不改規格，僅記錄最終命名）**：一級導覽最終為「現在／角色（顯示目前角色的名字，
> 預設「小樞」）／工作／連接與權限／更多」——本節與 §12.2 草案寫的「小樞」在實作中改成跟著目前角色名字走的
> 「角色」入口（因為角色不再是寫死的小樞，見 Character Presentation Protocol）。細節與程式碼對照見
> `docs/DESKTOP-GUIDE.md` 與 `docs/v05-capability-gap-matrix.md` §10–§11。

- 系統是否正常。
- 是否有感測器運作。
- 有什麼需要玩家決定。
- 小樞／Agent／硬體現在正在做什麼。
- 最近一次已驗證結果。
- 失敗、未知或離線例外。

不要在首頁重複完整權限地圖、全部 Session、全部歷史與完整設定。

### 12.2 小樞

- 外觀、女僕裝、顏色、名字。
- 安靜／自然／活潑。
- 玩耍、游標、靠近、氣泡、音效、拖曳、桌面移動。
- 主動對話模式與頻率。
- 安靜時段。
- 表情與動作預覽。
- 角色匯入／匯出。

### 12.3 工作

- Codex／Claude Code Provider。
- 工作階段與任務。
- 自動互動。
- waiting／approval／cancel／resume。
- 成本、時間與資料範圍。

### 12.4 連接與權限

- iPhone。
- ESP32／Arduino。
- BLE／Serial／MQTT／網路裝置。
- 感測器與動作能力。
- Consent、資料去向、測試與立即停止。

### 12.5 更多

> **2026-09-03 落地標籤（不改規格，僅記錄最終命名）**：「更多」最終的五個分頁標籤為
> 「記憶與資料／活動紀錄／外觀與語言／備份與還原／進階模式」；「進階模式」是「顯示進階功能」開關的唯一主人，
> 打開後才展開版本與 Runtime、Provider 診斷、配方 YAML、政策原始設定等第二層。角色管理併入「角色」頁而非
> 獨立分頁。`manage`（舊「角色與整合管理」）保留為隱藏相容路由（全域搜尋可達，無分頁按鈕）。

- 記憶與知識。
- Activity 歷史。
- 一般設定。
- 備份與更新。
- 進階功能。

Activity／Confirm 預設作為右上角 Inbox，不佔一級頁面。Emergency Stop 固定保留在頂部、Tray、角色快捷選單與 CLI。

### 12.6 每項設定只有一個主人

- 小樞如何表現 → 小樞。
- AI 做什麼工作 → 工作。
- 何時觸發流程 → 工作／自動互動。
- 可以讀取或操作什麼 → 連接與權限。
- 資料如何保存 → 記憶與知識。
- 語言、視窗、啟動、備份、版本 → 更多／設定。

其他頁面只能顯示摘要與「前往設定」，不可再放第二份相同開關。

---

## 十三、首次設定精靈

由七步縮成三個主要步驟，其他項目採漸進式詢問：

### 步驟一：認識小樞

- 顯示／不顯示。
- 預覽正式 Q 版貓娘女僕角色。
- 安靜／自然／活潑，預設自然。
- 玩耍與游標互動預設開啟。
- 音效預設關閉或低音量。

### 步驟二：要讓小樞幫忙工作嗎？

- 連接 Codex。
- 連接 Claude Code。
- 兩者都連接。
- 稍後再說。

只做 Discovery／登入狀態檢查，不自動授權工作區寫入。

### 步驟三：安全預設

- 麥克風、攝影機、位置預設關閉。
- 外部裝置與實體動作首次使用時詢問。
- Agent 寫入每個 workdir 明確確認。
- 主動對話預設「必要時」。
- 顯示如何暫停與 Emergency Stop。

硬體掃描、iPhone 配對、安靜時段、進階權限、知識更新等在第一次真正需要時再詢問；精靈提供「進一步自訂」，但不強迫完成。

---

## 十四、效能與誠實性要求

- 本機游標／玩具反應目標 16～100ms。
- 一般動畫不得因 HTTP、SQLite、Agent 或 AI 阻塞。
- Interaction Director 與 renderer 使用 bounded queues。
- 動畫與事件在 60fps 目標下量測；低效能裝置允許 30fps 降級。
- Reduced Motion 下保留狀態辨識但減少位移、彈跳與粒子。
- 原始游標軌跡不持久化、不傳 AI。
- 原始 iPhone motion 軌跡預設不持久化，轉為語意事件。
- 麥克風、攝影機與定位不得靜默啟用。
- 所有 unavailable、unsupported、claimed、acknowledged、unknown 狀態使用誠實文字。
- 不可為了畫面漂亮，把 Unknown、Blocked、Emergency 演成成功或賣萌慶祝。

---

## 十五、測試要求

### 15.1 全部既有回歸

- Rust fmt。
- Clippy `-D warnings`。
- Workspace tests。
- Desktop Tauri tests。
- Frontend typecheck、Vitest、build。
- CLI／API E2E。
- Playwright desktop 與 390px。
- Golden schema／tool export。
- Character Pack 與 malicious archive tests。
- Agent connector、cancel、process-tree、auth、receipt、estop。

不得只寫「全部通過」，要列出命令、passed／failed／skipped 數量與環境。

### 15.2 角色與遊戲互動

- Behavior Intent priority。
- Interaction interruption／resume。
- 不同通道動畫混合。
- 高頻事件 bounded。
- 同一動畫防重複。
- 點擊、連點、hover、拖曳、放下。
- 投擲軌跡、碰撞、追逐、抓取、帶回。
- 多角色互相注意與追逐。
- 角色隱藏／恢復。
- Reduced Motion。
- Quiet Hours。
- Fullscreen。
- 低效能降級。
- claimed-completed 不播放 verified。
- emergency 搶占所有遊戲動畫。

### 15.3 硬體

- Serial reconnect／timeout／cancel。
- BLE scan／connect／subscribe／disconnect／restore。
- MQTT reconnect、QoS 與重複訊息。
- Stable identity／無 stable ID。
- Pairing、verification、nonce、replay。
- Firmware hard limit＋Runtime limit。
- acknowledged-only／independent observation。
- ESP32 真硬體驗收與模擬器驗收分開。

### 15.4 iPhone

- 配對、撤銷、錯誤電腦、過期金鑰。
- Bonjour discovery、TLS WebSocket reconnect。
- 前景／背景／被系統終止後誠實狀態。
- Motion 語意事件。
- Haptic frequency limit。
- Camera／microphone／location permission denied／revoked。
- BLE gateway。
- iPhone 斷線後 capability unavailable。
- iOS 真機驗收；Simulator 不冒充 sensor／BLE 真機證據。

### 15.5 UI

- 5 個主要入口。
- 390px 所有主要流程可達。
- 鍵盤與 focus trap。
- Inbox 待確認數量。
- 每項設定只有單一 canonical owner。
- 三步首次設定。
- 裝置導向的人類語言。
- 一般模式不暴露不必要的 UUID、YAML、Token、Lease、Provider lifecycle。

---

## 十六、建議實作順序

### Phase 0：重新基線

- 同步最新版。
- 跑全部現有回歸。
- 記錄 commit、環境與現存限制。
- 建立本輪 Capability Gap Matrix；不得沿用「25/25 complete」掩蓋真實硬體與遊戲互動缺口。

### Phase 1：控制中心簡化

- 9 個入口縮成 5 個。
- 移除重複設定。
- 首頁瘦身。
- Activity 改 Inbox。
- 首次設定縮為 3 步。
- 保留進階技術入口。

### Phase 2：正式小樞與動畫核心

- 完成角色設計稿、輪廓、表情與分層資產格式。
- Interaction Director。
- 組合通道與動畫混合。
- 基礎生活、移動、玩家互動與 Agent 狀態動畫。
- 使用真資產，不得只畫靜態 mockup。

### Phase 3：遊戲互動

- 游標、點擊、連點、拖曳與放下。
- 玩具與輕量 2D 物理。
- 追逐、撲抓、帶回與拒絕歸還。
- 多角色架構與互動。
- 場景、匯出／匯入、Roll Call。

### Phase 4：AI 角色閉環

- Codex／Claude 事件標準化。
- Character Intent mapping。
- 真實 approval、working、claim、verify、cancel 演出。
- Conversation Provider 介面與無 Provider 降級。

### Phase 5：真硬體

- Serial、BLE、MQTT。
- ESP32 韌體、BOM、接線、參考 Adapter。
- 真裝置完整閉環。

### Phase 6：iPhone Provider

- SwiftUI App。
- 配對與安全連線。
- Motion、touch、haptic、notification、character presentation。
- BLE gateway。
- 真機驗收。

### Phase 7：整合與發版準備

- 跨平台、效能、race、斷線、恢復、對抗測試。
- 完整文件與證據。
- 明確列出仍未完成及無法在現有環境驗證的部分。

每個 Phase 都必須：先測試 → 實作 → 回歸 → 更新文件 → 提供真實畫面或機器證據，再進下一階段。

---

## 十七、完成定義

只有全部成立才算完成：

- 專案主體明確回到角色、硬體與 AI 三核心。
- 一般控制中心只有 5 個主要入口且沒有重複設定。
- 首次設定可在 3 個主要步驟完成。
- 小樞正式成為 Q 版貓娘女僕角色，不再只是工程示範素材。
- 即使沒有 AI、沒有硬體，小樞也能自主生活與玩耍。
- VS Code Pets 的游標、氣泡、丟球、多角色、追逐、命名、移除、匯入匯出與場景等互動基準全部具備。
- 小樞具備拖曳、放下、桌面空間、檔案接取、Agent 工作與硬體狀態等進階互動。
- Interaction Director 能處理注意力、組合動畫、中斷、恢復、變體與安全搶占。
- 本機低風險反應不呼叫 AI，具有即時性。
- Codex／Claude Code 是真實工作 Agent，不是 UI placeholder。
- Agent claim 不冒充 verified，角色演出遵守真實狀態。
- Serial、BLE、MQTT 至少完成可用 Adapter。
- ESP32 官方參考裝置能完成實際感知與動作閉環。
- iPhone 可作為 Mobile Provider，提供授權後的感測、呈現與 BLE gateway。
- 不宣稱 iPhone 可操作任意 USB 配件。
- 安全底線保留，但一般 UI 不被治理術語淹沒。
- 390px、鍵盤、Reduced Motion、Quiet Hours、Fullscreen 正常。
- 所有測試、實際命令、數量、畫面、硬體證據與已知限制完整交付。
- 不以 Mock、文件、靜態圖或 hard-coded response 冒充完成。

---

## 十八、最終交付報告

完成後提供：

1. 最新基準 commit、環境與 worktree 狀態。
2. 產品重構摘要。
3. 修改檔案完整清單。
4. 5 頁控制中心 IA 與設定歸屬。
5. 首次設定三步流程。
6. 小樞正式角色設定、配色、輪廓、服裝與資產格式。
7. 36 表情與全部動畫狀態清單。
8. Interaction Director、Attention、Utility、Scheduler 與 Mixer 架構。
9. VS Code Pets 基準功能逐項驗收。
10. 玩具與 2D 物理實作。
11. 玩家點擊、拖曳、投擲、多角色互動證據。
12. Codex／Claude Code 真 Session 驗收。
13. Agent 狀態到小樞演出的映射。
14. Serial、BLE、MQTT Adapter。
15. ESP32 BOM、接線、韌體與真機證據。
16. iPhone App、配對、感測、動器與 BLE Gateway。
17. 權限、資料範圍、風險分級與 Emergency Stop。
18. 記憶與知識 UI 簡化及資料相容性。
19. Migration 與向後相容。
20. 效能量測、事件延遲、FPS、記憶體與 bounded queue。
21. 所有測試命令及 passed／failed／skipped 數量。
22. Desktop、390px、角色動畫、硬體與 iPhone 真機畫面。
23. 無法執行或驗證項目與具體原因。
24. 替代檢查、仍存在風險與完整環境重跑命令。
25. 下一階段建議。

不得只寫「全部通過」。若沒有 iPhone 真機、ESP32、特定 OS 或 Apple Developer 簽章環境，必須清楚標示未完成，不得以 Simulator、Mock 或編譯成功冒充真機驗收。

請現在開始檢查最新專案，先跑回歸並依 Phase 實作。不要再擴張與三個核心無直接關係的新功能；安全留在底層，產品表面必須先做到可愛、好玩、能連真硬體、能讓 AI Agent 真正做事。
