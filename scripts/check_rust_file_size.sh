#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

ALLOWLIST_FILE="scripts/source_size_allowlist.txt"

# Helper to check if a relative path is in allowlist
is_allowlisted() {
    local target="$1"
    if [[ -f "${ALLOWLIST_FILE}" ]]; then
        grep -v '^[[:space:]]*#' "${ALLOWLIST_FILE}" | grep -v '^[[:space:]]*$' | grep -Fxq "${target}"
        return $?
    fi
    return 1
}

SEARCH_DIRS=()
for d in src crates tests examples benches; do
    if [[ -d "${d}" ]]; then
        SEARCH_DIRS+=("${d}")
    fi
done

if [[ ${#SEARCH_DIRS[@]} -eq 0 ]]; then
    echo "No source directories found."
    exit 0
fi

echo "================================================================================"
echo "               RUST SOURCE FILE SIZE AUDIT (LINE COUNT BUDGET)                  "
echo "================================================================================"
printf "%-12s | %-10s | %s\n" "LINE COUNT" "STATUS" "FILE"
echo "-------------+------------+-----------------------------------------------------"

FAILURES=0
WARNINGS=0
TOTAL_FILES=0

TMP_FILE="$(mktemp /tmp/rust_file_size.XXXXXX)"
find "${SEARCH_DIRS[@]}" -type f -name "*.rs" ! -path "*/target/*" 2>/dev/null | xargs wc -l | awk '$2 != "total" {print $1, $2}' | sort -nr > "${TMP_FILE}"

while read -r count filepath; do
    TOTAL_FILES=$((TOTAL_FILES + 1))
    status="OK"
    
    # Normalize filepath to relative path
    rel_path="${filepath#./}"
    
    if [ "${count}" -gt 400 ]; then
        if is_allowlisted "${rel_path}"; then
            status="ALLOWED"
        else
            status="FAIL (>400)"
            FAILURES=$((FAILURES + 1))
        fi
    elif [ "${count}" -gt 350 ]; then
        status="WARN (>350)"
        WARNINGS=$((WARNINGS + 1))
    fi

    printf "%12d | %-10s | %s\n" "${count}" "${status}" "${rel_path}"
done < "${TMP_FILE}"

rm -f "${TMP_FILE}"

echo "================================================================================"
echo "Audit Summary: ${TOTAL_FILES} files scanned | ${WARNINGS} warnings (351-400) | ${FAILURES} failures (>400)"
echo "================================================================================"

if [ "${FAILURES}" -gt 0 ]; then
    echo "ERROR: ${FAILURES} file(s) exceeded the 400-line hard limit and are not on the allowlist."
    exit 1
else
    echo "SUCCESS: All files satisfy the line-budget requirements."
    exit 0
fi
