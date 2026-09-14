-- AX 驅動工具。argv：<procName> <command> [args…]
--
-- 三個踩過的坑，改動時不要退回去：
--   1. 主視窗一律用**名字**找（"Interaction Control Center"），不能用 window 1：
--      隱藏／顯示桌面角色之後視窗順序會變，window 1 可能是角色視窗，後面每一步
--      就會安靜地對錯的視窗操作（而且看起來只是「找不到按鈕」）。
--   2. `entire contents of X` 一定要先 `set flat to …` 存成變數再 repeat。直接
--      `repeat with e in (entire contents of X)` 在 System Events 上會拿到空清單。
--      走訪時每個元素只抓一次 `properties`（一個 Apple event），不要分開抓
--      role／name／description：頁面元素變多之後，逐屬性走訪會超過 driver 的
--      per-call timeout（2026-09-07 的 mobile 走查就是這樣掛在 dump 上）。
--   3. **不要用 `keystroke`，一律 `key code`＋剪貼簿，而且要讀回驗證**。
--      這台機器的輸入來源是注音（com.apple.inputmethod.TCIM.Zhuyin）：
--      `keystroke "字串"` 會被輸入法吃掉／掉字（2026-09-07 composer 只收到
--      'Native fixture codex can'），`keystroke "g" using {command down, shift down}`
--      甚至完全打不開 Go-to-Folder（同一個面板改用 `key code 5` 就開得起來）。
--      key code 是實體鍵位，不受鍵盤布局／輸入法字元對應影響；文字則走剪貼簿貼上。
--      沒有讀回就回報成功等於製造假陽性（2026-09-06／09-07 的 choosefile 就是這樣：
--      面板其實停在它自己記住的舊目錄，第二次 Return 開了字母序第一個檔）。
--      貼完把使用者原本的剪貼簿還回去。
--
-- 指令：
--   windows                          列出視窗標題（每行一個）
--   click <role> <text>              點第一個 role 且名字含 text 的元素（role=AXAny 表示不限）
--   exists <role> <text>             回 yes/no
--   value <role> <text>              回該元素的 value（aria-pressed 的按鈕在 macOS 上是 AXCheckBox）
--   fill <role> <text> <value>       對已定位的輸入框貼上 value（全選→貼上→讀回比對，不符就重試並最終 error）
--   selectindex <role> <text> <n>    選單中選第 n 項（1-based，最多 16 項）
--   navclick <index>                 點「主要導覽」的第 index 顆按鈕（1-based）
--   tray <text>                      點狀態列選單中名字含 text 的項目
--   clickconfirm <role> <text> <role2> <text2>
--                                    兩段式確認：同一次呼叫內按下 <text> 再按 <text2>
--                                    （確認鈕 5 秒後自動解除，分兩次呼叫必然來不及）
--   choosefile <path>                在自己的 Open 面板選 path：Go-to-Folder 貼上路徑→讀回→
--                                    斷言面板真的到了該目錄（Where: 欄）→才開檔；任何一步
--                                    不成立就 error（呼叫端要記成 harness 失敗，不准當成功）
--   bounds                           回 x,y,w,h（主視窗）
--   resize <w> <h>                   設定主視窗大小
--   hscroll                          回 yes/no：主視窗裡有沒有水平捲軸
on labelOf(e)
	tell application "System Events"
		set lbl to ""
		try
			set lbl to (name of e) as text
		end try
		if lbl is "missing value" or lbl is "" then
			try
				set lbl to (description of e) as text
			end try
		end if
		if lbl is "missing value" then set lbl to ""
		return lbl
	end tell
end labelOf

-- 一個元素一次 Apple event；拿不到就回 missing value，由呼叫端跳過。
-- 只有 dump 需要整棵樹的 properties；找元素的指令一律用 props1 邊走邊抓，
-- 找到就停（2026-09-08：先把整棵樹的 properties 都抓完再搜尋，讓「裝置與能力」
-- 頁的每次 exists／click 都要 55-65 秒，兩段式確認鈕的 5 秒時限根本追不上）。
on props1(e)
	tell application "System Events"
		try
			return properties of e
		on error
			return missing value
		end try
	end tell
end props1

on propsOf(flat)
	tell application "System Events"
		set acc to {}
		repeat with e in flat
			try
				set end of acc to properties of e
			on error
				set end of acc to missing value
			end try
		end repeat
		return acc
	end tell
end propsOf

