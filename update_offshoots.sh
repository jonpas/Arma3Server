#!/bin/bash

# Update all offshoots with changes from the template while preserving
# their specific configuration (.env, main.cfg hostname, compose project name).

TEMPLATE="/opt/arma3/template"
ARMA_ROOT="/opt/arma3"

# Find all offshoot directories (skua_* but not the template itself)
OFFSHOOTS=()
for dir in "$ARMA_ROOT"/skua_*/; do
    [ -d "$dir" ] && OFFSHOOTS+=("$dir")
done

if [ ${#OFFSHOOTS[@]} -eq 0 ]; then
    echo "No offshoots found in $ARMA_ROOT"
    exit 0
fi

echo "Found ${#OFFSHOOTS[@]} offshoot(s):"
for offshoot in "${OFFSHOOTS[@]}"; do
    echo "  - $(basename "$offshoot")"
done
echo ""

for offshoot in "${OFFSHOOTS[@]}"; do
    NAME=$(basename "$offshoot")
    echo "=== Updating $NAME ==="

    #
    # Update docker-compose.yml — preserve the project name
    #
    COMPOSE_FILE="$offshoot/docker-compose.yml"
    if [ -f "$COMPOSE_FILE" ]; then
        # Extract existing project name
        EXISTING_NAME=$(grep -E '^name:' "$COMPOSE_FILE" | head -1)

        # Copy template compose file
        cp "$TEMPLATE/docker-compose.yml" "$COMPOSE_FILE"

        # Restore the project name
        if [ -n "$EXISTING_NAME" ]; then
            sed -i "s|^name:.*|$EXISTING_NAME|" "$COMPOSE_FILE"
        fi
        echo "  Updated docker-compose.yml (preserved project name)"
    fi

    #
    # Update configs/basic.cfg — no offshoot-specific values
    #
    if [ -f "$TEMPLATE/configs/basic.cfg" ]; then
        cp "$TEMPLATE/configs/basic.cfg" "$offshoot/configs/basic.cfg"
        echo "  Updated configs/basic.cfg"
    fi

    #
    # Update relaunch_hc.sh
    #
    if [ -f "$TEMPLATE/relaunch_hc.sh" ]; then
        cp "$TEMPLATE/relaunch_hc.sh" "$offshoot/relaunch_hc.sh"
        chmod +x "$offshoot/relaunch_hc.sh"
        echo "  Updated relaunch_hc.sh"
    fi

    #
    # Preserved (not touched):
    #   - .env (PORT, server-specific env vars)
    #   - configs/main.cfg (hostname, passwords, admins)
    #   - missions/, mods/, servermods/, mod_presets/
    #

    echo "  Done."
    echo ""
done

echo "All offshoots updated."
echo "Preserved per-offshoot: .env, configs/main.cfg, missions, mods, servermods, mod_presets"
