#!/bin/bash
# ==============================================================================
# JRX AI 1.0 — Ubuntu EC2 Setup & 24/7 Service Deployer
# ==============================================================================
set -e

echo "=== 1. Updating packages and installing prerequisites ==="
sudo apt-get update -y
sudo apt-get install -y build-essential pkg-config libssl-dev git curl tmux

# Install Rust if not installed
if ! command -v cargo &> /dev/null; then
    echo "=== Installing Rust toolchain ==="
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "=== 2. Building JRX Trading Bot (Release mode) ==="
cargo build -p trading-bot --release

echo "=== 3. Creating systemd 24/7 service ==="
SERVICE_FILE="/etc/systemd/system/jrx-bot.service"
USER_NAME=$(whoami)
APP_DIR=$(pwd)

sudo bash -c "cat << EOF > $SERVICE_FILE
[Unit]
Description=JRX AI 1.0 24/7 Cloud Trading Bot Daemon
After=network.target

[Service]
Type=simple
User=$USER_NAME
WorkingDirectory=$APP_DIR
ExecStart=$APP_DIR/target/release/trading-bot
Restart=always
RestartSec=10
EnvironmentFile=$APP_DIR/.env
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF"

sudo systemctl daemon-reload
echo "=== 4. Starting jrx-bot service ==="
sudo systemctl enable jrx-bot
sudo systemctl restart jrx-bot

echo "=============================================================================="
echo "  JRX AI Bot is now RUNNING 24/7 in the background!"
echo "  To view live logs:    journalctl -u jrx-bot -f"
echo "  To check status:      sudo systemctl status jrx-bot"
echo "  To restart:           sudo systemctl restart jrx-bot"
echo "  To stop:              sudo systemctl stop jrx-bot"
echo "  REST API running on:  http://<YOUR-EC2-IP>:8080/api/status"
echo "=============================================================================="
