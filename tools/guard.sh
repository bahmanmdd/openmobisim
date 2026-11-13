#!/bin/bash
# The leak guard: nothing private enters this repository, whose history is public. It looks for private paths and
# names in the working tree, in what is staged, in a commit message and in the commits being pushed. The paths it
# refuses are built in. The names it refuses are read from a file kept OUTSIDE this tree (so that the tree does not
# itself name what it guards against): one extended regular expression per line, blank lines and `#` lines ignored,
# named by the environment variable GUARD_NAMES. Without that file only the built-in rules run (that is how CI runs).
#   guard.sh tree    REPO               the working tree: tracked and untracked files, their paths and contents
#   guard.sh staged  REPO               what is staged (git's pre-commit hook)
#   guard.sh message FILE               a commit message (git's commit-msg hook)
#   guard.sh push    REPO               each commit being pushed, read from git's pre-push input
#   guard.sh install NAMES [WORKSPACE]  write the three hooks into this clone's .git/hooks, with GUARD_NAMES set to
#                                       NAMES and, when WORKSPACE is given, LIMITS_WORKSPACE for check_limits.sh
# Never bypassed (no `--no-verify`); a false alarm is fixed here. It fails closed: a search that errors stops it
# like a finding. Extended regular expressions only (macOS's grep has no -P).
set -uo pipefail

# Built in: private paths (an agent's instruction files, an editor's folder, a private documents folder, a
# numbered private document) and the names that go with them, and a citation of a numbered private decision (the
# word, in any case, then a number of three digits).
PATHS='^docs/|(^|/)\.claude/|(^|/)\.idea/|(^|/)CLAUDE\.md$|(^|/)[0-9]{2}_[A-Z][A-Z_]+\.md$'
NAMES='[0-9]{2}_[A-Z][A-Z_]+\.md'
NAMES="$NAMES|(^|[^[:alnum:]])[Dd][Ee][Cc][Ii][Ss][Ii][Oo][Nn][Ss]?[[:space:]_/-]*[0-9]{3}([^0-9]|$)"
if [ -n "${GUARD_NAMES:-}" ]; then
  if [ ! -r "$GUARD_NAMES" ]; then
    echo "guard: GUARD_NAMES=$GUARD_NAMES is not readable" >&2
    exit 2
  fi
  extra=$(grep -vE '^[[:space:]]*(#|$)' "$GUARD_NAMES" | paste -sd '|' -)
  [ -n "$extra" ] && NAMES="$NAMES|$extra"
fi
ZERO=0000000000000000000000000000000000000000

fail=0
# check WHAT SEARCH...: run SEARCH, which prints what it finds and exits 0 when it found something, 1 when it
# found nothing and more on an error; a finding or an error stops the guard.
check() {
  local what=$1 out rc
  shift
  out=$("$@" 2>&1)
  rc=$?
  if [ "$rc" -eq 0 ] && [ -n "$out" ]; then
    echo "guard: $what" >&2
    echo "$out" | cut -c1-200 | head -20 >&2
    fail=1
  elif [ "$rc" -gt 1 ]; then
    echo "guard: $what: the search failed (exit $rc)" >&2
    echo "$out" | head -5 >&2
    fail=1
  fi
}
on_disk() { local p hit=1; for p in docs CLAUDE.md .claude; do [ -e "$p" ] && { echo "$p"; hit=0; }; done; return $hit; }
tree_paths() { { git ls-files && git ls-files --others --exclude-standard; } | grep -E "$PATHS"; }
tree_names() { git grep -nIE --untracked "$NAMES" -- . ':!tools/guard.sh'; }
staged_paths() { git diff --cached --name-only --diff-filter=ACMR | grep -E "$PATHS"; }
staged_names() { git grep --cached -nIE "$NAMES" -- . ':!tools/guard.sh'; }
message_names() { grep -v '^#' "$1" | grep -nE "$NAMES"; }
commit_paths() { git ls-tree -r --name-only "$1" | grep -E "$PATHS"; }
commit_names() { git grep -nIE "$NAMES" "$1" -- . ':!tools/guard.sh'; }
log_names() { git log --format='%h %B' "$@" | grep -E "$NAMES"; }
added_names() { git log -p --format='commit %h' "$@" -- . ':!tools/guard.sh' | grep -E '^\+' | grep -E "$NAMES"; }

install_hooks() {
  local names workspace hooks here
  names=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
  [ -r "$names" ] || { echo "guard: names file $names is not readable" >&2; exit 2; }
  workspace=${2:-}
  [ -n "$workspace" ] && workspace=$(cd "$workspace" && pwd)
  here=$(cd "$(dirname "$0")" && pwd)
  hooks=$(cd "$here/.." && cd "$(git rev-parse --git-common-dir)" && pwd)/hooks
  mkdir -p "$hooks"
  write() {  # write NAME BODY
    local file="$hooks/$1"
    if [ -e "$file" ] && ! grep -q 'the leak guard' "$file"; then
      echo "guard: not overwritten, not ours: $file" >&2
      exit 1
    fi
    printf '#!/bin/sh\n# the leak guard, installed by tools/guard.sh\nexport GUARD_NAMES=%q\nexport LIMITS_WORKSPACE=%q\n%s\n' \
      "$names" "$workspace" "$2" > "$file"
    chmod +x "$file"
    echo "installed $file"
  }
  write pre-commit "top=\$(git rev-parse --show-toplevel); bash \"\$top/tools/guard.sh\" staged \"\$top\" && bash \"\$top/tools/check_limits.sh\""
  write commit-msg "exec bash \"\$(git rev-parse --show-toplevel)/tools/guard.sh\" message \"\$1\""
  write pre-push "exec bash \"\$(git rev-parse --show-toplevel)/tools/guard.sh\" push \"\$(git rev-parse --show-toplevel)\""
}

case "${1:-}" in
  tree)
    cd "${2:?usage: guard.sh tree REPO}" || exit 2
    check "a private path on disk" on_disk
    check "private paths in the tree" tree_paths
    check "private names in the files" tree_names
    ;;
  staged)
    cd "${2:?usage: guard.sh staged REPO}" || exit 2
    check "private paths staged" staged_paths
    check "private names in the staged files" staged_names
    ;;
  message)
    check "private names in the commit message" message_names "${2:?usage: guard.sh message FILE}"
    ;;
  push)
    cd "${2:?usage: guard.sh push REPO}" || exit 2
    while read -r _lref lsha _rref rsha; do
      [ "$lsha" = "$ZERO" ] && continue  # a branch deleted: nothing is sent
      check "private paths in ${lsha:0:7}" commit_paths "$lsha"
      check "private names in ${lsha:0:7}'s files" commit_names "$lsha"
      if [ "$rsha" = "$ZERO" ]; then range=("$lsha" --not --remotes); else range=("$rsha..$lsha"); fi
      check "private names in the messages pushed" log_names "${range[@]}"
      check "private names added by a commit pushed (even if removed later)" added_names "${range[@]}"
    done
    ;;
  install)
    install_hooks "${2:?usage: guard.sh install NAMES [WORKSPACE]}" "${3:-}"
    exit 0
    ;;
  *)
    echo "usage: guard.sh tree|staged|push REPO, guard.sh message FILE, or guard.sh install NAMES [WORKSPACE]" >&2
    exit 2
    ;;
esac
[ "$fail" = 0 ] || echo "guard: stopped: remove the private text; a false alarm is fixed in tools/guard.sh" >&2
exit "$fail"
