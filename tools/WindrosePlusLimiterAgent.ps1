<#
.SYNOPSIS
    Out-of-process release agent for the Windrose+ Idle CPU Limiter.

.DESCRIPTION
    The Idle CPU Limiter DLL caps the Windrose dedicated server process to a
    fraction of a CPU core while the server is idle. Its in-process release
    mechanisms (log parse, server_status.json) run inside the capped job, so
    when a player connects the release signal arrives slowly enough that the
    client times out mid-handshake.

    This agent runs outside the job at normal priority, watches for inbound
    TCP connections on the game port, and toggles the DLL's existing disable
    sentinel to lift the cap as soon as a client starts connecting. After
    GraceSeconds of no network activity, the sentinel is removed so the DLL
    can clamp again during true idle.

    No DLL changes are required — the agent only writes a file that the
    current DLL already reads every tick.

.PARAMETER GamePort
    Windrose dedicated-server port to monitor (UDP for game traffic, but we
    also catch TCP handshake activity on handshake-adjacent ports).

.PARAMETER DataDir
    Absolute path to the server's windrose_plus_data directory. The agent
    writes idle_cpu_limiter_disabled inside this directory.

.PARAMETER GraceSeconds
    How long to keep the cap lifted after the last detected connection
    attempt. Default matches the DLL's internal grace window.

.PARAMETER PollIntervalMs
    How often to poll Get-NetTCPConnection. 500ms is fast enough to catch
    a handshake before the default 30s client timeout.

.EXAMPLE
    pwsh -File WindrosePlusLimiterAgent.ps1 `
        -GamePort 7777 `
        -DataDir "C:\WindroseServer\windrose_plus_data"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][int]$GamePort,
    [Parameter(Mandatory=$true)][string]$DataDir,
    [int]$GraceSeconds = 180,
    [int]$PollIntervalMs = 500,
    [string]$ServerExeName = "WindroseServer-Win64-Shipping.exe",
    [switch]$LogToConsole
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $DataDir)) {
    throw "DataDir does not exist: $DataDir"
}

$sentinel = Join-Path $DataDir "idle_cpu_limiter_disabled"
$sentinelOwnerTag = "WindrosePlusLimiterAgent"
$graceUntil = [DateTime]::MinValue
$sentinelHeld = $false

function Write-AgentLog {
    param([string]$Message)
    if ($LogToConsole) {
        Write-Host "[$(Get-Date -Format 'HH:mm:ss')] $Message"
    }
}

function Test-SentinelIsOurs {
    if (-not (Test-Path -LiteralPath $sentinel)) { return $false }
    try {
        return ((Get-Content -LiteralPath $sentinel -Raw -ErrorAction Stop) -match [regex]::Escape($sentinelOwnerTag))
    } catch { return $false }
}

function Set-Sentinel {
    $body = "$sentinelOwnerTag released cap at $(Get-Date -Format 'o') for ${GraceSeconds}s after TCP activity on port $GamePort"
    Set-Content -LiteralPath $sentinel -Value $body -Force
    Write-AgentLog "cap lifted via sentinel"
}

function Clear-Sentinel {
    # Only clear our own sentinel — never stomp a disable placed by config.
    if (Test-SentinelIsOurs) {
        Remove-Item -LiteralPath $sentinel -Force -ErrorAction SilentlyContinue
        Write-AgentLog "cap restored (sentinel removed)"
    }
}

function Get-InboundConnectionCount {
    # SYN_RECEIVED and ESTABLISHED cover the arrival of a client connection.
    try {
        $conns = Get-NetTCPConnection -LocalPort $GamePort -State SynReceived,Established -ErrorAction Stop
        return ($conns | Measure-Object).Count
    } catch {
        return 0
    }
}

function Get-ServerProcess {
    try {
        return Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($ServerExeName)) -ErrorAction Stop | Select-Object -First 1
    } catch {
        return $null
    }
}

Write-AgentLog "agent started: port=$GamePort data=$DataDir grace=${GraceSeconds}s poll=${PollIntervalMs}ms"

# If a prior run left our sentinel in place, clear it on startup.
if (Test-SentinelIsOurs) { Clear-Sentinel }

$lastConnCount = 0

while ($true) {
    $serverProc = Get-ServerProcess
    if (-not $serverProc) {
        # Server not running — nothing to protect. Clean up and exit.
        if ($sentinelHeld) { Clear-Sentinel }
        Write-AgentLog "server process not found; agent exiting"
        break
    }

    $connCount = Get-InboundConnectionCount
    $now = Get-Date

    # New inbound activity — refresh grace window.
    if ($connCount -gt 0 -or $connCount -ne $lastConnCount) {
        if ($connCount -gt 0) {
            $graceUntil = $now.AddSeconds($GraceSeconds)
            if (-not $sentinelHeld) {
                Set-Sentinel
                $sentinelHeld = $true
            }
        }
    }
    $lastConnCount = $connCount

    # Grace expired — release the cap back to the DLL.
    if ($sentinelHeld -and $now -ge $graceUntil) {
        Clear-Sentinel
        $sentinelHeld = $false
    }

    # External disable (customer toggled it off via config) — back off.
    if ((Test-Path -LiteralPath $sentinel) -and -not (Test-SentinelIsOurs)) {
        $sentinelHeld = $false
    }

    Start-Sleep -Milliseconds $PollIntervalMs
}
