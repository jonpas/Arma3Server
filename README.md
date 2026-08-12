# Template

Template directory for Arma 3 to allow for sharing common files (workshop mods, scripts, the Dockerfile, Arma 3 Server) across multiple server instances (Persistence/Liberation, R&D/Training, and Prod/Ops).

## Servers and Ports
- Production: The mainline Operations server, at ports 2302-2304.
- R&D: The Training & Development testing server, at ports 2402-2404.
- Persistence: Used by Liberation and other such gamemodes, at ports 2502-2504.

## Instructions

Run this:
```bash
OFFSHOOT_NAME="<your-offshoot-name>" /opt/arma3/template/make_new_offshoot.sh
```

Then edit:
- main.cfg if you want to set a password
- .env for the HTML name

# Original
Based off https://github.com/BrettMayson/Arma3Server
