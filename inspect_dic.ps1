$ErrorActionPreference = 'SilentlyContinue'
$f = 'D:\AVEVA\Everything3D3.1\dicvir.dat'
$i = Get-Item $f
Write-Output ("dicvir.dat size: {0:N2} MB ({1} bytes)" -f ($i.Length/1MB), $i.Length)
$fs = [System.IO.File]::OpenRead($f)
$b = New-Object byte[] 64
[void]$fs.Read($b, 0, 64)
$fs.Close()
Write-Output ("HEX  : " + (($b | ForEach-Object { $_.ToString('X2') }) -join ' '))
Write-Output ("ASCII: " + (-join ($b | ForEach-Object { if ($_ -ge 32 -and $_ -lt 127) { [char]$_ } else { '.' } })))

Write-Output ""
Write-Output "=== occurrences of attribute-name strings in dicvir.dat ==="
foreach ($tok in @('HEIG','POS','WNOEVT','WNOE','NOEV','WNOCLM','GEOMSET','GRAPHIC','WEVENT')) {
    $c = (rg -a -c -F $tok $f 2>$null)
    Write-Output ("  {0,-8} : {1}" -f $tok, ($c | Select-Object -First 1))
}

Write-Output ""
Write-Output "=== sample readable strings (>=5 chars) from first 4000 bytes ==="
rg -a -o "[A-Za-z]{5,}" $f 2>$null | Select-Object -First 40
