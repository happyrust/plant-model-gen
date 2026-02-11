param(
  [Parameter(Mandatory=$true, Position=0)]
  [Alias("JsonPath")]
  [string]$Path
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Path)) {
  Write-Error "json file not found: $Path"
  exit 1
}

# 逐行归一化后做 SHA256：
# - 去除时间戳字段的差异（generated_at / export_time）
# - 保留其余内容（若排序/内容变动，签名会变化）
$sha = [System.Security.Cryptography.SHA256]::Create()

$counts = @{
  generated_at = 0
  export_time = 0
  refno = 0
  geo_hash = 0
  aabb_hash = 0
  tubings = 0
  instances = 0
  groups = 0
}

Get-Content -LiteralPath $Path | ForEach-Object {
  $line = $_

  if ($line -match '"generated_at"\s*:') { $counts.generated_at++ }
  if ($line -match '"export_time"\s*:')  { $counts.export_time++ }
  if ($line -match '"refno"\s*:')        { $counts.refno++ }
  if ($line -match '"geo_hash"\s*:')     { $counts.geo_hash++ }
  if ($line -match '"aabb_hash"\s*:')    { $counts.aabb_hash++ }
  if ($line -match '"tubings"\s*:')      { $counts.tubings++ }
  if ($line -match '"instances"\s*:')    { $counts.instances++ }
  if ($line -match '"groups"\s*:')       { $counts.groups++ }

  # normalize timestamp fields (line-based; safe for current exporters)
  $line = $line -replace '("generated_at"\s*:\s*)"(.*?)"', '$1"__TS__"'
  $line = $line -replace '("export_time"\s*:\s*)"(.*?)"', '$1"__TS__"'

  $bytes = [System.Text.Encoding]::UTF8.GetBytes($line + "`n")
  $null = $sha.TransformBlock($bytes, 0, $bytes.Length, $bytes, 0)
}

$null = $sha.TransformFinalBlock(@(), 0, 0)
$hash = ($sha.Hash | ForEach-Object { $_.ToString("x2") }) -join ""
$hash = $hash.ToUpperInvariant()

$len = (Get-Item -LiteralPath $Path).Length

Write-Output ("json: {0}" -f $Path)
Write-Output ("  size_bytes : {0}" -f $len)
Write-Output ("  sig.sha256 : {0}" -f $hash)
Write-Output ("  counts     : generated_at={0} export_time={1} refno={2} geo_hash={3} aabb_hash={4} tubings={5} instances={6} groups={7}" -f `
  $counts.generated_at, $counts.export_time, $counts.refno, $counts.geo_hash, $counts.aabb_hash, $counts.tubings, $counts.instances, $counts.groups)

