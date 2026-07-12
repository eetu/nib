#!/usr/bin/env bash
# Point git at the repo's tracked hooks (.githooks/). Run once after cloning.
#
# COMMIT THIS FILE WITH MODE 755. An editor/tool writes it 644, git stores the
# mode, and a 644 script can't be `./`-run. After creating:
#   chmod +x install-hooks.sh .githooks/* && git add install-hooks.sh .githooks
# (or, if already committed wrong: git update-index --chmod=+x <file>).
set -e
git config core.hooksPath .githooks
echo "Installed: core.hooksPath -> .githooks"
