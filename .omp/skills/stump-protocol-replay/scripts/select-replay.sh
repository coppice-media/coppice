#!/bin/sh
# Print the narrowest existing sibling-harness target for a protocol claim.
# This is routing only: it never launches a service or reads credentials.
set -eu

if [ "$#" -ne 1 ]; then
  printf '%s\n' "usage: select-replay.sh komga|mihon|kavita|abs|liseur|library|kobo|koreader" >&2
  exit 2
fi

case "$1" in
  komga|komelia) printf '%s\n' 'make replay' ;;
  mihon)         printf '%s\n' 'make replay-mihon' ;;
  kavita)        printf '%s\n' 'make replay-kavita' ;;
  abs|audiobookshelf|lissen) printf '%s\n' 'make replay-abs' 'comparison: make replay-abs-diff' ;;
  liseur-sync)   printf '%s\n' 'make replay-liseur-sync' ;;
  liseur)        printf '%s\n' 'native: make replay-liseur-sync' 'Komga client: make replay' 'OPDS/KOSync: targeted probe or device run' ;;
  library|library-management) printf '%s\n' 'make replay-library-management' ;;
  containers|collections|readlists) printf '%s\n' 'make replay-containers (only when project state marks it ready)' ;;
  kobo)          printf '%s\n' 'targeted Kobo fixture probe or physical-client run (no sibling Make target)' ;;
  koreader|kosync) printf '%s\n' 'targeted KOReader/KOSync fixture probe or physical-client run (no sibling Make target)' ;;
  *)
    printf 'unknown protocol claim: %s\n' "$1" >&2
    exit 1
    ;;
esac
