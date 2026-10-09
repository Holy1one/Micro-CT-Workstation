# Reads installer tables only; does not invoke installation or custom actions.
param([Parameter(Mandatory)][string]$Path)
$ErrorActionPreference='Stop'
$installer=New-Object -ComObject WindowsInstaller.Installer
$database=$installer.OpenDatabase((Resolve-Path -LiteralPath $Path).Path,0)
$properties=@{}
$view=$database.OpenView('SELECT `Property`, `Value` FROM `Property`')
$view.Execute()
while($record=$view.Fetch()){$properties[$record.StringData(1)]=$record.StringData(2)}
$summary=$database.SummaryInformation(0)
$features=@();$view=$database.OpenView('SELECT `Feature`, `Title`, `Level` FROM `Feature`');$view.Execute()
while($record=$view.Fetch()){$features+=@{id=$record.StringData(1);title=$record.StringData(2);level=$record.IntegerData(3)}}
$conditions=@();$view=$database.OpenView('SELECT `Condition`, `Description` FROM `LaunchCondition`');$view.Execute()
while($record=$view.Fetch()){$conditions+=@{condition=$record.StringData(1);description=$record.StringData(2)}}
$options=@();$view=$database.OpenView('SELECT `Dialog_`, `Type`, `Property`, `Text` FROM `Control`');$view.Execute()
while($record=$view.Fetch()){if($record.StringData(1) -eq 'ShortcutOptionsDlg' -and $record.StringData(2) -eq 'CheckBox'){$options+=@{property=$record.StringData(3);text=$record.StringData(4)}}}
@{productVersion=$properties['ProductVersion'];productCode=$properties['ProductCode'];upgradeCode=$properties['UpgradeCode'];productName=$properties['ProductName'];template=$summary.Property(7);perUser=-not $properties['ALLUSERS'];elevationAllowed=([int]$summary.Property(15) -band 8) -eq 0;repairEnabled=$properties['ARPNOREPAIR'] -ne '1' -and $properties['ARPNOREPAIR'] -ne 'yes';modifyEnabled=$properties['ARPNOMODIFY'] -ne '1';features=$features;conditions=$conditions;shortcutOptions=$options}|ConvertTo-Json -Depth 5 -Compress
