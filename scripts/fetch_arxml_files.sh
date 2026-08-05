#!/usr/bin/env bash
#
# Fetches the ARXML fixtures `tests/arxml_database.rs` checks against.
#
# They come from cantools' test corpus. cantools is MIT-licensed and is also the
# independent reader those tests compare against — its own assertions about these
# files are what `tests/arxml_database.rs` transcribes, so taking the files from
# the same place keeps the two in step.
#
# The files land in test_data/, which is gitignored: nothing here is
# redistributed, and the tests skip when they are absent. Same arrangement as
# `fetch_reference_files.sh`.

set -euo pipefail

BASE="https://raw.githubusercontent.com/cantools/cantools/master/tests/files/arxml"
DEST="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/test_data/arxml"

mkdir -p "$DEST"

for file in system-4.2.arxml ecu-extract-4.2.arxml; do
    if [ -f "$DEST/$file" ]; then
        echo "have $file"
        continue
    fi
    echo "fetching $file"
    curl -sSLf -o "$DEST/$file" "$BASE/$file"
done

echo "ARXML fixtures are in $DEST"
