#!/usr/bin/env bash
# Fails the build if any file in the git tree matches a known Gold Box
# game-data file name pattern (PLAN.md D10: no game data in the repo, ever).
#
# This checks tracked file *names* only, deliberately not file content —
# words like "ECL"/"DAX"/"GEO" appear constantly in this repo's own prose,
# code, and identifiers as the names of formats we implement, which would
# make a content grep for those words fail on entirely legitimate work.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Case-insensitive basename patterns for known Gold Box / DOS-era game files.
# Mirrors the game-data section of .gitignore.
patterns=(
    '\.dax$'
    '\.tlb$'
    '\.geo$'
    '^ecl[0-9]'
    '\.ecl$'
    '^savgam.*'
    '\.sav$'
    '^start\.exe$'
    '^game\.ovr$'
    '\.ovr$'
    '\.pic$'
    '\.cbm$'
    '\.fnt$'
    '\.pal$'
)

regex=$(IFS='|'; echo "${patterns[*]}")

matches=""
while IFS= read -r path; do
    base=$(basename "$path")
    if echo "$base" | grep -Eiq "$regex"; then
        matches+="$path"$'\n'
    fi
done < <(git ls-files)

if [[ -n "$matches" ]]; then
    echo "no-game-data-guard: found tracked file(s) matching known game-data name patterns:" >&2
    echo "$matches" >&2
    echo "Game data must never be committed to this repository (PLAN.md D10)." >&2
    exit 1
fi

echo "no-game-data-guard: clean — no tracked files match known game-data patterns."
