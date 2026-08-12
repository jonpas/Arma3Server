#!/bin/bash

# Relaunch headless clients inside the container
# This script runs OUTSIDE the container and executes HC scripts inside

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Get the container name from docker compose
CONTAINER_NAME=$(docker compose -f "$SCRIPT_DIR/docker-compose.yml" ps -q arma3 2>/dev/null)

if [ -z "$CONTAINER_NAME" ]; then
    echo "Error: Container is not running. Start the server first."
    exit 1
fi

# Find and execute all hc_command_*.sh scripts inside the container
HC_SCRIPTS=$(docker exec "$CONTAINER_NAME" bash -c 'ls /arma3/server/hc_command_*.sh 2>/dev/null | sort -V')

if [ -z "$HC_SCRIPTS" ]; then
    echo "No HC command scripts found in container."
    echo "Make sure the server was started with HEADLESS_CLIENTS > 0"
    exit 1
fi

echo "Found HC scripts:"
echo "$HC_SCRIPTS"
echo ""

for script in $HC_SCRIPTS; do
    echo "Launching: $script"
    docker exec -d "$CONTAINER_NAME" bash -c "$script"
done

echo ""
echo "All headless clients relaunched."
