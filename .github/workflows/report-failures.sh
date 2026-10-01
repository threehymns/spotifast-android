#!/usr/bin/env bash
# Post CI failure excerpts (or reporter diagnostics) as a commit comment.
#
# Usage: report-failures.sh <title> <markdown-out> <log> [<log> ...]
#
# Runs report-failures.py over the step logs; when no excerpt comes out
# (no interpreter, no logs, a crash), posts diagnostics instead so the
# next reader learns why instead of guessing. Exits 0 unless the final
# `gh api` post fails. Needs GH_TOKEN, REPORT_OS, REPORT_REPO and
# REPORT_SHA in the environment.
set -u
title=$1
out=$2
shift 2
export REPORT_TITLE="$title"
pybin="$(command -v python3 || command -v python || command -v py || true)"
if [ -n "$pybin" ]; then
  "$pybin" .github/workflows/report-failures.py "$@" > "$out" || true
fi
if ! test -s "$out"; then
  {
    echo "The reporter could not extract failure sections; diagnostics:"
    echo "python3: $(command -v python3 || echo MISSING)"
    echo "python: $(command -v python || echo MISSING)"
    echo "py: $(command -v py || echo MISSING)"
    echo "logs:"
    for log in "$@"; do
      if [ -f "$log" ]; then
        ls -la -- "$log" || echo "(unreadable) $log"
      else
        echo "(missing) $log"
      fi
    done
  } > "$out"
fi
gh api "repos/$REPORT_REPO/commits/$REPORT_SHA/comments" -F body=@"$out"
