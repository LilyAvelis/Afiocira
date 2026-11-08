#requires -Version 7.0
[CmdletBinding()]
param(
    [string]$StageDirectory,
    [Parameter(Mandatory)][string]$AssetsDirectory,
    [string]$ExpectedCommit,
    [Parameter(Mandatory)][ValidatePattern('\A[0-9]+\.[0-9]+\.[0-9]+\z')][string]$Version
)
$ErrorActionPreference = 'Stop'
if (-not $ExpectedCommit) {
    $ExpectedCommit = & git -C (Join-Path $PSScriptRoot '..') rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot determine the expected source commit.' }
}
if ($ExpectedCommit -notmatch '\A[a-fA-F0-9]{40}\z') { throw 'ExpectedCommit must be a full commit SHA.' }
$assets = (Resolve-Path -LiteralPath $AssetsDirectory).Path
$zipName = "Afiocira-$Version-Windows-x64.zip"
$installerName = "Afiocira-$Version-Setup-Windows-x64.exe"
$expectedAssets = @($zipName, $installerName, 'SHA256SUMS.txt')
$actualAssets = @(Get-ChildItem -LiteralPath $assets -Force)
if ($actualAssets.Count -ne 3 -or @($actualAssets | Where-Object { $_.PSIsContainer -or $_.Name -cnotin $expectedAssets }).Count) {
    throw 'Release assets must contain exactly the installer, portable ZIP and SHA256SUMS.txt.'
}
$checksums = @{}
foreach ($line in Get-Content -LiteralPath (Join-Path $assets 'SHA256SUMS.txt')) {
    if ($line -notmatch '^([a-fA-F0-9]{64})  ([^/\\]+)$') { throw 'Invalid SHA256SUMS.txt line.' }
    $name = $Matches[2]
    if ($name -cnotin @($zipName, $installerName) -or $checksums.ContainsKey($name)) { throw 'Unexpected or duplicated checksum filename.' }
    $checksums[$name] = $Matches[1]
}
if ($checksums.Count -ne 2) { throw 'SHA256SUMS.txt must describe exactly two assets.' }
foreach ($name in @($zipName, $installerName)) {
    if ((Get-FileHash -LiteralPath (Join-Path $assets $name) -Algorithm SHA256).Hash -ne $checksums[$name]) { throw "Asset SHA256 mismatch: $name" }
}

