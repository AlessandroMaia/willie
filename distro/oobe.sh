#!/bin/sh
# Runs once, on the first interactive shell. Willie's engine never opens
# one, so this only has to be harmless and fast.
set -eu
id -u willie >/dev/null 2>&1 || {
    echo "oobe: user willie is missing (image bug)" >&2
    exit 1
}
install -d -o willie -g willie -m 0750 /run/willie
exit 0
