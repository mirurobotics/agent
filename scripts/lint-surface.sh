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
# Exclude only the .agents subtree (content owned by another repo). Every other
# .sh file — including the shebang-less jinja partials under
# scripts/jinja/templates/ and the generated install scripts under
# scripts/install/ — is linted here, mirroring the shared surface-lint workflow.
# Findings in the generated scripts must still be fixed in the jinja templates,
# not the rendered output.
find . -name '*.sh' \
    ! -path './.agents/*' \
    -exec shellcheck {} +
echo ""

echo "actionlint"
echo "----------"
actionlint
echo ""

echo "Surface lint complete"
