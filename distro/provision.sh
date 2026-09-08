#!/bin/sh
# Turns a bare Debian slim root filesystem into the Willie system image.
# Everything the user works with (agent CLIs, toolchains) is NOT here.
set -eu
IMAGE_VERSION="$1"; CONF_DIR="$2"; BIN_DIR="$3"
export DEBIAN_FRONTEND=noninteractive

apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates curl git bubblewrap sudo procps iproute2 less zsh
apt-get clean
rm -rf /var/lib/apt/lists/*

# The image ships only the C.UTF-8 locale; make it the default so tools
# that emit UTF-8 (the harness, git) do not fall back to ASCII.
printf 'LANG=C.UTF-8\n' > /etc/default/locale

if ! id -u willie >/dev/null 2>&1; then
    useradd --uid 1000 --user-group --create-home --shell /bin/bash willie
fi
echo 'willie ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/willie
chmod 0440 /etc/sudoers.d/willie

install -d -m 0755 /opt/willie/bin /opt/willie/libexec /etc/willie
install -d -o willie -g willie -m 0750 /var/lib/willie
install -d -o willie -g willie -m 0700 /home/willie/projects
install -m 0644 "$CONF_DIR/wsl.conf" /etc/wsl.conf
install -m 0644 "$CONF_DIR/wsl-distribution.conf" /etc/wsl-distribution.conf
install -m 0755 "$CONF_DIR/oobe.sh" /opt/willie/libexec/oobe.sh
install -d -m 0755 /etc/willie/zsh
install -m 0644 "$CONF_DIR/zsh/.zshrc" /etc/willie/zsh/.zshrc
install -m 0755 "$BIN_DIR/willied" "$BIN_DIR/willie-sess" "$BIN_DIR/willie" \
    /opt/willie/bin/
printf '%s\n' "$IMAGE_VERSION" > /etc/willie/image-version

# Agent state lives outside the ephemeral sandbox home; the symlinks make a
# plain shell in the distribution use the same login as sessions do.
STATE=/home/willie/.willie/agent-state/claude
install -d -o willie -g willie -m 0700 \
    /home/willie/.willie \
    /home/willie/.willie/agent-state \
    "$STATE" \
    "$STATE/dot-claude"
if [ ! -f "$STATE/claude.json" ]; then
    printf '{}\n' > "$STATE/claude.json"
    chown willie:willie "$STATE/claude.json"
fi
chmod 0600 "$STATE/claude.json"
ln -sfn .willie/agent-state/claude/dot-claude /home/willie/.claude
ln -sfn .willie/agent-state/claude/claude.json /home/willie/.claude.json
chown -h willie:willie /home/willie/.claude /home/willie/.claude.json

echo "provisioned willie image $IMAGE_VERSION"
