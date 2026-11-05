#!/bin/bash
# The budgets, checked by the build. A piece that breaks one is not merged.
#   no source file over 800 lines          (Rust and Python; functions over 80 lines are clippy's too_many_lines)
#   Rust under 45 000 lines in all, Python under 5 000, the binding crate under 1 500
#   at most 40 parameters, each read by a test    (the rows of PARAMETERS.md; each name found under a tests folder)
#   a crate depends only on the crates before it  (the order below)
#   the handbook at most 60 lines, the status file at most 40   (only when LIMITS_WORKSPACE names the folder that
#                                                                holds them as CLAUDE.md and STATUS.md; CI skips it)
# Run from anywhere inside the repository. Exit 1 on the first list of breaches, 0 when every budget is met.
set -o pipefail
cd "$(git rev-parse --show-toplevel)" || exit 2
fail=0
breach() { echo "limits: $*" >&2; fail=1; }
lines() { [ "$#" -gt 0 ] && cat "$@" | wc -l | tr -d ' ' || echo 0; }

rs=()
while IFS= read -r f; do rs+=("$f"); done < <(git ls-files -co --exclude-standard '*.rs' | sort -u)
py=()
while IFS= read -r f; do py+=("$f"); done < <(git ls-files -co --exclude-standard '*.py' | sort -u)
bind=()
while IFS= read -r f; do bind+=("$f"); done < <(git ls-files -co --exclude-standard 'crates/binding/*.rs' | sort -u)

for f in "${rs[@]}" "${py[@]}"; do
  n=$(lines "$f")
  [ "$n" -gt 800 ] && breach "$f has $n lines (at most 800)"
done
total_rs=$(lines "${rs[@]}"); [ "$total_rs" -gt 45000 ] && breach "Rust has $total_rs lines (at most 45 000)"
total_py=$(lines "${py[@]}"); [ "$total_py" -gt 5000 ] && breach "Python has $total_py lines (at most 5 000)"
total_bind=$(lines "${bind[@]}"); [ "$total_bind" -gt 1500 ] && breach "the binding has $total_bind lines (at most 1 500)"

# Parameters: the rows of the table in PARAMETERS.md after its header and rule lines.
params=()
while IFS= read -r p; do params+=("$p"); done < <(grep -E '^\| *[A-Za-z_][A-Za-z0-9_]* *\|' PARAMETERS.md 2>/dev/null \
  | grep -vE '^\| *Parameter *\|' | sed -E 's/^\| *([A-Za-z0-9_]+) *\|.*/\1/')
[ "${#params[@]}" -gt 40 ] && breach "${#params[@]} parameters (at most 40)"
tests=()
while IFS= read -r f; do tests+=("$f"); done < <(git ls-files -co --exclude-standard '*/tests/*' 'tests/*' | sort -u)
for p in "${params[@]}"; do
  if [ "${#tests[@]}" -eq 0 ] || ! grep -qF -- "$p" "${tests[@]}"; then breach "parameter $p is read by no test"; fi
done

# The crate order: a crate's internal dependencies all come before it.
order=(types graph loading search choice transit demand loop io-osm io-gtfs io-parquet binding)
rank() { local i; for i in "${!order[@]}"; do [ "${order[$i]}" = "$1" ] && { echo "$i"; return; }; done; echo -1; }
for c in crates/*/; do
  c=${c%/}; name=${c#crates/}; r=$(rank "$name")
  [ "$r" -lt 0 ] && breach "crate $name is not in the crate order"
  while IFS= read -r dep; do
    d=$(rank "$dep")
    [ "$d" -lt 0 ] && breach "crate $name depends on $dep, which is not in the crate order"
    [ "$d" -ge "$r" ] && breach "crate $name depends on $dep, which comes after it"
  done < <(grep -oE '^openmobisim-[a-z-]+' "$c/Cargo.toml" | sed 's/^openmobisim-//' | sort -u)
done

# The handbook and the status file, outside this tree.
if [ -n "${LIMITS_WORKSPACE:-}" ]; then
  h=$(lines "$LIMITS_WORKSPACE/CLAUDE.md"); [ "$h" -gt 60 ] && breach "the handbook has $h lines (at most 60)"
  s=$(lines "$LIMITS_WORKSPACE/STATUS.md"); [ "$s" -gt 40 ] && breach "the status file has $s lines (at most 40)"
fi

echo "limits: Rust $total_rs, Python $total_py, binding $total_bind lines; ${#params[@]} parameters; crate order kept"
exit "$fail"
