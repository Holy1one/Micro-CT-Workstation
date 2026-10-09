# Owns protected local signing material. Secrets never enter argv or output.
param([ValidateSet('Initialize','Sign')][string]$Action='Initialize',[string]$File,[string]$Version)
$ErrorActionPreference='Stop'
$releaseRoot=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$keyDirectory=Join-Path $env:LOCALAPPDATA 'MicroCTReleaseKeys'
$keyPath=Join-Path $keyDirectory 'microct-updater.key'
$publicPath="$keyPath.pub"
$passwordPath=Join-Path $keyDirectory 'microct-updater-password.dpapi'
$sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User
function Set-PrivateAcl([string]$Path,[bool]$Directory) {
  $aclTool=Join-Path $env:SystemRoot 'System32/icacls.exe'
  $rights=if($Directory){'(OI)(CI)F'}else{'F'}
  $null=& $aclTool $Path '/inheritance:r' '/grant:r' "*$($sid.Value):$rights" "*S-1-5-18:$rights"
  if($LASTEXITCODE -ne 0){throw 'Cannot restrict signing material ACL'}
  foreach($existingRule in @((Get-Acl -LiteralPath $Path).Access)){
    $identity=$existingRule.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value
    if($identity -ne $sid.Value -and $identity -ne 'S-1-5-18'){
      $null=& $aclTool $Path '/remove' "*$identity"
      if($LASTEXITCODE -ne 0){throw 'Cannot remove an unrelated signing-material ACL principal'}
    }
  }
}
function Run-Signer([string[]]$Arguments,[string]$InputPassword,[bool]$Generate) {
  $start=[System.Diagnostics.ProcessStartInfo]::new()
  $start.FileName='node.exe'
  $start.WorkingDirectory=$releaseRoot
  $start.UseShellExecute=$false
  $start.CreateNoWindow=$true
  $start.RedirectStandardOutput=$true
  $start.RedirectStandardError=$true
  $start.RedirectStandardInput=$true
  if($Generate){
    $start.ArgumentList.Add((Join-Path $releaseRoot 'tools/release/update-keygen-child.mjs'))
    $start.ArgumentList.Add($keyPath)
    $start.Environment['MICROCT_KEYGEN_PASSWORD']=$InputPassword
  }else{
    $start.ArgumentList.Add((Join-Path $releaseRoot 'node_modules/@tauri-apps/cli/tauri.js'))
    foreach($argument in $Arguments){$start.ArgumentList.Add($argument)}
  }
  $start.Environment.Remove('CI')|Out-Null
  $start.Environment.Remove('TAURI_SIGNING_PRIVATE_KEY')|Out-Null
  $start.Environment.Remove('TAURI_SIGNING_PRIVATE_KEY_PATH')|Out-Null
  $start.Environment.Remove('TAURI_SIGNING_PRIVATE_KEY_PASSWORD')|Out-Null
  if(!$Generate){$start.Environment['TAURI_SIGNING_PRIVATE_KEY_PASSWORD']=$InputPassword}
  $child=[System.Diagnostics.Process]::new();$child.StartInfo=$start
  try {
    if(!$child.Start()){throw 'Cannot start signing tool'}
    $stdout=$child.StandardOutput.ReadToEndAsync();$stderr=$child.StandardError.ReadToEndAsync()
    $child.StandardInput.Close()
    $child.WaitForExit()
    $null=$stdout.GetAwaiter().GetResult();$null=$stderr.GetAwaiter().GetResult()
    if($child.ExitCode -ne 0){throw "Signing tool failed (exit $($child.ExitCode)); captured output suppressed to protect signing material"}
  }finally{$start.Environment.Remove('TAURI_SIGNING_PRIVATE_KEY_PASSWORD')|Out-Null;$start.Environment.Remove('MICROCT_KEYGEN_PASSWORD')|Out-Null;$child.Dispose()}
}
if(!(Test-Path -LiteralPath $keyDirectory)){New-Item -ItemType Directory -Path $keyDirectory|Out-Null}
Set-PrivateAcl $keyDirectory $true
$present=@(@($keyPath,$publicPath,$passwordPath)|Where-Object{Test-Path -LiteralPath $_})
if($present.Count -gt 0 -and $present.Count -ne 3){
  if($Action -eq 'Initialize' -and $present.Count -eq 1 -and $present[0] -eq $passwordPath){
    $recovered=[System.Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($passwordPath),$null,[System.Security.Cryptography.DataProtectionScope]::CurrentUser)
    $password=[Text.Encoding]::UTF8.GetString($recovered)
    try{Run-Signer @('signer','generate','-w',$keyPath) $password $true}finally{$password=$null;[Array]::Clear($recovered,0,$recovered.Length)}
  }else{throw 'Partial signing material exists; restore the matching files. Existing material is preserved.'}
}
if($present.Count -eq 0){
  if($Action -ne 'Initialize'){throw 'Initialize signing material first'}
  $random=[byte[]]::new(48);[System.Security.Cryptography.RandomNumberGenerator]::Fill($random)
  $password=[Convert]::ToBase64String($random)
  $protected=[System.Security.Cryptography.ProtectedData]::Protect([Text.Encoding]::UTF8.GetBytes($password),$null,[System.Security.Cryptography.DataProtectionScope]::CurrentUser)
  # Save the recoverable encrypted password before generating; never replace it.
  $stream=[IO.File]::Open($passwordPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
  try{$stream.Write($protected);$stream.Flush($true)}finally{$stream.Dispose()}
  Set-PrivateAcl $passwordPath $false
  try{Run-Signer @('signer','generate','-w',$keyPath) $password $true}finally{$password=$null;$random=$null;$protected=$null}
}
foreach($path in @($keyPath,$publicPath,$passwordPath)){if(!(Test-Path -LiteralPath $path)){throw 'Signing material is incomplete'};Set-PrivateAcl $path $false}
$public=[IO.File]::ReadAllText($publicPath).Trim()
$decoded=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($public))
if($decoded -notmatch '^untrusted comment: ' -or ($decoded -split "`n").Count -lt 2){throw 'Public key format is invalid'}
$tracked=Join-Path $releaseRoot 'backend/src-tauri/update-public-key.pub'
if(Test-Path -LiteralPath $tracked){if([IO.File]::ReadAllText($tracked).Trim() -ne $public){throw 'Tracked public key differs; refusing a trust-root replacement'}}else{[IO.File]::WriteAllText($tracked,$public+"`n",[Text.UTF8Encoding]::new($false))}
if($Action -eq 'Sign'){
  if(!$File -or !$Version){throw 'Sign requires File and Version'}
  $signPath=(Resolve-Path -LiteralPath $File).Path
  $allowed=Join-Path $releaseRoot ".cache/tests/release/$Version"
  if(!$signPath.StartsWith($allowed+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Only this version build evidence directory may be signed'}
  $passwordBytes=[System.Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($passwordPath),$null,[System.Security.Cryptography.DataProtectionScope]::CurrentUser)
  $password=[Text.Encoding]::UTF8.GetString($passwordBytes)
  try{Run-Signer @('signer','sign','--private-key-path',$keyPath,'--app-version',$Version,$signPath) $password $false}finally{$password=$null;[Array]::Clear($passwordBytes,0,$passwordBytes.Length)}
}
Write-Output "PASS protected signing material validated; action=$Action"
