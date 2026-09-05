[CmdletBinding()]
param(
    [switch]$SkipWebBundle,
    [switch]$SkipCompose
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Label,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command
    )

    Write-Host "`n==> $Label" -ForegroundColor Cyan
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed with exit code $LASTEXITCODE"
    }
}

Push-Location $repositoryRoot
try {
    Invoke-Checked "Rust formatting" { cargo fmt --all --check }
    Invoke-Checked "Workspace clippy" {
        cargo clippy --workspace --all-targets -- -D warnings
    }
    Invoke-Checked "Workspace tests" { cargo test --workspace }
    Invoke-Checked "Web-target Rust check" {
        cargo check -p features-courses --target wasm32-unknown-unknown
    }

    if (-not $SkipWebBundle) {
        if (-not (Get-Command dx -ErrorAction SilentlyContinue)) {
            throw "Dioxus CLI is required. Install the repository version with: cargo install dioxus-cli --version 0.7.9 --locked"
        }
        $dxVersion = (& dx --version 2>&1 | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or $dxVersion -notmatch '^dioxus 0\.7\.9(?:\s|$)') {
            throw "Dioxus CLI 0.7.9 is required; found '$dxVersion'. Install it with: cargo install dioxus-cli --version 0.7.9 --locked --force"
        }
        Invoke-Checked "Production web bundle" {
            dx build --package shell-web --platform web --release
        }
    }

    if (-not $SkipCompose) {
        if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
            throw "Docker Compose is required to validate the production deployment model"
        }

        # Safe, inert values used only for Compose interpolation. They never
        # enter the developer .env and are restored after validation.
        $composeEnvironment = @{
            APP_ORIGIN                         = "https://app.ci.example.test"
            SELF_SERVICE_SIGNUP_ENABLED        = "true"
            POSTGRES_PASSWORD                  = "ci-owner-password"
            POSTGRES_RUNTIME_PASSWORD          = "ci-runtime-password"
            MIGRATION_DATABASE_URL             = "postgresql://aulalite:ci-owner-password@postgres:5432/aulalite"
            DATABASE_URL                       = "postgresql://aulalite_app:ci-runtime-password@postgres:5432/aulalite"
            S3_ENDPOINT_URL                    = "https://storage.ci.example.test"
            AWS_ACCESS_KEY_ID                  = "ci-object-store-access"
            AWS_SECRET_ACCESS_KEY              = "ci-object-store-secret"
            FIREBASE_PROJECT_ID                = "ci-firebase-project"
            FIREBASE_TOKEN_ISSUER              = "https://securetoken.google.com/ci-firebase-project"
            FIREBASE_WEB_API_KEY               = "ci-publishable-web-key"
            FIREBASE_AUTH_DOMAIN               = "ci-firebase-project.firebaseapp.com"
            JWT_RS256_PRIVATE_KEY_PEM           = "ci-not-a-real-private-key"
            MEDIAMTX_PUBLIC_WEBRTC_URL          = "https://live.ci.example.test"
            MEDIAMTX_PUBLIC_HLS_URL             = "https://stream.ci.example.test"
            MEDIAMTX_ADDITIONAL_HOSTS           = "192.0.2.10"
            MEDIAMTX_AUTH_SHARED_HEADER         = "ci-media-auth-secret"
            STRIPE_SECRET_KEY                   = "sk_test_ci_only"
            STRIPE_WEBHOOK_SECRET               = "whsec_ci_only"
            RESEND_API_KEY                      = "re_ci_only_not_a_real_key"
            RESEND_FROM                         = "AulaLite CI <notifications@ci.example.test>"
            SSO_SESSION_SECRET                  = "ci-only-session-secret-at-least-32-characters"
            AULALITE_DATA_ENCRYPTION_KEY        = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        }
        $previousEnvironment = @{}
        foreach ($entry in $composeEnvironment.GetEnumerator()) {
            $previousEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable(
                $entry.Key,
                "Process"
            )
            [Environment]::SetEnvironmentVariable(
                $entry.Key,
                $entry.Value,
                "Process"
            )
        }

        try {
            Invoke-Checked "Production Compose overlay" {
                docker compose --env-file .env.example `
                    -f docker-compose.yml -f docker-compose.prod.yml `
                    config --quiet
            }
            Invoke-Checked "Standalone Dokploy Compose" {
                docker compose --env-file ops/dokploy.env.example `
                    -f docker-compose.dokploy.yml config --quiet
            }
        }
        finally {
            foreach ($entry in $previousEnvironment.GetEnumerator()) {
                [Environment]::SetEnvironmentVariable(
                    $entry.Key,
                    $entry.Value,
                    "Process"
                )
            }
        }
    }

    Write-Host "`nRelease preflight passed." -ForegroundColor Green
}
finally {
    Pop-Location
}
