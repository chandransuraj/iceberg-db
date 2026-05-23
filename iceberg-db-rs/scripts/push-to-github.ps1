# Sync iceberg-db-rs into GitHub repo iceberg-db and push.
# Run from PowerShell: .\scripts\push-to-github.ps1

$ErrorActionPreference = "Stop"

$RsSrc = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$RepoPrimary = Join-Path (Split-Path $RsSrc -Parent) "iceberg-db"
$RepoClone = Join-Path (Split-Path $RsSrc -Parent) "iceberg-db-git"

$user = gh api user -q .login
Write-Host "GitHub user: $user"
gh auth status

$null = gh repo view "$user/iceberg-db" --json url 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "Repo $user/iceberg-db not found or gh not authenticated. Run: gh auth login"
}

if (Test-Path (Join-Path $RepoPrimary ".git")) {
    $RepoRoot = $RepoPrimary
} else {
    if (-not (Test-Path (Join-Path $RepoClone ".git"))) {
        gh repo clone "https://github.com/$user/iceberg-db.git" $RepoClone
    }
    $RepoRoot = $RepoClone
}

Set-Location $RepoRoot

$branch = gh repo view "$user/iceberg-db" --json defaultBranchRef -q .defaultBranchRef.name 2>$null
if (-not $branch) { $branch = "main" }

$currentBranch = git branch --show-current 2>$null
if (-not $currentBranch) {
    git checkout -b $branch 2>$null
} elseif ($currentBranch -ne $branch) {
    git branch -M $branch 2>$null
}

$userId = gh api user -q .id
$noreplyEmail = "${userId}+${user}@users.noreply.github.com"
$gitName = git config --global user.name
if (-not $gitName) { $gitName = $user }
git config user.email $noreplyEmail
git config user.name $gitName
Write-Host "Git author for this repo: $gitName <$noreplyEmail>"

$dest = Join-Path $RepoRoot "iceberg-db-rs"
if (-not (Test-Path $dest)) {
    New-Item -ItemType Directory -Path $dest | Out-Null
}

Write-Host "Syncing files (robocopy mirror; excludes target/, web-wasm/dist/) ..."
$robolog = Join-Path $env:TEMP "idb-push-robocopy.log"
robocopy $RsSrc $dest /MIR /XD target .git "web-wasm\dist" /XF build.log _git_check.txt pat.txt *.rs.bk .env /NFL /NDL /NJH /NJS /NP | Tee-Object -FilePath $robolog
if ($LASTEXITCODE -ge 8) { throw "robocopy failed with exit $LASTEXITCODE (see $robolog)" }
Write-Host "Sync done (robocopy exit $LASTEXITCODE; 0-7 = success)."

$gi = Join-Path $dest ".gitignore"
$giLines = @()
if (Test-Path $gi) { $giLines = Get-Content $gi }
foreach ($line in @("/target/", "web-wasm/dist/")) {
    if ($giLines -notcontains $line) { Add-Content -Path $gi -Value $line }
}

Write-Host "Staging changes ..."
git add iceberg-db-rs/

$status = git status --porcelain
if ($status) {
    Write-Host "Committing ..."
    $commitMsg = @"
Fix browser WASM queries against Snowflake Horizon Iceberg tables

Run Horizon SQL in the browser via idb-wasm: PAT OAuth through idb-sf-proxy, lazy Iceberg
catalog loading, S3 reads through signed GET /_s3, and iceberg runtime spawn on wasm32 using
the JS executor so scans complete without a Tokio reactor panic.
"@
    git commit -m $commitMsg
} else {
    Write-Host "No file changes to commit."
}

Write-Host "Pushing to GitHub (fetch/rebase/push may take a minute on slow networks) ..."
$remoteHeads = git ls-remote --heads origin 2>$null
$remoteEmpty = -not $remoteHeads

if ($remoteEmpty) {
    Write-Host "Remote has no branches yet - pushing initial $branch ..."
    git push -u origin $branch
} else {
    git fetch origin $branch 2>$null
    $hasUpstream = git rev-parse --abbrev-ref "@{u}" 2>$null
    if ($hasUpstream) {
        git pull --rebase origin $branch
    }
    git push -u origin $branch
}

$sha = git rev-parse HEAD
$url = gh repo view "$user/iceberg-db" --json url -q .url

Write-Host ""
Write-Host "Done."
Write-Host "  Repo:   $url"
Write-Host "  Branch: $branch"
Write-Host "  Commit: $sha"
