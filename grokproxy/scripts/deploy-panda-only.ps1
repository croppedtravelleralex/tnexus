#Requires -Version 5.1
# Panda 上 pull + up（需 Tailscale 连通 panda / 100.69.228.93）
param(
    [string]$Tag = "0638fc0fcdbb3fc5fcd825dd748bda96a744289f"
)

$ErrorActionPreference = "Stop"
$ts = "C:\Program Files\Tailscale\tailscale.exe"
if (Test-Path $ts) {
    & $ts status 2>$null
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Tailscale 未连接。请以管理员启用 Tailscale 服务后重试。"
        Write-Host "  services.msc -> Tailscale -> 启动"
        Write-Host "  或: tailscale up"
    }
}

Write-Host "[*] sync + deploy grokproxy:$Tag on panda"
ssh -o BatchMode=yes panda "bash /root/TNexus/grokproxy/scripts/panda_sync_repo.sh"
ssh -o BatchMode=yes panda "GROKPROXY_TAG=$Tag bash /root/TNexus/deploy/panda/grokproxy-deploy.sh"
ssh -o BatchMode=yes panda "docker inspect grokproxy --format '{{.Config.Image}}'; curl -sf http://127.0.0.1:8110/healthz; echo"
