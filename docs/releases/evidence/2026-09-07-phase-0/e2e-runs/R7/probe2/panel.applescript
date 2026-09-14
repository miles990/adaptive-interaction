on run argv
    set pidv to (item 1 of argv) as integer
    tell application "System Events"
      set p to first process whose unix id is pidv
      tell p
        set out to ""
        repeat with w in windows
          set out to out & "WINDOW: " & (name of w) & " subrole=" & (subrole of w) & linefeed
        end repeat
        if exists window "Open" then
          tell window "Open"
            set out to out & "OPEN-PANEL children:" & linefeed
            repeat with e in UI elements
              set lbl to ""
              try
                set lbl to (name of e) as text
              end try
              set out to out & "  " & (role of e) & " | " & lbl & linefeed
            end repeat
            if exists sheet 1 then
              set out to out & "SHEET present" & linefeed
              repeat with e in UI elements of sheet 1
                set lbl to ""
                try
                  set lbl to (name of e) as text
                end try
                set vv to ""
                try
                  set vv to (value of e) as text
                end try
                set out to out & "  SHEET " & (role of e) & " | " & lbl & " | " & vv & linefeed
              end repeat
            end if
            try
              set out to out & "TITLE-STATIC: " & (value of static text 1) & linefeed
            end try
          end tell
        end if
        return out
      end tell
    end tell
    end run
