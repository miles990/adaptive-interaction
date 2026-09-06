-- AX 驅動工具。argv：<procName> <command> [args…]
--
-- 兩個踩過的坑，改動時不要退回去：
--   1. 主視窗一律用**名字**找（"Interaction Control Center"），不能用 window 1：
--      隱藏／顯示桌面角色之後視窗順序會變，window 1 可能是角色視窗，後面每一步
--      就會安靜地對錯的視窗操作（而且看起來只是「找不到按鈕」）。
--   2. `entire contents of X` 一定要先 `set flat to …` 存成變數再 repeat。直接
--      `repeat with e in (entire contents of X)` 在 System Events 上會拿到空清單。
--
-- 指令：
--   windows                          列出視窗標題（每行一個）
--   click <role> <text>              點第一個 role 且名字含 text 的元素（role=AXAny 表示不限）
--   exists <role> <text>             回 yes/no
--   value <role> <text>              回該元素的 value（aria-pressed 的按鈕在 macOS 上是 AXCheckBox）
--   fill <role> <text> <value>       對已定位的輸入框送全選＋鍵盤輸入（觸發真實 input 事件）
--   selectindex <role> <text> <n>    選單中選第 n 項（1-based，最多 16 項）
--   navclick <index>                 點「主要導覽」的第 index 顆按鈕（1-based）
--   tray <text>                      點狀態列選單中名字含 text 的項目
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
				if not (exists window "Open") then error "owned app has no Open panel"
				set frontmost to true
				keystroke "g" using {command down, shift down}
				delay 0.3
				keystroke (item 3 of argv)
				key code 36
				delay 0.4
				key code 36
				return "selected file in owned Open panel"
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
			-- 以下都要走 AX 樹（見檔頭第 2 點：先存成變數）。
			set flat to entire contents of target
			if cmd is "dump" then
				set out to ""
				repeat with e in flat
					try
						set out to out & (role of e) & ": " & my labelOf(e) & linefeed
					end try
				end repeat
				return out
			end if
			if cmd is "hscroll" then
				repeat with e in flat
					try
						if (role of e) is "AXScrollBar" then
							if (value of attribute "AXOrientation" of e) contains "Horizontal" then return "yes"
						end if
					end try
				end repeat
				return "no"
			end if
			if cmd is "navclick" then
				set wantIdx to (item 3 of argv) as integer
				set navIdx to 0
				set i to 0
				repeat with e in flat
					set i to i + 1
					try
						if (description of e) is "主要導覽" then
							set navIdx to i
							exit repeat
						end if
					end try
				end repeat
				if navIdx is 0 then error "nav not found"
				-- 導覽群組後面緊接著就是它的按鈕：往後掃到第一個非按鈕為止。
				set btns to {}
				set j to navIdx + 1
				repeat while j ≤ (count of flat)
					set r to ""
					try
						set r to (role of (item j of flat)) as text
					end try
					if r is not "AXButton" then exit repeat
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
			repeat with e in flat
				try
					set r to (role of e) as text
					if wantRole is "AXAny" or r is wantRole then
						set lbl to my labelOf(e)
						if lbl is not "" and lbl contains wantText then set end of hits to e
					end if
				end try
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
			if cmd is "fill" then
				if (role of target2) is not "AXTextArea" and (role of target2) is not "AXTextField" then error "fill target is not editable text"
				set frontmost to true
				click target2
				keystroke "a" using {command down}
				keystroke (item 5 of argv)
				return "filled owned input"
			end if
			if cmd is "selectindex" then
				set wantedIndex to (item 5 of argv) as integer
				if wantedIndex < 1 or wantedIndex > 16 then error "selection index outside 1..16"
				if (role of target2) is not "AXPopUpButton" and (role of target2) is not "AXComboBox" then error "select target is not a native select"
				set frontmost to true
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
			click target2
			return "clicked " & my labelOf(target2)
		end tell
	end tell
end run
