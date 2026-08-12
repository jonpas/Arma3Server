#!/bin/bash

cd /opt/arma3

# Set offshoot name to the current unix timestamp if no name was set.
OFFSHOOT_NAME=${OFFSHOOT_NAME:-$(date +%s)}

NEW_DIRECTORY="/opt/arma3/skua_${OFFSHOOT_NAME}"

echo "Creating new offshoot @ ${NEW_DIRECTORY}..."
echo "Note: you can run this script with OFFSHOOT_NAME='name here' ./make_new_offshoot.sh to set a custom name"

TEMPLATE="/opt/arma3/template"

echo "Making directory structure..."

# Make directory structure
mkdir -p \
  "${NEW_DIRECTORY}/missions" \
  "${NEW_DIRECTORY}/mod_presets" \
  "${NEW_DIRECTORY}/configs" \
  "${NEW_DIRECTORY}/mods" \
  "${NEW_DIRECTORY}/servermods"

echo "Copying template files..."

# Copy over the files
cp "${TEMPLATE}/docker-compose.yml" "${NEW_DIRECTORY}"
cp "${TEMPLATE}/.env.example" "${NEW_DIRECTORY}/.env"
cp "${TEMPLATE}/configs/basic.cfg" "${NEW_DIRECTORY}/configs"
cp "${TEMPLATE}/configs/main.cfg" "${NEW_DIRECTORY}/configs"

#
# Prompt user for values
#
read -rp "Enter PORT to use for this offshoot: " PORT
read -rp "Enter server name (replaces [Template]): " SERVER_NAME

#
# Set docker-compose project name
#
COMPOSE_FILE="${NEW_DIRECTORY}/docker-compose.yml"

if grep -qE '^[[:space:]]*name:' "$COMPOSE_FILE"; then
    sed -i "s|^[[:space:]]*name:.*|name: skua_${OFFSHOOT_NAME}|" "$COMPOSE_FILE"
else
    sed -i "1iname: skua_${OFFSHOOT_NAME}\n" "$COMPOSE_FILE"
fi

#
# Set PORT in .env
#
ENV_FILE="${NEW_DIRECTORY}/.env"

if grep -qE '^[[:space:]]*PORT=' "$ENV_FILE"; then
    sed -i "s|^[[:space:]]*PORT=.*|PORT=${PORT}|" "$ENV_FILE"
else
    echo "PORT=${PORT}" >> "$ENV_FILE"
fi

#
# Replace [Template] in main.cfg hostname
#
MAIN_CFG="${NEW_DIRECTORY}/configs/main.cfg"

if grep -qE '^[[:space:]]*hostname[[:space:]]*=' "$MAIN_CFG"; then
    perl -0777 -i -pe \
      's/^[[:space:]]*hostname[[:space:]]*=.*$/hostname = "Skua Intl. '"$SERVER_NAME"' | Info: https:\/\/skua.international";/m' \
      "$MAIN_CFG"
else
    echo "hostname = \"Skua Intl. ${SERVER_NAME} | Info: https://skua.international\";" >> "$MAIN_CFG"
fi

echo "All done!"
echo "Configured:"
echo "  - Project name: skua_${OFFSHOOT_NAME}"
echo "  - PORT: ${PORT}"
echo "  - Server name: ${SERVER_NAME}"
echo
echo "Make sure you review main.cfg, .env, and docker-compose.yml before launching."