-- 這幾個 handler 一定要包在 `tell application "System Events"` 裡：
-- properties 記錄的鍵（role／name／description）是 System Events 字典的詞彙，
-- 在外面讀會安靜地拿到空字串，整棵樹就會像「什麼都找不到」。
on roleOfProps(pr)
	if pr is missing value then return ""
	tell application "System Events"
		set r to ""
		try
			set r to (role of pr) as text
		end try
		if r is "missing value" then set r to ""
		return r
	end tell
end roleOfProps

on descOfProps(pr)
	if pr is missing value then return ""
	tell application "System Events"
		set d to ""
		try
			set d to (description of pr) as text
		end try
		if d is "missing value" then set d to ""
		return d
	end tell
end descOfProps

on labelOfProps(pr)
	if pr is missing value then return ""
	tell application "System Events"
		set lbl to ""
		try
			set lbl to (name of pr) as text
		end try
		if lbl is "missing value" or lbl is "" then
			set lbl to my descOfProps(pr)
		end if
		if lbl is "missing value" then set lbl to ""
		return lbl
	end tell
end labelOfProps

-- 剪貼簿：handler 放在最外層才會在 script 自己的 context 執行（Standard Additions）。
on readClipboard()
	try
		return (the clipboard as text)
	on error
		return missing value
	end try
end readClipboard

on writeClipboard(t)
	set the clipboard to t
end writeClipboard

on restoreClipboard(saved)
	if saved is not missing value then
		try
			set the clipboard to saved
		end try
	end if
end restoreClipboard

-- 這台機器上會有別的 App（例如 ChatGPT）把前景搶回去；而且本 App 是狀態列
-- 常駐型，啟用後不一定留在前景。合成按鍵只在「這一刻我們真的在前景」時才送，
-- 否則按鍵會打到別人的視窗，而且我們還會以為輸入成功。
on focusProcess(p)
	tell application "System Events"
		repeat 25 times
			try
				set frontmost of p to true
			end try
			try
				if frontmost of p then return true
			end try
			delay 0.1
		end repeat
		return false
	end tell
end focusProcess

on basenameOf(p)
	set previous to AppleScript's text item delimiters
	set AppleScript's text item delimiters to "/"
	set parts to text items of p
	set AppleScript's text item delimiters to previous
	repeat with i from (count of parts) to 1 by -1
		if (item i of parts) is not "" then return item i of parts
	end repeat
	return ""
end basenameOf

on parentNameOf(p)
	set previous to AppleScript's text item delimiters
	set AppleScript's text item delimiters to "/"
	set parts to text items of p
	set AppleScript's text item delimiters to previous
	set seen to 0
	repeat with i from (count of parts) to 1 by -1
		if (item i of parts) is not "" then
			set seen to seen + 1
			if seen is 2 then return item i of parts
		end if
	end repeat
	return ""
end parentNameOf

