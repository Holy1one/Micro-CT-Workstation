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
@{productVersion=$properties['ProductVersion'];productCode=$properties['ProductCode'];upgradeCode=$properties['UpgradeCode'];productName=$properties['ProductName'];template=$summary.Property(7)}|ConvertTo-Json -Compress
