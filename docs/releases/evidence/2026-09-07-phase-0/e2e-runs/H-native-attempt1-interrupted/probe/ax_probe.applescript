-- Probe helper (lives in the run output dir; the repo helper is untouched here).
on run argv
	set procName to item 1 of argv
	set cmd to item 2 of argv
	tell application "System Events"
		set procId to (text 5 thru -1 of procName) as integer
		set p to first process whose unix id is procId
		tell p
			if cmd is "windows" then
				set out to ""
				repeat with w in windows
					set out to out & (name of w) & " | subrole=" & (subrole of w) & linefeed
				end repeat
				return out
			end if
			if cmd is "panelfull" then
				if not (exists window "Open") then return "NO-OPEN-PANEL"
				set flat to entire contents of window "Open"
				set out to "count=" & (count of flat) & linefeed
				repeat with e in flat
					set r to ""
					set nm to ""
					set vl to ""
					set sb to ""
					set sel to ""
					try
						set r to (role of e) as text
					end try
					try
						set nm to (name of e) as text
					end try
					try
						set vl to (value of e) as text
					end try
					try
						set sb to (subrole of e) as text
					end try
					try
						set sel to (value of attribute "AXSelected" of e) as text
					end try
					set out to out & r & " |sub=" & sb & " |name=" & nm & " |value=" & vl & " |sel=" & sel & linefeed
				end repeat
				return out
			end if
			if cmd is "sheetexists" then
				if not (exists window "Open") then return "no-panel"
				tell window "Open"
					if exists sheet 1 then
						return "yes"
					else
						return "no"
					end if
				end tell
			end if
			if cmd is "gotofolder" then
				set frontmost to true
				keystroke "g" using {command down, shift down}
				return "sent cmd-shift-g"
			end if
			if cmd is "pasteclip" then
				set frontmost to true
				keystroke "a" using {command down}
				keystroke "v" using {command down}
				return "pasted"
			end if
			if cmd is "typepath" then
				set frontmost to true
				keystroke "a" using {command down}
				keystroke (item 3 of argv)
				return "typed"
			end if
			if cmd is "readsheet" then
				tell window "Open"
					if not (exists sheet 1) then return "NO-SHEET"
					set out to ""
					set flat to entire contents of sheet 1
					repeat with e in flat
						set r to ""
						set nm to ""
						set vl to ""
						try
							set r to (role of e) as text
						end try
						try
							set nm to (name of e) as text
						end try
						try
							set vl to (value of e) as text
						end try
						set out to out & r & " |name=" & nm & " |value=" & vl & linefeed
					end repeat
					return out
				end tell
			end if
			if cmd is "return" then
				set frontmost to true
				key code 36
				return "return"
			end if
			if cmd is "escape" then
				set frontmost to true
				key code 53
				return "escape"
			end if
			if cmd is "slowdump" then
				set target to missing value
				repeat with w in windows
					if (name of w) is "Interaction Control Center" then set target to w
				end repeat
				set flat to entire contents of target
				set out to ""
				repeat with e in flat
					try
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
						set out to out & (role of e) & ": " & lbl & linefeed
					end try
				end repeat
				return out
			end if
			if cmd is "batchdump" then
				set target to missing value
				repeat with w in windows
					if (name of w) is "Interaction Control Center" then set target to w
				end repeat
				set roleList to role of (entire contents of target)
				set nameList to name of (entire contents of target)
				set descList to description of (entire contents of target)
				set out to ""
				set n to count of roleList
				repeat with i from 1 to n
					set lbl to item i of nameList
					if lbl is missing value or lbl is "" then set lbl to item i of descList
					if lbl is missing value then set lbl to ""
					set out to out & (item i of roleList) & ": " & lbl & linefeed
				end repeat
				return out
			end if
			return "unknown command"
		end tell
	end tell
end run
