' Starts `mickey run` without a console window (window style 0 = hidden),
' its messages in %LOCALAPPDATA%\mickey-run.log for troubleshooting (never
' keys or clipboard content). mickey.exe sits next to this script.
Set shell = CreateObject("WScript.Shell")
dir = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
shell.Run "cmd /c """"" & dir & "\mickey.exe"" run > ""%LOCALAPPDATA%\mickey-run.log"" 2>&1""", 0, False