if (-not ('AfiociraReleaseCheck' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Security.Cryptography;
public sealed class AfiociraReleaseCheck {
    static readonly uint[] Table = MakeTable();
    static uint[] MakeTable() {
        var table = new uint[256];
        for (uint i = 0; i < 256; i++) { uint c = i; for (int j = 0; j < 8; j++) c = (c & 1) != 0 ? 0xedb88320U ^ (c >> 1) : c >> 1; table[i] = c; }
        return table;
    }
    public static uint[] ReadCrcs(string path) {
        using (var stream = File.OpenRead(path)) using (var reader = new BinaryReader(stream)) {
            int length = (int)Math.Min(stream.Length, 65557);
            stream.Position = stream.Length - length; var tail = reader.ReadBytes(length);
            int end = -1;
            for (int i = length - 22; i >= 0; i--) {
                if (BitConverter.ToUInt32(tail, i) == 0x06054b50 && i + 22 + BitConverter.ToUInt16(tail, i + 20) == length) { end = i; break; }
            }
            if (end < 0) throw new InvalidDataException("ZIP end record missing.");
            if (BitConverter.ToUInt16(tail, end + 4) != 0 || BitConverter.ToUInt16(tail, end + 6) != 0) throw new InvalidDataException("Multi-disk ZIP unsupported.");
            int count = BitConverter.ToUInt16(tail, end + 10);
            uint size = BitConverter.ToUInt32(tail, end + 12), offset = BitConverter.ToUInt32(tail, end + 16);
            if (count == 65535 || offset == uint.MaxValue || size == uint.MaxValue) throw new InvalidDataException("ZIP64 unsupported for this release format.");
            if (BitConverter.ToUInt16(tail, end + 8) != count || (long)offset + size != stream.Length - length + end) throw new InvalidDataException("Invalid ZIP directory bounds.");
            stream.Position = offset; var crcs = new uint[count];
            for (int i = 0; i < count; i++) {
                if (reader.ReadUInt32() != 0x02014b50) throw new InvalidDataException("Invalid ZIP directory record.");
                stream.Position += 12; crcs[i] = reader.ReadUInt32(); stream.Position += 8;
                int name = reader.ReadUInt16(), extra = reader.ReadUInt16(), comment = reader.ReadUInt16();
                stream.Position += 12L + name + extra + comment;
            }
            if (stream.Position != (long)offset + size) throw new InvalidDataException("ZIP directory size mismatch.");
            return crcs;
        }
    }
    public string Sha256; public long Length;
    public static AfiociraReleaseCheck Read(Stream stream, uint expectedCrc, byte[][] privatePaths) {
        uint crc = 0xffffffff; long length = 0; int maxPattern = 1;
        foreach (var pattern in privatePaths) maxPattern = Math.Max(maxPattern, pattern.Length);
        var buffer = new byte[65536 + maxPattern]; int carry = 0, count;
        using (var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256)) {
            while ((count = stream.Read(buffer, carry, 65536)) > 0) {
                hash.AppendData(buffer, carry, count); length += count;
                for (int i = carry; i < carry + count; i++) crc = Table[(crc ^ buffer[i]) & 255] ^ (crc >> 8);
                foreach (var pattern in privatePaths) if (buffer.AsSpan(0, carry + count).IndexOf(pattern.AsSpan()) >= 0) throw new InvalidDataException("Private build path in first-party executable.");
                int total = carry + count; carry = Math.Min(maxPattern - 1, total);
                Buffer.BlockCopy(buffer, total - carry, buffer, 0, carry);
            }
            if ((crc ^ 0xffffffff) != expectedCrc) throw new InvalidDataException("ZIP entry CRC32 mismatch.");
            return new AfiociraReleaseCheck { Sha256 = BitConverter.ToString(hash.GetHashAndReset()).Replace("-", "").ToLowerInvariant(), Length = length };
        }
    }
}
'@
}
function Normalize-ReleasePath([string]$Path) {
    $normalized = $Path.Replace('\', '/')
    if (-not $normalized -or $normalized -match '(^/|:|(^|/)\.\.?(/|$)|//|\x00)') { throw "Unsafe package path: $Path" }
    return $normalized
}
$privatePaths = [Collections.Generic.List[byte[]]]::new()
$workspace = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
foreach ($path in @($workspace, $env:GITHUB_WORKSPACE, $env:USERPROFILE) | Where-Object { $_ } | Select-Object -Unique) {
    foreach ($variant in @($path, $path.Replace('\', '/')) | Select-Object -Unique) {
        $privatePaths.Add([Text.Encoding]::UTF8.GetBytes($variant))
        $privatePaths.Add([Text.Encoding]::Unicode.GetBytes($variant))
    }
}
$ownExecutables = @('Afiocira.exe', 'photo-cli.exe', 'AfioLauncher.exe')
$zipPath = Join-Path $assets $zipName
$crcs = [AfiociraReleaseCheck]::ReadCrcs($zipPath)
$zip = [IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    if ($zip.Entries.Count -ne $crcs.Length) { throw 'ZIP directory entry count mismatch.' }
    $entries = [Collections.Generic.Dictionary[string, object]]::new([StringComparer]::OrdinalIgnoreCase)
    $allNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    for ($i = 0; $i -lt $zip.Entries.Count; $i++) {
        $entry = $zip.Entries[$i]
        $name = Normalize-ReleasePath $entry.FullName
        if (-not $allNames.Add($name.TrimEnd('/'))) { throw "Duplicated ZIP entry: $name" }
        if ($name.EndsWith('/')) { if ($entry.Length -ne 0 -or $crcs[$i] -ne 0) { throw 'Invalid ZIP directory entry.' }; continue }
        $patterns = [byte[][]]@()
        if ($name -cin $ownExecutables) { $patterns = $privatePaths.ToArray() }
        $stream = $entry.Open()
        try { $result = [AfiociraReleaseCheck]::Read($stream, $crcs[$i], [byte[][]]$patterns) } finally { $stream.Dispose() }
        if ($result.Length -ne $entry.Length) { throw "ZIP entry size mismatch: $name" }
        $entries.Add($name, [pscustomobject]@{ Entry = $entry; Hash = $result.Sha256; Bytes = $result.Length })
    }
    foreach ($name in @($ownExecutables) + @('PACKAGE-MANIFEST.json', 'README.md', 'CHANGELOG.md', 'LICENSE', 'current-version.txt')) {
        if ($name -cnotin $entries.Keys) { throw "Required package file missing or incorrectly cased: $name" }
    }
    $reader = [IO.StreamReader]::new($entries['PACKAGE-MANIFEST.json'].Entry.Open())
    try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    if ($manifest.version -cne $Version -or $manifest.source_commit -ne $ExpectedCommit) { throw 'Package version or source commit does not match this release.' }
    $manifestNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $manifest.files) {
        $name = Normalize-ReleasePath $file.path
        if ($name -ieq 'PACKAGE-MANIFEST.json' -or -not $manifestNames.Add($name)) { throw "Invalid or duplicated manifest entry: $name" }
        if ($file.sha256 -notmatch '^[a-fA-F0-9]{64}$' -or $null -eq $file.bytes -or $file.bytes -lt 0 -or $name -cnotin $entries.Keys) { throw "Invalid manifest file: $name" }
        if ($entries[$name].Hash -ne $file.sha256 -or $entries[$name].Bytes -ne $file.bytes) { throw "Manifest hash or size mismatch: $name" }
    }
    if ($manifestNames.Count -ne $entries.Count - 1) { throw 'ZIP contains files missing from the manifest.' }
    if ($StageDirectory) {
        $stage = (Resolve-Path -LiteralPath $StageDirectory).Path
        $stageFiles = @(Get-ChildItem -LiteralPath $stage -Recurse -File -Force)
        if ($stageFiles.Count -ne $entries.Count) { throw 'Stage and ZIP file counts differ.' }
        foreach ($file in $stageFiles) {
            $name = Normalize-ReleasePath ([IO.Path]::GetRelativePath($stage, $file.FullName))
            if (-not $entries.ContainsKey($name) -or $entries[$name].Bytes -ne $file.Length -or $entries[$name].Hash -ne (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash) { throw "Stage differs from ZIP: $name" }
        }
    }
    Write-Host "Verified Afiocira $Version ($($entries.Count) package files), installer, checksums and source $ExpectedCommit."
} finally { $zip.Dispose() }
