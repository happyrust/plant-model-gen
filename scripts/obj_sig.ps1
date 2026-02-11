param(
  [Parameter(Mandatory=$true, Position=0)]
  [string]$Path,

  [Parameter(Mandatory=$false)]
  [string]$OutJson
)

if (-not (Test-Path $Path)) {
  Write-Error "obj file not found: $Path"
  exit 1
}

function New-Aabb() {
  return @{
    minX = [double]::PositiveInfinity
    minY = [double]::PositiveInfinity
    minZ = [double]::PositiveInfinity
    maxX = [double]::NegativeInfinity
    maxY = [double]::NegativeInfinity
    maxZ = [double]::NegativeInfinity
  }
}

function Update-Aabb([hashtable]$aabb, [double]$x, [double]$y, [double]$z) {
  if ($x -lt $aabb.minX) { $aabb.minX = $x }
  if ($y -lt $aabb.minY) { $aabb.minY = $y }
  if ($z -lt $aabb.minZ) { $aabb.minZ = $z }
  if ($x -gt $aabb.maxX) { $aabb.maxX = $x }
  if ($y -gt $aabb.maxY) { $aabb.maxY = $y }
  if ($z -gt $aabb.maxZ) { $aabb.maxZ = $z }
}

$globalAabb = New-Aabb
$groups = @()

$cur = @{
  name = "<none>"
  v = 0
  f = 0
  aabb = (New-Aabb)
}

function Flush-Group() {
  if ($null -ne $cur -and ($cur.v -gt 0 -or $cur.f -gt 0 -or $cur.name -ne "<none>")) {
    $groups += @{
      name = $cur.name
      v = $cur.v
      f = $cur.f
      aabb = $cur.aabb
    }
  }
}

Get-Content $Path | ForEach-Object {
  $line = $_
  if ($line -match '^g\s+(.+)\s*$') {
    Flush-Group
    $cur = @{
      name = $matches[1].Trim()
      v = 0
      f = 0
      aabb = (New-Aabb)
    }
    return
  }
  if ($line -match '^v\s+([-\d\.eE]+)\s+([-\d\.eE]+)\s+([-\d\.eE]+)\s*$') {
    $x = [double]$matches[1]
    $y = [double]$matches[2]
    $z = [double]$matches[3]
    $cur.v++
    Update-Aabb $cur.aabb $x $y $z
    Update-Aabb $globalAabb $x $y $z
    return
  }
  if ($line -match '^f\s+') {
    $cur.f++
    return
  }
}

Flush-Group

# 生成稳定签名：按 group name 排序 + AABB 四舍五入，避免浮点字符串差异
$groupsSorted = $groups | Sort-Object name | ForEach-Object {
  $a = $_.aabb
  @{
    name = $_.name
    v = $_.v
    f = $_.f
    aabb = @{
      min = @([math]::Round($a.minX, 6), [math]::Round($a.minY, 6), [math]::Round($a.minZ, 6))
      max = @([math]::Round($a.maxX, 6), [math]::Round($a.maxY, 6), [math]::Round($a.maxZ, 6))
    }
  }
}

$sigForHash = @{
  group_count = ($groupsSorted | Measure-Object).Count
  totals = @{
    v = ($groupsSorted | Measure-Object -Property v -Sum).Sum
    f = ($groupsSorted | Measure-Object -Property f -Sum).Sum
    aabb = @{
      min = @([math]::Round($globalAabb.minX, 6), [math]::Round($globalAabb.minY, 6), [math]::Round($globalAabb.minZ, 6))
      max = @([math]::Round($globalAabb.maxX, 6), [math]::Round($globalAabb.maxY, 6), [math]::Round($globalAabb.maxZ, 6))
    }
  }
  groups = $groupsSorted
}

$json = ($sigForHash | ConvertTo-Json -Depth 12)

if ($OutJson) {
  $dir = Split-Path -Parent $OutJson
  if ($dir -and -not (Test-Path $dir)) {
    New-Item -ItemType Directory -Path $dir | Out-Null
  }
  Set-Content -Path $OutJson -Value $json -Encoding UTF8
}

$tmp = [System.IO.Path]::GetTempFileName()
Set-Content -Path $tmp -Value $json -Encoding UTF8
$hash = (Get-FileHash -Algorithm SHA256 $tmp).Hash
Remove-Item $tmp -ErrorAction SilentlyContinue

Write-Output ("obj: {0}" -f $Path)
Write-Output ("  group_count : {0}" -f $sigForHash.group_count)
Write-Output ("  totals.v    : {0}" -f $sigForHash.totals.v)
Write-Output ("  totals.f    : {0}" -f $sigForHash.totals.f)
Write-Output ("  totals.aabb : min=({0},{1},{2}) max=({3},{4},{5})" -f $sigForHash.totals.aabb.min[0],$sigForHash.totals.aabb.min[1],$sigForHash.totals.aabb.min[2],$sigForHash.totals.aabb.max[0],$sigForHash.totals.aabb.max[1],$sigForHash.totals.aabb.max[2])
Write-Output ("  sig.sha256  : {0}" -f $hash)

if ($OutJson) {
  Write-Output ("  sig.json    : {0}" -f $OutJson)
}
