#!/bin/bash
# Auto-detect the running cocoa-way instance's runtime directory
export XDG_RUNTIME_DIR=$(find /var/folders /tmp -type d -name "cocoa-way" 2>/dev/null | head -n 1)
export WAYLAND_DISPLAY=wayland-1

if [ -z "$XDG_RUNTIME_DIR" ]; then
    echo "Error: Could not find cocoa-way runtime directory. Please ensure cocoa-way is running."
    exit 1
fi

echo "Found cocoa-way at: $XDG_RUNTIME_DIR"
echo "Running waypipe command..."

# If the command is 'ssh', automatically add the recommended StreamLocalBindUnlink=yes option
# This fixes "remote port forwarding failed" errors when the socket file already exists on the remote
if [ "$1" = "ssh" ]; then
    echo "Info: Detected SSH mode. Injecting '-o StreamLocalBindUnlink=yes' to fix socket conflicts."
    # Insert the option after 'ssh'
    shift
    exec waypipe --compress=zstd ssh -o StreamLocalBindUnlink=yes "$@"
else
    # Execute waypipe with the provided arguments
    exec waypipe "$@"
fi
