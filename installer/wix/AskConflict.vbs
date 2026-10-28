' Prompt Keep vs Overwrite when config/parsers already exist. Sets CONFLICTACTION.
Dim fso, sh, folder, config, parsersFolder, hasConflict, answer, file
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh = CreateObject("WScript.Shell")
folder = sh.ExpandEnvironmentStrings("%USERPROFILE%") & "\.config\comruvi"
config = folder & "\config.toml"
parsersFolder = folder & "\parsers"
hasConflict = False
If fso.FileExists(config) Then hasConflict = True
If fso.FolderExists(parsersFolder) Then
  For Each file In fso.GetFolder(parsersFolder).Files
    If LCase(fso.GetExtensionName(file.Name)) = "toml" Then
      Select Case LCase(file.Name)
        Case "anima-ai.toml", "comfyui-generic.toml", "flux2.toml", "krea-ai.toml", "photo.toml", "z-image.toml"
          hasConflict = True
      End Select
    End If
  Next
End If
If hasConflict Then
  answer = MsgBox( _
    "Existing comruvi config and/or parsers were found under:" & vbCrLf & _
    folder & vbCrLf & vbCrLf & _
    "Yes = Overwrite (backup existing as *_YYMMDD_HHMM)" & vbCrLf & _
    "No  = Keep (save installer copies with the build version in the name)", _
    vbYesNo + vbQuestion + vbDefaultButton2, _
    "comruvi setup")
  If answer = vbYes Then
    Session.Property("CONFLICTACTION") = "Overwrite"
  Else
    Session.Property("CONFLICTACTION") = "Keep"
  End If
End If
