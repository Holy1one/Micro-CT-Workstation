# Read compiled MSI UI tables. No install or custom action is invoked.
param([Parameter(Mandatory)][string]$Path)
$ErrorActionPreference='Stop'
$taskInstaller=New-Object -ComObject WindowsInstaller.Installer
$taskDatabase=$taskInstaller.OpenDatabase((Resolve-Path -LiteralPath $Path).Path,0)
if(([int]$taskDatabase.SummaryInformation(0).Property(15) -band 8) -ne 0){throw 'Installer must allow UAC elevation for protected rollback cleanup during repair'}
$taskPrivilegeView=$taskDatabase.OpenView('SELECT `Value` FROM `Property` WHERE `Property` = ''ALLUSERS''')
$taskPrivilegeView.Execute()
$taskScopeRecord=$taskPrivilegeView.Fetch()
if($taskScopeRecord -and $taskScopeRecord.StringData(1)){throw 'Existing current-user installation context must be preserved'}
$taskFonts=@{}
$taskView=$taskDatabase.OpenView('SELECT `TextStyle`, `FaceName`, `Size` FROM `TextStyle`')
$taskView.Execute()
while($taskRecord=$taskView.Fetch()){$taskFonts[$taskRecord.StringData(1)]=@{face=$taskRecord.StringData(2);size=$taskRecord.IntegerData(3)}}
foreach($taskStyle in @(@('WixUI_Font_Normal',8),@('WixUI_Font_Bigger',12),@('WixUI_Font_Title',9))){
    if($taskFonts[$taskStyle[0]].face -ne 'Tahoma' -or $taskFonts[$taskStyle[0]].size -ne $taskStyle[1]){throw 'Installer fonts do not match the shared WixUI control measurements'}
}
$taskView=$taskDatabase.OpenView('SELECT `Value` FROM `Property` WHERE `Property` = ''DefaultUIFont''')
$taskView.Execute()
if($taskView.Fetch().StringData(1) -ne 'WixUI_Font_Normal'){throw 'Unexpected default installer font'}
$taskDialogs=@();$taskView=$taskDatabase.OpenView('SELECT `Dialog` FROM `Dialog`');$taskView.Execute()
while($taskRecord=$taskView.Fetch()){$taskDialogs+=$taskRecord.StringData(1)}
if('BrowseDlg' -in $taskDialogs){throw 'Legacy fixed-size folder browser must not enter the MSI'}
$taskControlCount=0;$taskView=$taskDatabase.OpenView('SELECT `Dialog_`, `Control`, `Type`, `Height` FROM `Control`');$taskView.Execute()
while($taskRecord=$taskView.Fetch()){
    $taskControlCount++
    if($taskRecord.StringData(3) -in @('DirectoryList','DirectoryCombo')){throw 'Legacy folder image-list control is present'}
    if($taskRecord.StringData(3) -eq 'Text' -and $taskRecord.IntegerData(4) -lt 10){throw ('Text control is shorter than a normal line: '+$taskRecord.StringData(1)+'/'+$taskRecord.StringData(2))}
}
$taskView=$taskDatabase.OpenView('SELECT `Type`, `Source`, `Target` FROM `CustomAction` WHERE `Action` = ''ChooseInstallFolder''')
$taskView.Execute();$taskAction=$taskView.Fetch()
if(-not $taskAction -or $taskAction.IntegerData(1) -ne 1 -or $taskAction.StringData(2) -ne 'NativeFolderPicker' -or $taskAction.StringData(3) -ne 'ChooseInstallFolder'){throw 'Missing immediate native folder selection action'}
$taskView=$taskDatabase.OpenView('SELECT `Event`, `Argument` FROM `ControlEvent` WHERE `Dialog_` = ''InstallDirDlg'' AND `Control_` = ''ChangeFolder''')
$taskView.Execute();$taskNativeEvent=$false;$taskPathRefresh=$false
while($taskRecord=$taskView.Fetch()){
    if($taskRecord.StringData(1) -eq 'DoAction' -and $taskRecord.StringData(2) -eq 'ChooseInstallFolder'){$taskNativeEvent=$true}
    if($taskRecord.StringData(1) -eq '[INSTALLDIR]' -and $taskRecord.StringData(2) -eq '[NATIVE_SELECTED_PATH]'){$taskPathRefresh=$true}
    if($taskRecord.StringData(1) -eq 'NewDialog' -and $taskRecord.StringData(2) -eq 'InstallDirDlg'){throw 'Reopening the current MSI dialog causes error 2856'}
}
if(-not $taskNativeEvent){throw 'Change Folder button does not open the native picker'}
if(-not $taskPathRefresh){throw 'Selected folder must update the path control through an MSI property event'}
Write-Output ("PASS compiled installer UI: {0} dialogs / {1} controls; matching Tahoma fonts; native folder picker" -f $taskDialogs.Count,$taskControlCount)
