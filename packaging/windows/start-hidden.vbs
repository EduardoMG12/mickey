' Starts `mickey run` without a console window (window style 0 = hidden).
' mickey.exe sits next to this script.
Set shell = CreateObject("WScript.Shell")
dir = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
shell.Run """" & dir & "\mickey.exe"" run", 0, False
