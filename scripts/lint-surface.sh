#!/bin/sh
set -e

command -v yamllint >/dev/null 2>&1 || { echo "error: yamllint not installed" >&2; exit 1; }
command -v shellcheck >/dev/null 2>&1 || { echo "error: shellcheck not installed" >&2; exit 1; }
command -v actionlint >/dev/null 2>&1 || { echo "error: actionlint not installed" >&2; exit 1; }

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

echo "yamllint"
echo "--------"
yamllint -c .yamllint.yml .
echo ""

echo "shellcheck"
echo "----------"
# Exclude:
#   - the .agents subtree (content owned by another repo);
#   - the jinja template fragments under scripts/jinja/templates/ (shebang-less
#     .sh partials concatenated by scripts/jinja/render.sh; shellcheck flags
#     them SC2148 on their own);
#   - the generated install scripts under scripts/install/ (rendered from the
#     jinja templates by scripts/jinja/render.py — they are build artifacts, so
#     findings must be fixed in the templates, not the generated output).
find . -name '*.sh' \
    ! -path './.agents/*' \
    ! -path './scripts/jinja/templates/*' \
    ! -path './scripts/install/*' \
    -exec shellcheck {} +
echo ""

echo "actionlint"
echo "----------"
actionlint
echo ""

echo "Surface lint complete"
