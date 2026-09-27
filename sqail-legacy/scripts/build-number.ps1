# Generates/increments build number in yyyymmdd-revision format.
# Stores state in build-number.json at project root.

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$BuildFile = Join-Path $ProjectRoot "build-number.json"
$Today = Get-Date -Format "yyyyMMdd"

if (Test-Path $BuildFile) {
    $Content = Get-Content $BuildFile | ConvertFrom-Json
    $PrevDate = $Content.date
    $PrevRev = $Content.revision
    
    if ($PrevDate -eq $Today) {
        $Revision = $PrevRev + 1
    } else {
        $Revision = 1
    }
} else {
    $Revision = 1
}

$BuildData = @{
    date = $Today
    revision = $Revision
    buildNumber = "$Today-$Revision"
}

$BuildData | ConvertTo-Json | Set-Content $BuildFile

Write-Output "$Today-$Revision"
