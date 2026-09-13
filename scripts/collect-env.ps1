# Windows 정보 수집. 출력 전체를 복사해서 전달하면 된다.
Write-Output "=== collect-env (windows) ==="
Write-Output ("date: " + (Get-Date).ToString("o"))
Write-Output ("os: " + (Get-CimInstance Win32_OperatingSystem).Caption + " " + (Get-CimInstance Win32_OperatingSystem).Version)

foreach ($c in @("rustc", "cargo")) {
  $v = & $c -V 2>$null
  if ($LASTEXITCODE -eq 0) { Write-Output ("{0}: {1}" -f $c, $v) } else { Write-Output ("{0}: MISSING" -f $c) }
}

Write-Output "--- 입력기(IME) ---"
Get-WinUserLanguageList | ForEach-Object { Write-Output ("lang: " + $_.LanguageTag + " | input methods: " + (($_.InputMethodTips) -join ", ")) }

Write-Output "--- GPU ---"
Get-CimInstance Win32_VideoController | ForEach-Object { Write-Output ($_.Name + " | driver " + $_.DriverVersion) }

Write-Output "--- DPI ---"
Add-Type -AssemblyName System.Windows.Forms
Write-Output ("primary screen: " + [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.ToString())
Write-Output "=== end ==="
