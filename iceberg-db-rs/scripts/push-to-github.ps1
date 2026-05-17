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
$branch = gh repo view "$user/iceberg-db" --json defaultBranchRef -q .defaultBranchRef.name
$dest = Join-Path $RepoRoot "iceberg-db-rs"

if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
New-Item -ItemType Directory -Path $dest | Out-Null

robocopy $RsSrc $dest /E /XD target .git /XF build.log _git_check.txt pat.txt *.rs.bk .env /NFL /NDL /NJH /NJS | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy failed with exit $LASTEXITCODE" }

$gi = Join-Path $dest ".gitignore"
$giContent = if (Test-Path $gi) { Get-Content $gi -Raw } else { "" }
if ($giContent -notmatch '(?m)^/target/') {
    Add-Content -Path $gi -Value "`n/target/`n"
}

git add iceberg-db-rs/
$status = git status --porcelain
if (-not $status) {
    Write-Host "Nothing to commit (already up to date)."
} else {
    git commit -m @"
Add Rust iceberg-db-rs with Snowflake Horizon IRC support

Native SQL engine (DataFusion + iceberg-rust), YAML config compatible with Java iceberg-db,
Snowflake Horizon PAT OAuth (client_id + PAT), vended S3 credentials, and HTTP debug logging.
"@
    git pull --rebase origin $branch
    git push origin "HEAD:$branch"
}

$sha = git rev-parse HEAD
$url = gh repo view "$user/iceberg-db" --json url -q .url
Write-Host ""
Write-Host "Done."
Write-Host "  Repo:   $url"
Write-Host "  Branch: $branch"
Write-Host "  Commit: $sha"
