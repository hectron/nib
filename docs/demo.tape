Output docs/demo.gif

Require nib

Set Shell "bash"
Set Theme "rose-pine-dawn"
Set FontFamily "TX-02"
Set FontSize 16
Set Width 1100
Set Height 700
Set TypingSpeed 35ms

Env NIB_DEMO "1"

Sleep 1s
Type `nib t Plan launch tomorrow !high`
Enter
Sleep 3s

Ctrl+L
Sleep 500ms
Type "nib today"
Enter
Sleep 3s

Ctrl+L
Sleep 500ms
Type "nib note list -f Work"
Enter
Sleep 3s

Ctrl+L
Sleep 500ms
Type "nib today --json"
Enter
Sleep 3s
