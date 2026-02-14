#!/bin/sh
set -e

git_repo_root_dir=$(git rev-parse --show-toplevel)
echo "git_repo_root_dir: $git_repo_root_dir"
cd "$git_repo_root_dir"

src_lines=$(git ls-files | grep '\.rs$' | grep -v '/tests/' | xargs cat | wc -l)
echo "Source code: $src_lines lines"

test_lines=$(git ls-files | grep '\.rs$' | grep '/tests/' | xargs cat | wc -l)
echo "Test code: $test_lines lines"

total_lines=$(git ls-files | grep '\.rs$' | xargs cat | wc -l)
echo "Total: $total_lines lines"
