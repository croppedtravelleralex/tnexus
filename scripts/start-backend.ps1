# 在 WSL 中一键启动 API + Worker（前端仍需单独 npm run dev）
wsl -e bash -lc "cd /mnt/d/SelfMadeTool/TNexus && sed -i 's/\r$//' .env 2>/dev/null; set -a && source ./.env && set +a && ./target/debug/tnexus-api & ./target/debug/tnexus-worker & wait"