on run argv
	set procName to item 1 of argv
	set cmd to item 2 of argv
	tell application "System Events"
		if procName starts with "pid:" then
			set procId to (text 5 thru -1 of procName) as integer
			if not (exists (first process whose unix id is procId)) then error "process not running: " & procName
			set ownedProcess to first process whose unix id is procId
		else
			if not (exists process procName) then error "process not running: " & procName
			set ownedProcess to process procName
		end if
		tell ownedProcess
			if cmd is "choosefile" then
				-- Only the task-owned application's native file panel receives keys.
				set wantedPath to item 3 of argv
				set wantedName to my basenameOf(wantedPath)
				set wantedFolder to my parentNameOf(wantedPath)
				if wantedName is "" or wantedFolder is "" then error "choosefile needs an absolute file path, got: " & wantedPath
				if not (exists window "Open") then error "owned app has no Open panel"
				set savedClipboard to my readClipboard()
				my writeClipboard(wantedPath)
				-- 1. Go-to-Folder sheet：面板剛開時不一定是 key window，前景也可能被
				--    別的 App 搶走，所以每一輪都重新拿前景＋把面板 raise 起來再送和絃，
				--    並斷言 sheet 真的出現（沒出現就 error，不硬著頭皮往下打字）。
				set sheetOpen to false
				repeat 8 times
					if not my focusProcess(ownedProcess) then
						my restoreClipboard(savedClipboard)
						error "choosefile: another app kept the foreground; refusing to type blind"
					end if
					if not (exists window "Open") then
						my restoreClipboard(savedClipboard)
						error "choosefile: Open panel disappeared before Go-to-Folder for " & wantedPath
					end if
					try
						perform action "AXRaise" of window "Open"
					end try
					try
						set value of attribute "AXFocused" of window "Open" to true
					end try
					delay 0.15
					key code 5 using {command down, shift down} -- Cmd-Shift-G
					repeat 12 times
						delay 0.25
						try
							if (exists sheet 1 of window "Open") then
								set sheetOpen to true
								exit repeat
							end if
						end try
					end repeat
					if sheetOpen then exit repeat
					delay 0.5
				end repeat
				if not sheetOpen then
					-- 說出當下看到什麼，下一次才有得追：只列自己這個 process 的視窗名。
					set seenWindows to ""
					try
						repeat with w in (every window)
							set seenWindows to seenWindows & (name of w) & "; "
						end repeat
					end try
					my restoreClipboard(savedClipboard)
					error "choosefile: Go-to-Folder sheet never opened for " & wantedPath & " (owned windows: " & seenWindows & ")"
				end if
				-- 2. 貼上路徑並讀回：沒讀到一模一樣的字串就不算輸入成功。
				set pasted to false
				set fieldValue to ""
				repeat 4 times
					if not my focusProcess(ownedProcess) then
						my restoreClipboard(savedClipboard)
						error "choosefile: another app kept the foreground; refusing to type blind"
					end if
					key code 0 using {command down} -- Cmd-A
					key code 9 using {command down} -- Cmd-V
					repeat 12 times
						delay 0.25
						set fieldValue to ""
						try
							set fieldValue to (value of text field 1 of sheet 1 of window "Open") as text
						end try
						if fieldValue is wantedPath then
							set pasted to true
							exit repeat
						end if
					end repeat
					if pasted then exit repeat
				end repeat
				if not pasted then
					my restoreClipboard(savedClipboard)
					error "choosefile: Go-to-Folder readback mismatch (wanted " & wantedPath & ", field " & fieldValue & ")"
				end if
				-- 3. 送出；自動完成清單可能吃掉第一個 Return，所以重試到 sheet 消失。
				set sheetClosed to false
				repeat 3 times
					if not my focusProcess(ownedProcess) then
						my restoreClipboard(savedClipboard)
						error "choosefile: another app kept the foreground; refusing to type blind"
					end if
					key code 36
					repeat 16 times
						delay 0.25
						try
							if not (exists window "Open") then
								set sheetClosed to true
								exit repeat
							end if
							if not (exists sheet 1 of window "Open") then
								set sheetClosed to true
								exit repeat
							end if
						end try
					end repeat
					if sheetClosed then exit repeat
				end repeat
				if not sheetClosed then
					my restoreClipboard(savedClipboard)
					error "choosefile: Go-to-Folder sheet stayed open for " & wantedPath
				end if
				-- 4. 斷言面板真的換到了目標目錄。少了這一步，下一個 Return 會開
				--    面板自己記住的舊目錄裡被選到的檔案（假陽性的來源）。
				set located to false
				set whereValue to ""
				repeat 24 times
					try
						set whereValue to (value of pop up button 1 of splitter group 1 of window "Open") as text
					end try
					if whereValue is wantedFolder then
						set located to true
						exit repeat
					end if
					delay 0.25
				end repeat
				if not located then
					my restoreClipboard(savedClipboard)
					error "choosefile: panel did not navigate to folder " & wantedFolder & " (Where: " & whereValue & ") for " & wantedPath
				end if
				-- 5. 真的開檔，並斷言面板收掉了。
				--    （試過先在面板裡找到該列並設 AXSelected 再送 Return：
				--     `entire contents of window "Open"` 在這台機器上一次要十幾秒，
				--     輪詢下來直接超過 driver 的 180 秒上限，所以不留在這裡。
				--     面板偶爾不開檔的殘留問題見本次報告的 harness 限制。）Go-to-Folder 換完目錄的那一瞬間
				--    檔案清單不一定已經把目標列選起來，這一下 Return 就會落空
				--    （2026-09-08 legacy 第二輪即卡在此）。所以送出改成有界重試，
				--    每次重新確認前景；重試不會換檔案（面板還停在同一個目錄，
				--    而該目錄只放這一個檔），最後仍沒收掉才 error。
				set panelClosed to false
				repeat 3 times
					if not my focusProcess(ownedProcess) then
						my restoreClipboard(savedClipboard)
						error "choosefile: another app kept the foreground; refusing to type blind"
					end if
					key code 36
					repeat 24 times
						delay 0.25
						if not (exists window "Open") then
							set panelClosed to true
							exit repeat
						end if
					end repeat
					if panelClosed then exit repeat
					delay 0.5
				end repeat
				my restoreClipboard(savedClipboard)
				if not panelClosed then error "choosefile: Open panel stayed open after choosing " & wantedName
				return "chose " & wantedName & " in " & wantedFolder
			end if
			if cmd is "windows" then
				set out to ""
				repeat with w in windows
					set out to out & (name of w) & linefeed
				end repeat
				return out
			end if
			if cmd is "tray" then
				set wanted to item 3 of argv
				-- 狀態列：menu bar 2 是這個 App 自己的 status item。
				tell menu bar 2
					click menu bar item 1
					delay 0.6
					tell menu 1 of menu bar item 1
						repeat with mi in menu items
							set t to ""
							try
								set t to name of mi
							end try
							if t contains wanted then
								click mi
								return "clicked:" & t
							end if
						end repeat
						key code 53 -- Escape：不要把選單留在畫面上
					end tell
				end tell
				error "tray item not found: " & wanted
			end if
			-- 主視窗以名字定位（見檔頭第 1 點）。
			set target to missing value
			repeat with w in windows
				if (name of w) is "Interaction Control Center" then set target to w
			end repeat
			if target is missing value then
				if (count of windows) is 0 then error "no window"
				set target to window 1
			end if
			if cmd is "bounds" then
				set p to position of target
				set s to size of target
				return ((item 1 of p) as text) & "," & ((item 2 of p) as text) & "," & ((item 1 of s) as text) & "," & ((item 2 of s) as text)
			end if
			if cmd is "resize" then
				set w to (item 3 of argv) as integer
				set h to (item 4 of argv) as integer
				set size of target to {w, h}
				return "resized"
			end if
			-- 以下都要走 AX 樹（見檔頭第 2 點：先存成變數，再一次抓 properties）。
			set flat to entire contents of target
			set total to (count of flat)
			if cmd is "dump" then
				set props to my propsOf(flat)
				set out to ""
				repeat with i from 1 to total
					set pr to item i of props
					if pr is not missing value then
						set out to out & my roleOfProps(pr) & ": " & my labelOfProps(pr) & linefeed
					end if
				end repeat
				return out
			end if
			if cmd is "hscroll" then
				repeat with i from 1 to total
					if my roleOfProps(my props1(item i of flat)) is "AXScrollBar" then
						try
							if (value of attribute "AXOrientation" of (item i of flat)) contains "Horizontal" then return "yes"
						end try
					end if
				end repeat
				return "no"
			end if
			if cmd is "navclick" then
				set wantIdx to (item 3 of argv) as integer
				set navIdx to 0
				repeat with i from 1 to total
					if my descOfProps(my props1(item i of flat)) is "主要導覽" then
						set navIdx to i
						exit repeat
					end if
				end repeat
				if navIdx is 0 then error "nav not found"
				-- 導覽群組後面緊接著就是它的按鈕：往後掃到第一個非按鈕為止。
				set btns to {}
				set j to navIdx + 1
				repeat while j ≤ total
					if my roleOfProps(my props1(item j of flat)) is not "AXButton" then exit repeat
					set end of btns to (item j of flat)
					set j to j + 1
				end repeat
				if (count of btns) < wantIdx then error "nav has only " & (count of btns) & " items"
				click item wantIdx of btns
				return "clicked nav " & wantIdx & " (" & my labelOf(item wantIdx of btns) & ")"
			end if
			set wantRole to item 3 of argv
			set wantText to item 4 of argv
			set hits to {}
			repeat with i from 1 to total
				set pr to my props1(item i of flat)
				if pr is not missing value then
					set r to my roleOfProps(pr)
					if wantRole is "AXAny" or r is wantRole then
						set lbl to my labelOfProps(pr)
						if lbl is not "" and lbl contains wantText then
							set end of hits to (item i of flat)
							exit repeat -- Every command below acts only on the first hit.
						end if
					end if
				end if
			end repeat
			if cmd is "exists" then
				if (count of hits) > 0 then
					return "yes"
				else
					return "no"
				end if
			end if
			if (count of hits) < 1 then error "not found (" & wantRole & "/" & wantText & ")"
			set target2 to item 1 of hits
			if cmd is "clickconfirm" then
				-- 兩段式確認鈕（ConfirmButton）武裝後 5 秒就自己解除（Dialog.tsx），
				-- 所以「按下動作鈕」與「按下確認鈕」必須在同一次呼叫裡完成，第二段
				-- 只在動作鈕所屬容器裡找，不再重走整個視窗。這是量測工具的時序修正：
				-- 確認鈕仍必須真的出現、真的被按下，找不到就 error（不得當成已確認）。
				set confirmRole to item 5 of argv
				set confirmText to item 6 of argv
				set scopes to {}
				try
					set parentOne to value of attribute "AXParent" of target2
					set end of scopes to parentOne
					try
						set end of scopes to (value of attribute "AXParent" of parentOne)
					end try
				end try
				if (count of scopes) is 0 then error "clickconfirm: no container around " & wantText
				click target2
				set confirmHit to missing value
				set sawAny to false
				repeat 25 times
					repeat with sc in scopes
						set scoped to {}
						try
							set scoped to entire contents of sc
						end try
						if (count of scoped) > 0 then set sawAny to true
						repeat with e in scoped
							set pr2 to my props1(e)
							if pr2 is not missing value then
								if confirmRole is "AXAny" or my roleOfProps(pr2) is confirmRole then
									set lbl2 to my labelOfProps(pr2)
									if lbl2 is not "" and lbl2 contains confirmText then
										set confirmHit to e
										exit repeat
									end if
								end if
							end if
						end repeat
						if confirmHit is not missing value then exit repeat
					end repeat
					if confirmHit is not missing value then exit repeat
					delay 0.1
				end repeat
				if confirmHit is missing value then
					if sawAny then
						error "clickconfirm: confirm control never appeared (" & confirmRole & "/" & confirmText & ")"
					else
						error "clickconfirm: container went stale before confirm (" & confirmRole & "/" & confirmText & ")"
					end if
				end if
				click confirmHit
				return "confirmed " & confirmText
			end if
			if cmd is "fill" then
				if (role of target2) is not "AXTextArea" and (role of target2) is not "AXTextField" then error "fill target is not editable text"
				set wantedValue to item 5 of argv
				if not my focusProcess(ownedProcess) then error "fill: another app kept the foreground; refusing to type blind"
				click target2
				-- 剪貼簿貼上而不是逐字輸入（見檔頭第 3 點），而且每次都讀回；
				-- 讀不到一模一樣的值就重試，最後仍不符就 error，不留假成功。
				set savedClipboard to my readClipboard()
				my writeClipboard(wantedValue)
				set filled to false
				set current to ""
				repeat 4 times
					if not my focusProcess(ownedProcess) then
						my restoreClipboard(savedClipboard)
						error "fill: another app kept the foreground; refusing to type blind"
					end if
					key code 0 using {command down} -- Cmd-A
					key code 9 using {command down} -- Cmd-V
					repeat 12 times
						delay 0.25
						set current to ""
						try
							set current to (value of target2) as text
						end try
						if current is wantedValue then
							set filled to true
							exit repeat
						end if
					end repeat
					if filled then exit repeat
				end repeat
				my restoreClipboard(savedClipboard)
				if not filled then error "fill readback mismatch (wanted " & wantedValue & ", got " & current & ")"
				return "filled owned input"
			end if
			if cmd is "selectindex" then
				set wantedIndex to (item 5 of argv) as integer
				if wantedIndex < 1 or wantedIndex > 16 then error "selection index outside 1..16"
				if (role of target2) is not "AXPopUpButton" and (role of target2) is not "AXComboBox" then error "select target is not a native select"
				if not my focusProcess(ownedProcess) then error "selectindex: another app kept the foreground; refusing to type blind"
				click target2
				-- Home selects the first menu item without depending on the prior
				-- value or on whether arrow keys wrap around at the menu boundary.
				key code 115
				repeat (wantedIndex - 1) times
					key code 125
				end repeat
				key code 36
				return "selected owned option " & wantedIndex
			end if
			if cmd is "value" then
				set v to ""
				try
					set v to (value of target2) as text
				end try
				return v
			end if
			-- 頁面重新算圖會讓剛找到的參照失效（「無法取得 group N of …」）。
			-- click 一旦丟出錯誤就是「沒有按到」，所以重走一次樹再按是安全的，
			-- 不會變成按兩下；找不到元素或連續失敗就照實 error 出去。
			repeat with attempt from 1 to 3
				try
					click target2
					set clickedLabel to ""
					try
						set clickedLabel to my labelOf(target2)
					end try
					return "clicked " & clickedLabel
				on error clickError
					if attempt is 3 then error "click failed after 3 attempts (" & wantRole & "/" & wantText & "): " & clickError
				end try
				delay 0.4
				set flat to entire contents of target
				set total to (count of flat)
				set target2 to missing value
				repeat with i from 1 to total
					set pr to my props1(item i of flat)
					if pr is not missing value then
						if wantRole is "AXAny" or my roleOfProps(pr) is wantRole then
							set lbl to my labelOfProps(pr)
							if lbl is not "" and lbl contains wantText then
								set target2 to (item i of flat)
								exit repeat
							end if
						end if
					end if
				end repeat
				if target2 is missing value then error "click retry: element vanished (" & wantRole & "/" & wantText & ")"
			end repeat
			error "click: unreachable"
		end tell
	end tell
end run
