Output docs/demo.gif

Require nib

Set Shell "bash"
Set FontSize 16
Set Width 1100
Set Height 700
Set TypingSpeed 35ms

Env NIB_DEMO "1"
Env NO_COLOR "1"

Sleep 1s
Type `nib t "Plan launch" --due tomorrow --priority high`
Enter
Sleep 3s

Ctrl+L
Sleep 500ms
Type "nib t"
Enter
Sleep 3s

Ctrl+L
Sleep 500ms
Type "nib cal --week"
Enter
Sleep 3s

Ctrl+L
Sleep 500ms
Type "nib t --json"
Enter
Sleep 3s
